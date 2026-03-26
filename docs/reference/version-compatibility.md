---
# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
title: Version Compatibility Matrix
subtitle: Python, Kubernetes, and upgrade-path compatibility for Dynamo releases
---

**See also:** [Support Matrix](support-matrix.md) for hardware, CUDA, and backend versions | [Release Artifacts](release-artifacts.md) for container images and wheels

## Python Version Compatibility

| **Dynamo** | **Python 3.10** | **Python 3.11** | **Python 3.12** | **Notes** |
| :--------- | :-------------- | :-------------- | :-------------- | :-------- |
| **main (ToT)** | Supported | Partial¹ | Supported | |
| **v1.1.0-dev.1** | Supported | Partial¹ | Supported | |
| **v1.0.1** | Supported | Partial¹ | Supported | |
| **v1.0.0** | Supported | Partial¹ | Supported | |
| **v0.9.x** | Supported | Partial¹ | Supported | |
| **v0.8.x** | Supported | Partial¹ | Experimental² | |
| **v0.7.x** | Supported | Partial¹ | Not tested | |

**Notes:**
1. **Python 3.11 — TensorRT-LLM excluded.** The `ai-dynamo[trtllm]` extra is incompatible with
   Python 3.11 due to upstream TensorRT-LLM constraints. All other extras (`vllm`, `sglang`, `kvbm`)
   are supported.
2. Python 3.12 support was introduced experimentally in v0.8.0 and is required for the KV Block
   Manager (`kvbm`). Full Python 3.12 support (all extras) became stable in v1.0.0.

---

## Kubernetes Version Compatibility

| **Dynamo** | **K8s 1.27** | **K8s 1.28** | **K8s 1.29** | **K8s 1.30** | **K8s 1.31** | **K8s 1.32** |
| :--------- | :----------- | :----------- | :----------- | :----------- | :----------- | :----------- |
| **main (ToT)** | — | Supported | Supported | Supported | Supported | Supported |
| **v1.0.x** | — | Supported | Supported | Supported | Supported | Supported |
| **v0.9.x** | Supported | Supported | Supported | Supported | Supported | — |
| **v0.8.x** | Supported | Supported | Supported | Supported | — | — |

Dynamo uses `apiextensions.k8s.io/v1` CRDs which requires Kubernetes 1.22+. Older clusters
are not supported.

---

## Operator API Version Compatibility

The Dynamo Kubernetes Operator exposes CRDs under `nvidia.com/v1alpha1`. The table below shows
which operator minor version introduced or deprecated each CRD/field.

| **CRD / Field** | **Introduced** | **Deprecated** | **Notes** |
| :-------------- | :------------- | :------------- | :-------- |
| `DynamoGraphDeployment` | v0.6.0 | — | Core CRD |
| `DynamoComponentDeployment` | v0.6.0 | — | Core CRD |
| `DynamoGraphDeploymentRequest` | v0.8.0 | — | Auto-profiling |
| `DynamoGraphDeploymentScalingAdapter` | v1.0.0 | — | HPA/KEDA integration |
| `DynamoCheckpoint` | v1.0.0 | — | Fast pod restore |
| `spec.services[*].autoscaling` | v0.8.0 | v1.0.0 | Use `scalingAdapter` instead |

---

## Upgrade Compatibility

The following table indicates safe upgrade paths between Dynamo operator versions.

| **From** | **To** | **Safe?** | **Notes** |
| :------- | :----- | :-------- | :-------- |
| v0.8.x | v0.9.x | Yes | CRD schema additive |
| v0.9.x | v1.0.x | Yes | `autoscaling` field deprecated; migrate to `scalingAdapter` |
| v1.0.x | main | Yes | No breaking CRD changes |
| v0.7.x | v0.9.x | Manual | Skip-version upgrade; re-apply CRDs manually |
| v0.6.x | v0.8.x | Manual | Skip-version upgrade; re-apply CRDs manually |

> [!Tip]
> Always upgrade one minor version at a time unless the table above explicitly marks
> a cross-version jump as safe. Run `helm upgrade` with `--atomic` to roll back
> automatically on failure.

### CRD Upgrade Notes

- CRDs are managed by the operator via a pre-install/pre-upgrade Helm hook that runs
  `crd-apply` with server-side apply.  No manual `kubectl apply -f crds/` is needed
  when using Helm >= v3.8.
- If you manage CRDs out-of-band (e.g. GitOps), set `upgradeCRD: false` in your
  `values.yaml` and apply the CRD manifests from the Helm chart's `crds/` directory
  yourself before upgrading the operator.

---

## Component Compatibility Within a Deployment

When running disaggregated or multi-component deployments, all components must use the
**same Dynamo release**. Mixing component versions across minor releases is not supported.

| **Scenario** | **Supported?** |
| :----------- | :------------- |
| Prefill + Decode workers from same Dynamo version | Yes |
| Frontend + Worker from same Dynamo version | Yes |
| Frontend v1.0.x + Worker v0.9.x | No |
| Rolling upgrade of workers while frontend serves traffic | Yes (patch versions only) |
