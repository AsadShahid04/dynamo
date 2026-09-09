---
# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
title: Restart a DGD
subtitle: Trigger a graph-level restart of an existing DynamoGraphDeployment by updating spec.restart, and monitor the restart through completion.
---

## What a DGD restart does

A **DGD restart** triggers a rolling restart of all components in an existing `DynamoGraphDeployment` (DGD). The restart is orchestrated at the graph level by changing `spec.restart.id` to a new unique value, and the Dynamo operator coordinates the component rollouts according to the configured strategy. This operation is distinct from applying a new `spec` revision that triggers a managed rolling update.

Use a restart to:

- Apply a backend configuration change that running processes cannot reload (for example, updating vLLM engine arguments or SGLang memory settings)
- Recover from a transient backend failure where pods remain scheduled but the inference engine is unhealthy
- Reset worker state after a manual intervention or diagnostic operation

> [!NOTE]
> A restart does not redeploy the graph from scratch or change the `spec.components` list. It coordinates the rollout of existing component definitions. If you need to change replicas, images, or resource requests, apply a new `spec` revision instead and let the operator perform a managed rolling update — see [Deploy with DGD](../model-deployment/deploy-with-dgd.md).

## Prerequisites

Before triggering a restart, ensure:

- A **healthy existing DGD** is deployed and serving traffic. The DGD must have been created successfully and must not include `spec.restart` in its initial manifest — the `v1beta1` validation rule rejects `spec.restart` on creation.
- `kubectl` access to the cluster and the DGD's namespace.
- No active **managed rolling update** is currently in the `Pending` or `InProgress` phase. The operator rejects a new `restart.id` while a managed rolling update is active.

### Verify current readiness

Before initiating a restart, confirm the DGD is healthy and no prior operation is still active:

```bash
export NAMESPACE=your-namespace
export DGD_NAME=my-graph

# Check the DGD status
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.status.observedGeneration}' && echo
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}' && echo

# Verify no restart or rolling update is in progress
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.status.restart.inProgress}' && echo
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.status.rollingUpdate.phase}' && echo
```

A healthy, idle DGD shows:

- `.status.observedGeneration` matches `.metadata.generation` (the operator has observed the current spec)
- The `Ready` condition is `True`
- `.status.restart.inProgress` is `false` or absent
- `.status.rollingUpdate.phase` is absent, `Completed`, or not `Pending`/`InProgress`

If a prior restart or rolling update is still active, wait for it to complete before submitting a new `restart.id`.

## Restart strategies

The `spec.restart.strategy` field controls how component rollouts are coordinated. The strategy applies to **component-level** coordination; within each component, the backing Deployment or Grove `PodCliqueSet` controls the pod-level rollout according to its own `spec.strategy`.

| Strategy | Behavior | Use when |
|----------|----------|----------|
| **Sequential** (default) | Components restart one at a time. When `strategy.order` is omitted, the operator uses the alphabetical order of component names. When `strategy.order` is supplied, the operator uses that exact sequence. | You want to control the order of component restarts, minimize concurrent disruption, or prioritize specific components (for example, restart workers before the frontend). |
| **Parallel** | All components restart concurrently. | You need the fastest possible restart and can tolerate the availability impact of all components rolling at once. The actual availability depends on per-component `replicas` and the underlying rollout strategy. |

### Component versus pod rollout

The restart strategy coordinates **components**, not individual pods. Each component's pod-level rollout is governed by its own provider:

- **Deployment-backed components** (the default when `multinode` is not set) use the Deployment's `spec.strategy` (typically `RollingUpdate` with `maxUnavailable` and `maxSurge`).
- **Grove-backed components** (when `multinode` is set) use the Grove `PodCliqueSet` rollout strategy.

> [!NOTE]
> Sequential restart serializes components, not replicas. A component with `replicas: 3` and a Deployment `RollingUpdate` strategy may have multiple pods terminating and starting concurrently, but the next component in the sequence will not start its rollout until the current component's rollout completes.

## Perform a restart

Every restart requires a new unique `restart.id`. The operator ignores a `restart.id` it has already processed. The ID is an arbitrary string and does not need to follow a specific format; a timestamp or UUID is a common choice.

