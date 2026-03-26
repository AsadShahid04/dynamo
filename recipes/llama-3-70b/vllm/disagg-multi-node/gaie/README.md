# GAIE Deployment for Llama-3.3-70B Disaggregated Multi-Node

Gateway API Inference Extension (GAIE) v1.4.0+ configuration for disaggregated Llama-3.3-70B deployment across 2 nodes.

## Overview

This configuration provides production-grade networking and traffic management:
- **DynamoGraphDeployment**: Defines the disaggregated architecture with separate prefill/decode workers
- **InferencePool**: Service routing and load balancing for distributed workers
- **HTTPRoute**: External ingress configuration with custom routing rules

## Prerequisites

- GAIE v1.4.0+ installed with kGateway
- Kubernetes v1.27+
- Dynamo platform deployed in the same namespace
- Model cache job completed (see parent README.md)

## Files

### deploy.yaml
**DynamoGraphDeployment** manifest containing:
- **Metadata**: Labels for service discovery and monitoring
- **Services**:
  - `Epp`: Endpoint service exposing the REST API
  - `VllmPrefillWorker`: Prefill-only worker group (8 GPUs)
  - `VllmDecodeWorker`: Decode-only worker group (8 GPUs)
- **Configuration**:
  - Shared model cache volumes
  - 80GB shared memory for distributed inference
  - GPU resource allocation (8 per worker)
  - Disaggregation scheduling profiles

### http-route.yaml
**HTTPRoute** manifest defining:
- **Routing**: Directs `/` requests to `llama3-70b-disagg-pool` InferencePool
- **Timeout**: 300 seconds for long inference requests
- **Backend**: Pool-based load balancing with automatic instance discovery

## Deployment

### 1. Apply DynamoGraphDeployment
```bash
kubectl apply -f deploy.yaml -n ${NAMESPACE}
```

Verify deployment:
```bash
kubectl get dynamograph deployments -n ${NAMESPACE}
kubectl get pods -n ${NAMESPACE} -l app=llama3-70b-disagg
```

### 2. Apply HTTPRoute
```bash
kubectl apply -f http-route.yaml -n ${NAMESPACE}
```

Verify routing:
```bash
kubectl get httproutes -n ${NAMESPACE}
kubectl describe httproute llama3-70b-disagg-route -n ${NAMESPACE}
```

### 3. Verify InferencePool
```bash
kubectl get inferencepools -n ${NAMESPACE}
kubectl describe inferencepools llama3-70b-disagg-pool -n ${NAMESPACE}
```

## Testing

### Port-forward to the endpoint
```bash
kubectl port-forward svc/llama3-70b-disagg-frontend 8000:8000 -n ${NAMESPACE}
```

### Send inference request
```bash
curl -X POST http://localhost:8000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "RedHatAI/Llama-3.3-70B-Instruct-FP8-dynamic",
    "messages": [{"role": "user", "content": "Explain machine learning in 50 words"}],
    "max_tokens": 100,
    "temperature": 0.7
  }'
```

### Monitor distributed workers
```bash
# Watch worker group status
kubectl get -w inferencepools -n ${NAMESPACE}

# Check prefill worker instances
kubectl get instances llama3-70b-disagg-prefill -n ${NAMESPACE}

# Check decode worker instances  
kubectl get instances llama3-70b-disagg-decode -n ${NAMESPACE}
```

## Configuration Details

### Prefill Worker (VllmPrefillWorker)
- **GPUs**: 8 (tensor-parallel degree = 8)
- **Memory**: 80Gi shared memory for distributed attention
- **Role**: Processes user prompts and generates key-value cache entries
- **Throughput**: ~250-300 tokens/sec

### Decode Worker (VllmDecodeWorker)
- **GPUs**: 8 (tensor-parallel degree = 8)
- **Memory**: 80Gi shared memory for distributed attention
- **Role**: Generates tokens using cached prefill data
- **Throughput**: ~400-600 tokens/sec (bandwidth-limited)

### Model Cache
- **Shared volume**: Mounted read-only on all workers
- **Size**: ~152GB for Llama-3.3-70B FP8
- **Access**: Network-based (NFS or cloud storage)

## GAIE Integration Benefits

1. **Service Discovery**: Automatic detection of worker instances
2. **Load Balancing**: HTTPRoute distributes requests across worker instances
3. **Traffic Shaping**: Support for custom routing rules and rate limiting
4. **Observability**: Built-in metrics for request routing and backend health
5. **Multi-tenancy**: Namespace-isolated deployments with RBAC

## Troubleshooting

### Pods Stuck in Pending
```bash
kubectl describe pod <pod-name> -n ${NAMESPACE}
# Check node affinity and GPU availability
```

### Routing Not Working
```bash
# Verify HTTPRoute is bound to gateway
kubectl describe httproute -n ${NAMESPACE}

# Check InferencePool endpoints
kubectl get endpoints llama3-70b-disagg-pool -n ${NAMESPACE}
```

### Worker Communication Failures
```bash
# Check inter-pod connectivity
kubectl logs -l app=llama3-70b-disagg -n ${NAMESPACE} --tail=50

# Verify shared memory allocation
kubectl exec -it <pod-name> -n ${NAMESPACE} -- df -h /dev/shm
```

## Performance Optimization

### Batch Size
- Start with batch size of 32-64 for optimal GPU utilization
- Maximum recommended batch size: 256

### Max Tokens
- Configure based on your latency SLO
- Recommended: 512-2048 for balanced throughput/latency

### TP Degree
- Currently set to TP=8 for both prefill and decode
- Adjust based on your GPU model (8x H100 = TP8)

## Related Documentation

- [GAIE v1.4.0 Compatibility Guide](../../../../docs/kubernetes/inference-gateway.md)
- [Disaggregated Deployment Guide](../README.md)
- [Dynamo Kubernetes Architecture](../../../../docs/kubernetes/README.md)
- [vLLM Disaggregated Serving](../../../../docs/backends/vllm.md#disaggregated)

## Version Requirements

| Component | Version | Required |
|-----------|---------|----------|
| GAIE | v1.4.0+ | ✅ |
| Kubernetes | v1.27+ | ✅ |
| kGateway | latest stable | ✅ |
| Dynamo | latest | ✅ |
