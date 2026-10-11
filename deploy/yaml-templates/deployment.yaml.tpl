apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{name}}
  namespace: {{namespace}}
spec:
  replicas: {{replicas}}
  selector:
    matchLabels:
      app: {{name}}
  template:
    metadata:
      labels:
        app: {{name}}
    spec:
{{initContainersBlock}}
      containers:
        - name: {{name}}
          image: {{image}}
          ports:
            - containerPort: {{containerPort}}
          resources:
            requests:
              cpu: {{requestsCpu}}
              memory: {{requestsMemory}}
            limits:
              cpu: {{limitsCpu}}
              memory: {{limitsMemory}}
{{volumeMountsBlock}}
{{volumesBlock}}
