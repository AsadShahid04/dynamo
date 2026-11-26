---
# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
title: Update and Roll Back a DGD
subtitle: Apply updates to a running DynamoGraphDeployment, monitor rollout progress, and perform last-known-good rollbacks
---

Use this guide to update a running `DynamoGraphDeployment` (DGD) — changing worker images, arguments, resources, or pod template fields — and to roll back to a previous known-good configuration when needed.

## Prerequisites

- A running DGD deployed with [Deploy with DGD](../model-deployment/deploy-with-dgd.md)
- `kubectl` access to the cluster
- For a rollback, a copy of the last known-good DGD spec

## Update strategies and component support

The Dynamo operator manages rolling updates for single-node Deployment-backed workers. Updates to other component types and topologies have different behaviors:

| Component / Topology | Update Strategy | Behavior |
|---|---|---|
| **Deployment-backed single-node workers** | `RollingUpdate` (default) | Managed rolling updates with surge/unavailable controls |
| **Grove `PodCliqueSet` workers** | `OnDelete` | Pods update only when manually deleted; no automatic rollout |
| **LWS workers** | In-place | LWS replicas update in place; no new generations |
| **Multinode workers** | In-place | Multinode pods update in place; no rolling generations |
| **Frontend, EPP, Planner** | Rolling | Standard Kubernetes Deployment rolling updates |

The default `RollingUpdate` strategy for single-node workers creates new pods before deleting old ones, controlled by `maxSurge` and `maxUnavailable` annotations. To update in place instead (all old pods deleted before new ones start), use the `Recreate` strategy per component.

> [!WARNING]
> **Known limitation:** The operator currently provides no automatic rollback or rollout timeout. Monitor rollout progress and intervene manually if a rollout stalls.

## Update a DGD spec

DGD updates are **patch-style**. Edit the spec file or use `kubectl patch` to change only the fields you need — the operator merges your change with the existing resource rather than replacing it. This approach preserves fields managed by the operator or set by other controllers.