### Step 1: Generate a unique restart ID

Choose a new restart ID. Common patterns include:

```bash
# Timestamp-based ID
RESTART_ID="restart-$(date +%Y%m%d-%H%M%S)"

# UUID-based ID
RESTART_ID="restart-$(uuidgen | tr '[:upper:]' '[:lower:]')"

echo "Generated restart ID: ${RESTART_ID}"
```

Record the ID for your operational logs.

### Step 2: Dry-run the restart patch

Before applying the restart, perform a **server-side dry-run** to verify the patch is valid and will be accepted by the operator's webhook:

```bash
kubectl patch dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} \
  --type=merge \
  --patch "{\"spec\":{\"restart\":{\"id\":\"${RESTART_ID}\"}}}" \
  --dry-run=server
```

A successful dry-run confirms:

- The patch syntax is valid
- No managed rolling update is currently blocking admission
- The operator webhook accepts the new `restart.id`

If the dry-run fails with an admission error, check that no rolling update is in the `Pending` or `InProgress` phase and that the ID is new.

### Step 3: Apply the restart

Apply the restart patch to trigger the graph restart:

<Tabs>
<Tab title="Default sequential">

The default sequential strategy restarts components in **alphabetical order** by component name. To use the default order, omit `strategy.order`:

```bash
kubectl patch dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} \
  --type=merge \
  --patch "{\"spec\":{\"restart\":{\"id\":\"${RESTART_ID}\"}}}"
```

**Example**: A DGD with components `Frontend`, `VllmDecodeWorker`, and `VllmPrefillWorker` will restart in the order `Frontend`, `VllmDecodeWorker`, `VllmPrefillWorker`.

To see the exact component names and their alphabetical order:

```bash
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.spec.components[*].name}' | tr ' ' '\n' | sort
```

</Tab>

<Tab title="Explicit sequential order">

To restart components in a **specific order**, set `strategy.order` to the complete ordered list of component names. The list must include every component name exactly once, and is invalid for parallel restarts.

**Example**: Restart workers before the frontend to ensure inference capacity is ready when the frontend comes back online:

```bash
# Derive the complete component list
COMPONENTS=$(kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.spec.components[*].name}' | tr ' ' '\n' | jq -R . | jq -s .)

# Inspect and reorder as needed
echo "Current components: ${COMPONENTS}"

# Apply with explicit order (example: workers first, then frontend)
kubectl patch dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} \
  --type=merge \
  --patch "{
    \"spec\": {
      \"restart\": {
        \"id\": \"${RESTART_ID}\",
        \"strategy\": {
          \"type\": \"Sequential\",
          \"order\": [\"VllmPrefillWorker\", \"VllmDecodeWorker\", \"Frontend\"]
        }
      }
    }
  }"
```

> [!WARNING]
> The `strategy.order` list must be complete and match the exact component names in `spec.components`. A misspelled name or a partial list will cause the patch to be rejected.

</Tab>

<Tab title="Parallel">

A **parallel restart** starts all component rollouts concurrently. To trigger a parallel restart, set `strategy.type` to `Parallel` and omit `strategy.order` (it is invalid for parallel restarts):

```bash
kubectl patch dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} \
  --type=merge \
  --patch "{
    \"spec\": {
      \"restart\": {
        \"id\": \"${RESTART_ID}\",
        \"strategy\": {
          \"type\": \"Parallel\"
        }
      }
    }
  }"
```

**Availability impact**: A parallel restart does not guarantee simultaneous pod termination or a total outage. The actual availability depends on:

- Per-component `replicas` count
- The backing Deployment or Grove `spec.strategy` (for example, `maxUnavailable` for Deployments)
- Graceful shutdown duration and in-flight request migration (if enabled)

A parallel restart is fastest but may have higher request-time impact than a sequential restart. Use it when end-to-end restart speed is more important than maintaining continuous availability.

</Tab>
</Tabs>

## Monitor restart progress

After applying the restart, monitor `.status.restart` to track progress:

```bash
# Watch the restart status
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} --watch -o jsonpath='{.status.restart}{"\n"}'
```

