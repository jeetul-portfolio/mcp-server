const fs = require('fs');
const path = require('path');

const configPath = path.join(__dirname, 'services.json');
const pvcConfigPath = path.join(__dirname, 'pvc.json');
const templatesDir = path.join(__dirname, 'yaml-templates');
const outputDir = path.join(__dirname, 'ymls');
const deploymentsOutputDir = path.join(outputDir, 'deployments');
const networkingOutputDir = path.join(outputDir, 'networking');
const storageOutputDir = path.join(outputDir, 'storage');

function loadConfig(filePath) {
  try {
    const fileContent = fs.readFileSync(filePath, 'utf-8');
    return JSON.parse(fileContent);
  } catch (error) {
    throw new Error(`Unable to read or parse config file at ${filePath}: ${error.message}`);
  }
}

function loadTemplate(templateName) {
  const templatePath = path.join(templatesDir, templateName);

  try {
    return fs.readFileSync(templatePath, 'utf-8');
  } catch (error) {
    throw new Error(`Unable to read template file at ${templatePath}: ${error.message}`);
  }
}

function applyTemplate(template, values) {
  return Object.keys(values).reduce((content, key) => {
    return content.replaceAll(`{{${key}}}`, String(values[key]));
  }, template);
}

function buildInitContainersBlock(commands, image) {
  if (!Array.isArray(commands) || commands.length === 0) {
    return '';
  }

  const lines = [
    '      initContainers:',
    '        - name: pre-start-commands',
    `          image: ${image || 'busybox:1.36'}`,
    '          command:',
    '            - /bin/sh',
    '            - -c',
    '            - |',
  ];

  commands.forEach((command) => {
    lines.push(`              ${command}`);
  });

  return lines.join('\n');
}

function buildVolumeMountBlocks(configMounts, pvcMounts) {
  const hasConfigMounts = Array.isArray(configMounts) && configMounts.length > 0;
  const hasPvcMounts = Array.isArray(pvcMounts) && pvcMounts.length > 0;

  if (!hasConfigMounts && !hasPvcMounts) {
    return {
      volumeMountsBlock: '',
      volumesBlock: '',
    };
  }

  const mountLines = ['          volumeMounts:'];
  const volumeLines = ['      volumes:'];

  if (hasConfigMounts) {
    configMounts.forEach((mount, index) => {
      if (!mount.configMapName || !mount.mountPath) {
        throw new Error(
          `Each configMount must include both "configMapName" and "mountPath". Invalid entry at index ${index}.`
        );
      }

      const volumeName = mount.volumeName || `config-mount-${index + 1}`;

      mountLines.push(`            - name: ${volumeName}`);
      mountLines.push(`              mountPath: ${mount.mountPath}`);

      if (mount.subPath) {
        mountLines.push(`              subPath: ${mount.subPath}`);
      }

      if (typeof mount.readOnly === 'boolean') {
        mountLines.push(`              readOnly: ${mount.readOnly}`);
      }

      volumeLines.push(`        - name: ${volumeName}`);
      volumeLines.push('          configMap:');
      volumeLines.push(`            name: ${mount.configMapName}`);

      if (Array.isArray(mount.items) && mount.items.length > 0) {
        volumeLines.push('            items:');
        mount.items.forEach((item) => {
          if (item.key && item.path) {
            volumeLines.push(`              - key: ${item.key}`);
            volumeLines.push(`                path: ${item.path}`);
          }
        });
      }
    });
  }

  if (hasPvcMounts) {
    pvcMounts.forEach((mount, index) => {
      if (!mount.claimName || !mount.mountPath) {
        throw new Error(
          `Each pvcMount must include both "claimName" and "mountPath". Invalid entry at index ${index}.`
        );
      }

      const volumeName = mount.volumeName || mount.claimName;

      mountLines.push(`            - name: ${volumeName}`);
      mountLines.push(`              mountPath: ${mount.mountPath}`);

      if (mount.subPath) {
        mountLines.push(`              subPath: ${mount.subPath}`);
      }

      if (typeof mount.readOnly === 'boolean') {
        mountLines.push(`              readOnly: ${mount.readOnly}`);
      }

      volumeLines.push(`        - name: ${volumeName}`);
      volumeLines.push('          persistentVolumeClaim:');
      volumeLines.push(`            claimName: ${mount.claimName}`);
    });
  }

  return {
    volumeMountsBlock: mountLines.join('\n'),
    volumesBlock: volumeLines.join('\n'),
  };
}

function buildAccessModesBlock(accessModes) {
  if (!Array.isArray(accessModes) || accessModes.length === 0) {
    throw new Error('PVC must include at least one accessMode.');
  }
  return accessModes.map((mode) => `    - ${mode}`).join('\n');
}

function buildStorageClassNameBlock(storageClassName) {
  if (!storageClassName) return '';
  return `  storageClassName: ${storageClassName}`;
}

