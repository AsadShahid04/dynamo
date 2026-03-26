<!-- SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
SPDX-License-Identifier: Apache-2.0 -->

# Llama-3.3-70B — Aggregated GAIE Recipe

Deploy **Llama-3.3-70B-Instruct** in aggregated mode with
[Gateway API Inference Extension (GAIE)](../../../../../deploy/inference-gateway/README.md)
for KV-aware, gateway-level load balancing.

## Prerequisites

1. Dynamo Platform installed on the cluster.
2. GAIE (Gateway API Inference Extension) installed — see
   [Inference Gateway documentation](../../../../../docs/kubernetes/inference-gateway.md).
3. A `Gateway` resource (e.g. Envoy Gateway or Istio) already provisioned.
4. Model downloaded — run the model-cache job from `recipes/llama-3-70b/model-cache/` first.
5. HuggingFace token secret:
   ```bash
   kubectl create secret generic hf-token-secret \
     --from-literal=HF_TOKEN="<your-token>" \
     -n <namespace>
   ```

## Deploy

```bash
export NAMESPACE=dynamo-demo

# 1. Deploy the Dynamo inference graph (aggregated vLLM workers + EPP)
kubectl apply -f deploy.yaml -n ${NAMESPACE}

# 2. Wait for the deployment to be ready
kubectl wait --for=condition=Available \
  dynamographdeployment/llama3-70b-agg \
  -n ${NAMESPACE} --timeout=600s

# 3. Create the HTTPRoute for gateway-level routing
kubectl apply -f http-route.yaml -n ${NAMESPACE}
```

## Verify

```bash
# Check that EPP and worker pods are running
kubectl get pods -n ${NAMESPACE} -l app.kubernetes.io/instance=llama3-70b-agg

# Send a request through the Gateway
curl http://<gateway-address>/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Host: llama3-70b.example.com" \
  -d '{
    "model": "RedHatAI/Llama-3.3-70B-Instruct-FP8-dynamic",
    "messages": [{"role": "user", "content": "Hello!"}],
    "max_tokens": 50
  }'
```

## Files

| File | Description |
|------|-------------|
| `deploy.yaml` | `DynamoGraphDeployment` with EPP and aggregated vLLM workers |
| `http-route.yaml` | `HTTPRoute` that directs traffic through the Gateway to the EPP service |

## Notes

- The EPP in this recipe uses the `disagg-profile-handler` plugin with graceful
  degradation — it operates in aggregated mode (no prefill profile) and applies
  KV-overlap scoring to route requests to the worker with the best cache reuse.
- To switch to disaggregated mode, see `../../../disagg-single-node/gaie/`.
- Update the `Gateway` name and namespace in `http-route.yaml` to match your
  cluster's Gateway resource before applying.