The operator maintains the following fields under `.status.restart`:

- `.observedID`: The restart ID the operator is currently processing. When this matches your submitted `restart.id`, the restart has been acknowledged.
- `.phase`: The current restart phase. See [Restart phases](#restart-phases) below.
- `.inProgress`: `true` while the restart is active, `false` when it completes or is superseded.

You can also watch component-level rollout progress via each component's `DynamoComponentDeployment`:

```bash
kubectl get dynamocomponentdeployment -n ${NAMESPACE} -l nvidia.com/dynamo-graph=${DGD_NAME}
```

### Restart phases

The operator sets `.status.restart.phase` to one of the following values during a restart:

| Phase | Meaning | Next step |
|-------|---------|-----------|
| **Restarting** | The restart is in progress. Components are being rolled out according to the configured strategy. | Wait for the phase to transition to `Completed` or `Superseded`. |
| **Completed** | The restart finished successfully. All components have been restarted and are healthy. | Verify the DGD and proceed. |
| **Superseded** | The restart was superseded by a new managed rolling update that entered the `Pending` or `InProgress` phase while the restart was active. The restart stops, and the rolling update takes over. | Inspect the rolling update status. If the rolling update completes and the original restart goal is still needed, submit a new `restart.id`. |

The API also declares `Pending` and `Failed` restart phases, but **current reconciliation logic does not assign them**. There is no restart timeout or automatic transition to `Failed`; a component that never becomes ready can leave the restart in the `Restarting` phase indefinitely. If a restart appears stuck:

1. Inspect the component's pods and logs to identify the underlying issue
2. Fix the root cause (for example, correct an invalid backend argument or resolve a resource contention issue)
3. If the component becomes healthy, the restart will complete
4. If the issue cannot be resolved in place, consider rolling back the `spec` or deploying a corrected configuration as a new managed rolling update

> [!NOTE]
> Do not wait for a `Failed` phase — it is declared in the API but not currently produced. Monitor pod health, component readiness, and logs to detect and resolve stuck restarts manually.

## Admission and conflicts with rolling updates

The operator enforces the following admission rules:

- **On DGD creation**: `spec.restart` must be absent. The v1beta1 validation rule rejects a manifest that includes `spec.restart` when the DGD is first created.
- **While a rolling update is active**: A new `restart.id` is rejected if a managed rolling update is in the `Pending` or `InProgress` phase. You must wait for the rolling update to complete or reach another phase before submitting a restart.
- **While a restart is in progress**: If a managed rolling update enters the `Pending` or `InProgress` phase (for example, because you applied a new `spec` revision), the operator marks the in-flight restart as `Superseded` and emits a `RestartSuperseded` event. The rolling update takes precedence, and the restart stops.

To check whether a rolling update is blocking a restart:

```bash
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.status.rollingUpdate.phase}' && echo
```

If the phase is `Pending` or `InProgress`, wait for it to transition before submitting a new `restart.id`.

## Verify after restart

Once `.status.restart.phase` is `Completed`, verify the DGD is healthy and serving traffic:

<Steps>
<Step title="Check DGD readiness">

```bash
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}' && echo
```

The `Ready` condition should be `True`.

</Step>

<Step title="Check pods">

```bash
kubectl get pods -n ${NAMESPACE} -l nvidia.com/dynamo-graph=${DGD_NAME}
```

All pods should be `Running` and ready.

</Step>

<Step title="Check models endpoint">

Forward the frontend service and verify the models list:

```bash
kubectl port-forward -n ${NAMESPACE} svc/${DGD_NAME}-frontend 8000:8000 &
sleep 2
curl http://127.0.0.1:8000/v1/models
```

The response should include your model.

</Step>

<Step title="Run an inference smoke test">

Send a simple chat completion request:

```bash
curl http://127.0.0.1:8000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "your-model-name",
    "messages": [{"role": "user", "content": "Hello!"}],
    "max_tokens": 16
  }'
```

A successful response confirms the graph is healthy and serving requests.

</Step>
</Steps>

## Example: Full restart workflow

Below is a complete workflow for restarting a DGD with an explicit sequential order:

```bash
export NAMESPACE=my-namespace
export DGD_NAME=qwen3-32b-agg

# 1. Verify the DGD is healthy and idle
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}' && echo
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.status.restart.inProgress}' && echo

# 2. List component names and decide on order
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.spec.components[*].name}' | tr ' ' '\n' | sort

# 3. Generate a restart ID
RESTART_ID="restart-$(date +%Y%m%d-%H%M%S)"
echo "Restart ID: ${RESTART_ID}"

# 4. Dry-run the restart
kubectl patch dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} \
  --type=merge \
  --patch "{
    \"spec\": {
      \"restart\": {
        \"id\": \"${RESTART_ID}\",
        \"strategy\": {
          \"type\": \"Sequential\",
          \"order\": [\"VllmWorker\", \"Frontend\"]
        }
      }
    }
  }" \
  --dry-run=server

# 5. Apply the restart
kubectl patch dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} \
  --type=merge \
  --patch "{
    \"spec\": {
      \"restart\": {
        \"id\": \"${RESTART_ID}\",
        \"strategy\": {
          \"type\": \"Sequential\",
          \"order\": [\"VllmWorker\", \"Frontend\"]
        }
      }
    }
  }"

# 6. Watch progress
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} --watch -o jsonpath='{.status.restart}{"\n"}'

# 7. After completion, verify readiness
kubectl get dynamographdeployment ${DGD_NAME} -n ${NAMESPACE} -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}' && echo
kubectl get pods -n ${NAMESPACE} -l nvidia.com/dynamo-graph=${DGD_NAME}

# 8. Smoke test
kubectl port-forward -n ${NAMESPACE} svc/${DGD_NAME}-frontend 8000:8000 &
sleep 2
curl http://127.0.0.1:8000/v1/models
```

## Events and logs

During a restart, the operator emits Kubernetes events that can help diagnose issues:

```bash
kubectl get events -n ${NAMESPACE} --sort-by=.lastTimestamp | grep -i restart
```

Look for:

- `RestartInitiated`: The restart has started
- `RestartCompleted`: The restart finished successfully
- `RestartSuperseded`: The restart was superseded by a rolling update

To inspect component-level logs:

```bash
# Frontend logs
kubectl logs -n ${NAMESPACE} -l nvidia.com/dynamo-graph=${DGD_NAME},nvidia.com/dynamo-component=Frontend --tail=100

# Worker logs
kubectl logs -n ${NAMESPACE} -l nvidia.com/dynamo-graph=${DGD_NAME},nvidia.com/dynamo-component=VllmWorker --tail=100
```

## Troubleshooting

| Symptom | Cause | Resolution |
|---------|-------|------------|
| Patch rejected with admission error | A managed rolling update is in `Pending` or `InProgress` phase | Wait for the rolling update to complete, then retry the restart |
| `.status.restart.observedID` does not match submitted ID | The operator has not yet processed the new ID | Wait a few seconds and check again. If it remains stuck, inspect operator logs for reconciliation errors |
| Restart stuck in `Restarting` phase | A component's pods are not becoming ready | Inspect pod status and logs. Fix the underlying issue (for example, incorrect backend arguments, resource contention, or image pull errors). The restart will complete once the component is healthy |
| `Superseded` phase after applying restart | A new `spec` revision or managed rolling update was applied while the restart was in progress | The rolling update takes precedence. If the restart is still needed after the rolling update completes, submit a new `restart.id` |

For broader deployment troubleshooting, see the [troubleshoot-dynamo skill failure decision tree](https://github.com/ai-dynamo/dynamo/blob/main/.agents/skills/troubleshoot-dynamo/references/failure-decision-tree.md).

## Related operations

- **[Deploy with DGD](../model-deployment/deploy-with-dgd.md)**: Create and apply a new DGD from scratch
- **[DynamoGraphDeployment API Reference](../../reference/kubernetes-api/dynamo-graph-deployment.mdx)**: Full field reference for the DGD CRD, including the `spec.restart` field
- **[Performance Tuning](performance-tuning.md)**: Optimize backend engine arguments and resource allocations that may require a restart to take effect
