# Llama-3.3-70B Disaggregated Multi-Node Recipe

Disaggregated deployment of **Llama-3.3-70B-Instruct** using vLLM with FP8 dynamic quantization across 2 nodes.

## Overview

This configuration separates prefill and decode operations across dedicated GPU nodes:
- **Prefill Node**: 8 GPUs (H100/H200) for prompt processing
- **Decode Node**: 8 GPUs (H100/H200) for token generation
- **Total**: 16 GPUs across 2 nodes

The disaggregated architecture maximizes GPU utilization and throughput for large batch inference workloads.

## Prerequisites

1. **Dynamo Platform installed** — See [Kubernetes Deployment Guide](../../../../docs/kubernetes/README.md)
2. **2 GPU nodes** with 8 H100 or H200 GPUs each
3. **HuggingFace token** with access to Llama models
4. **Shared storage** for model cache (accessible from both nodes)

## Deployment Options

### Standard Deployment

```bash
export NAMESPACE=dynamo-demo
kubectl create namespace ${NAMESPACE}

# Create HuggingFace token secret
kubectl create secret generic hf-token-secret \
  --from-literal=HF_TOKEN="your-token-here" \
  -n ${NAMESPACE}

# Download model cache (ensure storageClassName matches your cluster)
kubectl apply -f ../model-cache/ -n ${NAMESPACE}
kubectl wait --for=condition=Complete job/model-download -n ${NAMESPACE} --timeout=3600s

# Deploy disaggregated configuration
kubectl apply -f deploy.yaml -n ${NAMESPACE}
```

### GAIE (Gateway API Inference Extension) Deployment

For production deployments with advanced networking and traffic management:

```bash
# Prerequisites: GAIE v1.4.0+ with kGateway
kubectl apply -f gaie/deploy.yaml -n ${NAMESPACE}
kubectl apply -f gaie/http-route.yaml -n ${NAMESPACE}
```

The GAIE deployment includes:
- **InferencePool** for request routing and load balancing
- **HTTPRoute** for external traffic ingress (port 8000)
- **Automatic service discovery** for distributed workers

## Components

### deploy.yaml
Kubernetes manifests defining:
- **Epp** (Endpoint/Frontend): Service for REST API exposure
- **VllmPrefillWorker**: Prefill-only worker group (TP=8, no decode)
- **VllmDecodeWorker**: Decode-only worker group (TP=8, no prefill)
- Shared model cache volume mounts
- GPU resource allocation (8 per worker)
- Disaggregation scheduling profiles

### GAIE Integration (gaie/ folder)
- **deploy.yaml**: DynamoGraphDeployment and InferencePool configuration
- **http-route.yaml**: HTTPRoute for ingress traffic routing

## Testing

```bash
# Port-forward the frontend (standard deployment)
kubectl port-forward svc/llama3-70b-disagg-multi-frontend 8000:8000 -n ${NAMESPACE}

# For GAIE deployment, route through the InferencePool
kubectl port-forward svc/llama3-70b-disagg-pool 8000:8000 -n ${NAMESPACE}

# Send a test request
curl http://localhost:8000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "RedHatAI/Llama-3.3-70B-Instruct-FP8-dynamic",
    "messages": [{"role": "user", "content": "Hello!"}],
    "max_tokens": 50,
    "temperature": 0.7
  }'
```

## Performance Notes

- **Prefill throughput**: ~250-300 tokens/sec per node with batch processing
- **Decode throughput**: ~400-600 tokens/sec per node (memory-bandwidth limited)
- **Latency**: Disaggregation adds ~50ms overhead vs. aggregated deployment
- **Total throughput**: Higher with large batches (100+ batch size)

## Storage Configuration

Update `storageClassName` in deployment manifests to match your Kubernetes cluster:
- `local-path` — For development/single-node clusters
- `nfs` — For shared network storage
- `ebs` (AWS) — For managed cloud storage

Model cache job downloads ~152GB and takes 15-30 minutes depending on network speed.

## Troubleshooting

### Pod Pending on Specific Nodes
Verify node selectors and affinity rules in deploy.yaml match your cluster topology.

### Out of Memory (OOM) Errors
Ensure sufficient GPU memory. Each worker requires ~80GB of GPU memory for Llama-3-70B with FP8.

### GAIE Routing Issues
- Verify GAIE v1.4.0+ is installed: `kubectl get crd | grep inference`
- Check InferencePool endpoint: `kubectl get inferencepools -n ${NAMESPACE}`
- Check HTTPRoute status: `kubectl describe httproute -n ${NAMESPACE}`

## Related Documentation

- [GAIE Recipe Guide](../../../../docs/kubernetes/inference-gateway.md)
- [Dynamo Kubernetes Deployment](../../../../docs/kubernetes/README.md)
- [vLLM Backend Configuration](../../../../docs/backends/vllm.md)