function generatePvcFiles(pvcs, pvcTemplate, storageOutputDir) {
  pvcs.forEach((pvc, index) => {
    const missingFields = [];
    if (!pvc.name) missingFields.push('name');
    if (!pvc.namespace) missingFields.push('namespace');
    if (!pvc.storage) missingFields.push('storage');
    if (!Array.isArray(pvc.accessModes) || pvc.accessModes.length === 0) missingFields.push('accessModes');

    if (missingFields.length > 0) {
      throw new Error(
        `PVC at index ${index} is missing required field(s): ${missingFields.join(', ')}.`
      );
    }

    const values = {
      name: pvc.name,
      namespace: pvc.namespace,
      storage: pvc.storage,
      accessModesBlock: buildAccessModesBlock(pvc.accessModes),
      storageClassNameBlock: buildStorageClassNameBlock(pvc.storageClassName),
    };

    const pvcYaml = applyTemplate(pvcTemplate, values);
    const pvcYamlPath = path.join(storageOutputDir, `${pvc.name}.yaml`);
    fs.writeFileSync(pvcYamlPath, pvcYaml, 'utf-8');
    console.log(`Generated: ${pvcYamlPath}`);
  });
}

function resolveDeploymentValues(general, deployment, index) {
  const requestsDefaults = (general.resources && general.resources.requests) || {};
  const limitsDefaults = (general.resources && general.resources.limits) || {};

  const requests = {
    cpu: (deployment.resources && deployment.resources.requests && deployment.resources.requests.cpu) || requestsDefaults.cpu,
    memory:
      (deployment.resources && deployment.resources.requests && deployment.resources.requests.memory) ||
      requestsDefaults.memory,
  };

  const limits = {
    cpu: (deployment.resources && deployment.resources.limits && deployment.resources.limits.cpu) || limitsDefaults.cpu,
    memory:
      (deployment.resources && deployment.resources.limits && deployment.resources.limits.memory) || limitsDefaults.memory,
  };

  const namespace = deployment.namespace || general.namespace || 'default';
  const replicas = deployment.replicas || 1;

  const missingFields = [];
  if (!deployment.name) missingFields.push('name');
  if (!deployment.image) missingFields.push('image');
  if (!deployment.containerPort) missingFields.push('containerPort');

  if (missingFields.length > 0) {
    throw new Error(
      `Deployment at index ${index} is missing required field(s): ${missingFields.join(', ')}.`
    );
  }

  if (!requests.cpu || !requests.memory || !limits.cpu || !limits.memory) {
    throw new Error(
      `Missing CPU or memory values for deployment "${deployment.name}". Set defaults in "general.resources" or per deployment "resources".`
    );
  }

  const service = deployment.service || {};
  const { volumeMountsBlock, volumesBlock } = buildVolumeMountBlocks(deployment.configMounts, deployment.pvcMounts);
  const preStartCommands = deployment.preStartCommands || general.preStartCommands;
  const preStartImage = deployment.preStartImage || general.preStartImage || 'busybox:1.36';
  const initContainersBlock = buildInitContainersBlock(preStartCommands, preStartImage);

  return {
    name: deployment.name,
    namespace,
    replicas,
    image: deployment.image,
    containerPort: deployment.containerPort,
    requestsCpu: requests.cpu,
    requestsMemory: requests.memory,
    limitsCpu: limits.cpu,
    limitsMemory: limits.memory,
    serviceType: service.type || 'ClusterIP',
    servicePort: service.port || deployment.containerPort,
    serviceTargetPort: service.targetPort || deployment.containerPort,
    initContainersBlock,
    volumeMountsBlock,
    volumesBlock,
  };
}

function generateFiles() {
  const config = loadConfig(configPath);
  const general = config.general || {};
  const deployments = config.deployments;

  if (!Array.isArray(deployments) || deployments.length === 0) {
    throw new Error('services.json must contain a non-empty "deployments" array.');
  }

  const deploymentTemplate = loadTemplate('deployment.yaml.tpl');
  const serviceTemplate = loadTemplate('service.yaml.tpl');

  if (!fs.existsSync(outputDir)) {
    fs.mkdirSync(outputDir, { recursive: true });
  }

  if (!fs.existsSync(deploymentsOutputDir)) {
    fs.mkdirSync(deploymentsOutputDir, { recursive: true });
  }

  if (!fs.existsSync(networkingOutputDir)) {
    fs.mkdirSync(networkingOutputDir, { recursive: true });
  }


  if (fs.existsSync(pvcConfigPath)) {
    const pvcConfig = loadConfig(pvcConfigPath);
    const pvcTemplate = loadTemplate('pvc.yaml.tpl');
    if (Array.isArray(pvcConfig.pvcs) && pvcConfig.pvcs.length > 0) {
      generatePvcFiles(pvcConfig.pvcs, pvcTemplate, storageOutputDir);
    }
  }

  deployments.forEach((deployment, index) => {
    const values = resolveDeploymentValues(general, deployment, index);

    const deploymentYamlPath = path.join(deploymentsOutputDir, `${deployment.name}-deployment.yaml`);
    const serviceYamlPath = path.join(networkingOutputDir, `${deployment.name}-service.yaml`);

    const deploymentYaml = applyTemplate(deploymentTemplate, values);
    const serviceYaml = applyTemplate(serviceTemplate, values);

    fs.writeFileSync(deploymentYamlPath, deploymentYaml, 'utf-8');
    fs.writeFileSync(serviceYamlPath, serviceYaml, 'utf-8');

    console.log(`Generated: ${deploymentYamlPath}`);
    console.log(`Generated: ${serviceYamlPath}`);
  });
}

generateFiles();