> [!IMPORTANT]
> **Do not use replace-style apply.** While issue [#13495](https://github.com/ai-dynamo/dynamo/issues/13495) remains open, `kubectl replace` or `helm upgrade --force` can leave stranded component generations. Always use patch-style `kubectl apply` or `kubectl patch`.

### Validate and apply an update

Before applying, perform a **server-side dry run** to catch validation errors:

```bash
kubectl apply -f updated-dgd.yaml -n ${NAMESPACE} --dry-run=server
```

If validation passes, apply the change:

```bash
kubectl apply -f updated-dgd.yaml -n ${NAMESPACE}
```

The apply increments `metadata.generation`. Watch for the operator to observe and reconcile it:

```bash
kubectl get dynamographdeployment <name> -n ${NAMESPACE} \
  -o jsonpath='{.metadata.generation}{" "}{.status.observedGeneration}{"\n"}'
```

Once `status.observedGeneration` matches `metadata.generation`, the operator has begun reconciling the new spec.

### Monitor rollout progress

For Deployment-backed workers with `RollingUpdate` strategy, the DGD status tracks rollout phases through `status.rollingUpdate`:

```bash
kubectl get dynamographdeployment <name> -n ${NAMESPACE} \
  -o jsonpath='{.status.rollingUpdate.phase}{"\n"}'
```

**Rollout phases:**

- `Pending` — rollout queued but not yet started
- `InProgress` — new pods are being created or old pods are being deleted
- `Completed` — all worker replicas match the current spec
- `Failed` — rollout encountered an error (currently **not** set by the controller for timeout or other transient failures)

Watch the full DGD status continuously:

```bash
kubectl get dynamographdeployment <name> -n ${NAMESPACE} -w
```

Or wait for the `Ready` condition to become `True` again after observing the new generation:

```bash
kubectl wait --for=condition=Ready dynamographdeployment/<name> \
  -n ${NAMESPACE} --timeout=900s
```

### Check generated DynamoComponentDeployment resources

Each DGD component becomes a `DynamoComponentDeployment` (DCD). List them to see per-component rollout state:

```bash
kubectl get dynamocomponentdeployment -n ${NAMESPACE} \
  -l nvidia.com/graph-deployment-name=<dgd-name>
```

Inspect a specific DCD's events for detailed rollout activity:

```bash
kubectl describe dynamocomponentdeployment <dcd-name> -n ${NAMESPACE}
```

For Deployment-backed workers, the operator also creates standard Kubernetes Deployments. Check their rollout status:

```bash
kubectl rollout status deployment/<deployment-name> -n ${NAMESPACE}
```

## Configure rolling update parameters

For Deployment-backed single-node workers, control surge and unavailability with annotations on the **component** in the DGD spec:

```yaml
spec:
  components:
  - name: VllmWorker
    type: worker
    annotations:
      nvidia.com/deployment-rolling-update-max-surge: "1"
      nvidia.com/deployment-rolling-update-max-unavailable: "0"
    replicas: 8
    podTemplate:
      # ...
```

- `max-surge` — maximum number of new pods created above desired replicas (default `1`)
- `max-unavailable` — maximum number of pods that can be unavailable during the update (default `1`)

> [!WARNING]
> **Availability gap:** Issue [#13622](https://github.com/ai-dynamo/dynamo/issues/13622) documents a zero-routable-worker gap when using `maxSurge: 0` and `maxUnavailable: 1`. During the rollout, all old pods may terminate before new pods become ready, causing temporary request failures. Use `maxSurge: 1` and `maxUnavailable: 0` to avoid this gap when continuous availability is required.

### Use Recreate strategy for in-place updates

To delete all old pods before starting new ones (accepting temporary downtime), set the strategy per component:

```yaml
spec:
  components:
  - name: VllmWorker
    type: worker
    annotations:
      nvidia.com/deployment-strategy: "Recreate"
    # ...
```

This guarantees no overlap between old and new pods, which can be useful when resource constraints prevent surge or when the update requires exclusive access to a resource.

## Grove and LWS update behavior

**Grove `PodCliqueSet` workers** use the `OnDelete` update strategy. After you apply a spec change, the operator updates the `PodCliqueSet` template but does not automatically delete running pods. To roll out the update:

1. Apply the DGD change
2. Manually delete each Grove worker pod:
   ```bash
   kubectl delete pod <pod-name> -n ${NAMESPACE}
   ```
3. The `PodCliqueSet` controller recreates the pod with the new spec

> [!NOTE]
> Issue [#8173](https://github.com/ai-dynamo/dynamo/issues/8173) tracks managed rolling updates for Grove. Until resolved, Grove updates are manual and require pod deletion.

**LWS workers** update in place. The LWS controller applies the new pod template to existing replicas without creating new generations. This avoids pod churn but means updates take effect only after the LWS reconciler restarts the affected containers.

## Updating a DGDR-generated DGD

A `DynamoGraphDeploymentRequest` (DGDR) generates and owns a DGD. After the initial profiling and deployment, the DGDR spec becomes **immutable** except for the deferred runtime flow. To change a DGDR-managed DGD:

1. **Take ownership of the generated DGD.** The DGD survives DGDR deletion but remains owned by it while the DGDR exists. Delete the DGDR to release ownership:
   ```bash
   kubectl delete dynamographdeploymentrequest <dgdr-name> -n ${NAMESPACE}
   ```
   The generated DGD persists and continues serving traffic.

2. **Apply updates directly to the DGD.** Now that the DGD is no longer owned by a DGDR, edit and apply it as shown above.

> [!IMPORTANT]
> Once you delete the DGDR and take ownership of the DGD, the Planner stops managing autoscaling and topology changes. The DGD becomes a static, operator-reconciled resource. To regain Planner management, create a new DGDR and migrate traffic, or manually apply Planner recommendations.

For more on the DGDR lifecycle, see [Auto Deploy with DGDR](../auto-deployment/auto-deploy-with-dgdr.md).

## Roll back to a known-good configuration

The operator does not currently provide automatic rollback or a rollout undo command. To revert a failed or unwanted update, manually reapply the last known-good DGD spec.

> [!WARNING]
> **Do not revert during an active rollout.** Issue [#13620](https://github.com/ai-dynamo/dynamo/issues/13620) shows that reverting from spec A to B and back to A during `InProgress` can bypass `maxUnavailable` and delete all serving pods, causing a full outage. Only roll back after the rollout reaches `Completed` or after you have manually stabilized the deployment.

### Rollback procedure

1. **Wait for rollout completion or stabilize manually.** Confirm `status.rollingUpdate.phase` is `Completed`, or manually delete problematic new pods and wait for the old generation to stabilize.

2. **Reapply the last known-good spec:**
   ```bash
   kubectl apply -f last-known-good-dgd.yaml -n ${NAMESPACE}
   ```

3. **Monitor the rollback as an update.** The rollback is itself a DGD update and follows the same rollout process. Watch `observedGeneration` and `rollingUpdate.phase`:
   ```bash
   kubectl get dynamographdeployment <name> -n ${NAMESPACE} -w
   ```

4. **Verify the service.** Once `Ready` is `True`, send test requests to confirm the rollback succeeded.

### Keep a known-good backup

Before applying any update, save the current DGD spec:

```bash
kubectl get dynamographdeployment <name> -n ${NAMESPACE} -o yaml > dgd-backup-$(date +%Y%m%d-%H%M%S).yaml
```

This backup includes the full spec, status, and metadata. To restore, extract only the `spec` and `metadata.name`/`metadata.namespace` fields and reapply.

## Frontend and worker version compatibility

Dynamo frontends and workers maintain API compatibility within a release series, but mixing major versions or pre-release builds can cause request failures or undefined behavior. During a rolling update:

- **Keep the frontend and worker images on the same release** (for example, `1.4.0`).
- **Update the frontend after workers stabilize** to ensure new requests route to compatible workers.
- **Avoid mixed-version operations that depend on unreleased protocol changes.**

For the exact compatibility matrix and version skew policy, see [Frontend-Worker Compatibility](https://github.com/ai-dynamo/dynamo/issues/13886) (tracks the documentation gap; refer to release notes until that issue is resolved).

## Graceful shutdown and request draining

When a worker pod terminates, the operator signals it to drain in-flight requests before the container stops. This reduces user-visible request failures during updates. The drain window is controlled by:

- `terminationGracePeriodSeconds` on the pod (default 30s)
- Worker-side drain timeout and readiness probe configuration

For details on configuring graceful shutdown behavior, see [Graceful Shutdown](../fault-tolerance/graceful-shutdown.md).

> [!TIP]
> Increase `terminationGracePeriodSeconds` for workloads with long-running requests (for example, large output token counts) to allow all in-flight requests to complete before the pod is killed.

## Troubleshooting updates

### Update not reconciling

**Symptom:** `observedGeneration` does not match `metadata.generation` after applying.

**Cause:** Operator may be stopped, the namespace may be paused, or validation failed silently.

**Resolution:**
1. Check operator logs:
   ```bash
   kubectl logs -n dynamo-system deployment/dynamo-operator --tail=100
   ```
2. Describe the DGD for events:
   ```bash
   kubectl describe dynamographdeployment <name> -n ${NAMESPACE}
   ```
3. Confirm the DGD passed validation with `--dry-run=server` before applying.

### Rollout stuck in InProgress

**Symptom:** `status.rollingUpdate.phase` stays `InProgress` for longer than expected.

**Cause:** New pods may be failing to start, old pods may be stuck terminating, or insufficient cluster resources.

**Resolution:**
1. Check pod status:
   ```bash
   kubectl get pods -n ${NAMESPACE} -l nvidia.com/graph-deployment-name=<dgd-name>
   ```
2. Inspect failing pods:
   ```bash
   kubectl describe pod <pod-name> -n ${NAMESPACE}
   kubectl logs <pod-name> -n ${NAMESPACE} -c main
   ```
3. Common causes:
   - Image pull errors (invalid tag, missing pull secret)
   - Insufficient GPU or CPU resources
   - Volume mount failures
   - Startup probe failures due to slow model loading

### Pods not routable after update

**Symptom:** New pods are `Running` but requests fail or timeout.

**Cause:** New pods may not have passed readiness probes, or the KV-aware router has stale worker state.

**Resolution:**
1. Check pod readiness:
   ```bash
   kubectl get pods -n ${NAMESPACE} -l nvidia.com/graph-deployment-name=<dgd-name>
   ```
   Confirm `READY` shows `1/1` or the expected container count.
2. Inspect readiness probe failures:
   ```bash
   kubectl describe pod <pod-name> -n ${NAMESPACE}
   ```
3. For KV-aware routing, confirm the router sees the new workers:
   ```bash
   kubectl logs -n ${NAMESPACE} deployment/<dgd-name>-frontend -c main | grep "worker discovered"
   ```

### Grove update not taking effect

**Symptom:** Applied a DGD change for a Grove worker but pods still run the old spec.

**Cause:** Grove uses `OnDelete` — pods do not automatically restart.

**Resolution:**
1. Confirm the `PodCliqueSet` template updated:
   ```bash
   kubectl get podcliquesets -n ${NAMESPACE}
   kubectl describe podcliquesets <pcs-name> -n ${NAMESPACE}
   ```
2. Manually delete each pod:
   ```bash
   kubectl delete pod <pod-name> -n ${NAMESPACE}
   ```
3. Verify new pods use the updated spec.

## Related resources

- [Deploy with DGD](../model-deployment/deploy-with-dgd.md) — initial DGD authoring and deployment
- [DynamoGraphDeployment API Reference](../../reference/kubernetes-api/dynamo-graph-deployment.mdx) — full DGD spec and status fields
- [Graceful Shutdown](../fault-tolerance/graceful-shutdown.md) — configure request draining during pod termination
- [Observability](observability.mdx) — monitor rollout progress with metrics and events
- [Auto Deploy with DGDR](../auto-deployment/auto-deploy-with-dgdr.md) — DGDR-to-DGD lifecycle and ownership
