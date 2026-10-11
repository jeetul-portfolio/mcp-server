apiVersion: v1
kind: Service
metadata:
  name: {{name}}
  namespace: {{namespace}}
spec:
  selector:
    app: {{name}}
  type: {{serviceType}}
  ports:
    - protocol: TCP
      port: {{servicePort}}
      targetPort: {{serviceTargetPort}}
