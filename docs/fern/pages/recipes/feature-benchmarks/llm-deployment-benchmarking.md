---
# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
title: LLM Deployment Benchmarking
subtitle: Compare aggregated and disaggregated LLM deployments with AIPerf
---

Compare aggregated and disaggregated NVIDIA Dynamo deployments by sweeping
concurrency with [AIPerf](https://github.com/ai-dynamo/aiperf) and reading the
resulting throughput and latency curves. vLLM, SGLang, and TensorRT-LLM workers
all expose the same OpenAI-compatible HTTP API, so the benchmark commands below
stay the same after you change the launcher. The examples use vLLM and
`Qwen/Qwen3-0.6B`.

Load-generation options, in-cluster jobs, trace replay, and the full plot
catalog live in the [Dynamo Benchmarking guide](benchmarking-guide.md). This
page is the aggregated-versus-disaggregated comparison that used to live beside
`benchmarks/llm`.

> [!NOTE]
> `benchmarks/llm/perf.sh`, `plot_pareto.py`, and `nginx.conf` are not in the
> current tree. The last release tag that contains them is
> [v1.2.1](https://github.com/ai-dynamo/dynamo/tree/v1.2.1/benchmarks/llm).
> Run `aiperf profile` and `aiperf plot` directly. `aiperf plot` replaces
> `plot_pareto.py`, so this workflow does not install matplotlib, seaborn,
> pandas, or numpy.

## Prerequisites

The checked-in vLLM launchers run `Qwen/Qwen3-0.6B` on one GPU (aggregated) or
two GPUs (disaggregated). A larger prefill and decode layout, such as an 8x H100
node, is a sizing choice. Follow
[Disaggregated Serving](../../kubernetes/disaggregated-serving/overview.md)
and [Sizing with AIConfigurator](../../kubernetes/disaggregated-serving/sizing-with-aiconfigurator.mdx)
before you treat a GPU count as the comparison.

1. Install Dynamo for a
   [local machine](../../cli/installation/install-dynamo.mdx) or a
   [Kubernetes cluster](../../kubernetes/installation/install-dynamo.md).
   Local discovery uses etcd and NATS:

   ```bash
   docker compose -f dev/docker-compose.yml up -d
   ```

2. Install AIPerf on the machine that sends load. Dynamo runtime images already
   include it.

   ```bash
   pip install aiperf
   ```

3. Set `HF_TOKEN` before launch when the model is gated. To download weights
   ahead of time:

   ```bash
   hf download Qwen/Qwen3-0.6B
   ```

The `--model` value must match the model name the server advertises.

## Deploy a Disaggregated Endpoint

Pick Kubernetes or a local launcher. Both serve `http://localhost:8000` after
the port-forward or the local frontend is up.

### Kubernetes

[`examples/backends/vllm/deploy/disagg.yaml`](https://github.com/ai-dynamo/dynamo/blob/main/examples/backends/vllm/deploy/disagg.yaml)
is one prefill worker and one decode worker, one GPU each. Prefill passes
`--disaggregation-mode prefill` and decode passes `--disaggregation-mode decode`,
both with a NIXL `kv-transfer-config`.
The image tag in that file is the placeholder `my-tag`. Set a real runtime
image, and create `hf-token-secret`, before you apply it. The
[vLLM deploy README](https://github.com/ai-dynamo/dynamo/blob/main/examples/backends/vllm/deploy/README.md)
covers both.

```bash
export NAMESPACE=dynamo-system

kubectl apply -f examples/backends/vllm/deploy/disagg.yaml -n "$NAMESPACE"
kubectl wait --for=condition=Ready dynamographdeployment/vllm-disagg \
  -n "$NAMESPACE" --timeout=30m

kubectl port-forward -n "$NAMESPACE" svc/vllm-disagg-frontend 8000:8000 \
  >/tmp/dynamo-port-forward.log 2>&1 &
export PORT_FORWARD_PID=$!

until curl --silent --fail http://localhost:8000/health >/dev/null; do
  sleep 2
done
```

That manifest is not the old four-prefill, tensor-parallel decode layout.
Change replica counts and `--tensor-parallel-size` in the deployment spec, using
the disaggregated serving guide, when you want a different shape. SGLang and
TensorRT-LLM ship their own `deploy/disagg.yaml` files under
[`examples/backends`](https://github.com/ai-dynamo/dynamo/tree/main/examples/backends).

### Local

Run the launcher in one terminal. It stays in the foreground and stops the
workers when you interrupt it.

```bash
cd examples/backends/vllm
bash launch/disagg.sh
```

The script starts `python -m dynamo.frontend` on port 8000, a decode worker on
GPU 0, and a prefill worker on GPU 1. Each worker passes
`--disaggregation-mode` and `--kv-transfer-config`. The removed
`--is-prefill-worker` and `--is-decode-worker` flags are not accepted. The
script also passes `--enforce-eager` so the process starts quickly. Drop that
flag for a measurement you will compare across builds. See the script and the
[vLLM CLI template](../cli-templates/vllm.mdx#disaggregated).

From another terminal, wait until the frontend is ready:

```bash
until curl --silent --fail http://localhost:8000/health >/dev/null; do
  sleep 2
done
```

The same two-GPU pattern is
[`examples/backends/sglang/launch/disagg.sh`](https://github.com/ai-dynamo/dynamo/blob/main/examples/backends/sglang/launch/disagg.sh)
(`python3 -m dynamo.sglang`) and
[`examples/backends/trtllm/launch/disagg.sh`](https://github.com/ai-dynamo/dynamo/blob/main/examples/backends/trtllm/launch/disagg.sh)
(`python3 -m dynamo.trtllm`). The
[local disaggregated serving guide](../../cli/disaggregated-serving/overview.mdx)
lists all three.

## Deploy the Aggregated Comparison

Use a second artifact directory and the same model, input length, output
length, and concurrency list.

On Kubernetes, delete the disaggregated deployment, apply
[`agg.yaml`](https://github.com/ai-dynamo/dynamo/blob/main/examples/backends/vllm/deploy/agg.yaml)
(one GPU worker, service `vllm-agg-frontend`), and port-forward that service.
Locally, stop `disagg.sh` and run
[`launch/agg.sh`](https://github.com/ai-dynamo/dynamo/blob/main/examples/backends/vllm/launch/agg.sh).

Those two examples do not use the same GPU count: disaggregated serving in the
launcher takes two GPUs, and aggregated serving takes one. Read
throughput per GPU, or scale both layouts to the same GPU budget, before you
call the gap a serving-mode result.
`agg.sh` also defaults `MAX_CONCURRENT_SEQS` to 2 (`--max-num-seqs`). Raise
that variable before you sweep concurrency past 2, or the server rejects the
extra in-flight requests.

`benchmarks/llm/nginx.conf` and the two native `vllm serve` processes it
fronted are not in this tree. To measure a non-Dynamo OpenAI endpoint, pass
its URL to `--url`. To fan one client across several URLs, use AIPerf
[multi-URL load balancing](https://github.com/ai-dynamo/aiperf/blob/main/docs/tutorials/multi-url-load-balancing.md).

## Run the Concurrency Sweep

Keep the request shape fixed. Start with a short list. High concurrency on
these small launchers returns server errors. Raise the list only after a lower
point stays healthy.

```bash
MODEL="Qwen/Qwen3-0.6B"
URL="http://localhost:8000"
RUN="vllm-disagg"

for c in 1 2 5 10; do
    aiperf profile \
        --model "$MODEL" \
        --url "$URL" \
        --endpoint-type chat \
        --streaming \
        --concurrency "$c" \
        --request-count $(( c * 3 > 10 ? c * 3 : 10 )) \
        --synthetic-input-tokens-mean 2000 \
        --output-tokens-mean 256 \
        --extra-inputs max_tokens:256 \
        --extra-inputs min_tokens:256 \
        --extra-inputs ignore_eos:true \
        --artifact-dir "artifacts/${RUN}/c${c}"
done
```

Repeat the loop with `RUN="vllm-agg"` after the aggregated endpoint is ready.
`perf.sh` on the v1.2.1 tag defaulted to input length 3000, output length 150,
and concurrency `1,2,4,8,16,32,64,128,256`. This sweep uses the lengths from
the current benchmarking guide and a shorter list that fits the checked-in
Qwen3-0.6B launchers. Raise concurrency only while `/health` stays successful.
Add `--gpu-telemetry` to each `aiperf profile` when DCGM is running on the
GPUs. Without it, `aiperf plot` still draws Time To First Token (TTFT) against
throughput, and it skips the per-GPU Pareto curves.

Each run writes `profile_export_aiperf.json` under the directory you passed to
`--artifact-dir`. The old `perf.sh` layout (`artifacts_root/artifacts_N/-concurrencyN/`)
is not produced anymore.

For a sweep inside the cluster, use a recipe `perf.yaml` or
[`benchmarks/incluster/benchmark_job.yaml`](https://github.com/ai-dynamo/dynamo/blob/main/benchmarks/incluster/benchmark_job.yaml)
as described in the
[Dynamo Benchmarking guide](benchmarking-guide.md#server-side-benchmarking-in-cluster).
Port-forwarding adds client overhead. Use the in-cluster job when the number
you want is the server's.

## Read the Plot

```bash
aiperf plot artifacts/vllm-disagg artifacts/vllm-agg
```

`aiperf plot artifacts/ --dashboard` opens the interactive view. Separate
artifact directories are the series labels. `perf.sh --deployment-kind` is gone,
so name the directories after the configuration.

For a multi-run comparison, AIPerf writes:

- `ttft_vs_throughput.png` for every multi-run plot
- `pareto_curve_throughput_per_gpu_vs_latency.png` and
  `pareto_curve_throughput_per_gpu_vs_interactivity.png` when the runs include
  GPU telemetry

On the per-GPU curve, the horizontal axis is user throughput (tokens per second
per user, the inverse of per-user latency) and the vertical axis is average
tokens per second per GPU. A point is on the frontier when no other point is
both faster for the user and higher throughput per GPU.

Read a comparison at a fixed user throughput. If one series shows 145 tokens
per second per GPU and the other shows 80 at the same user throughput, the
absolute gap is 65 tokens per second per GPU and the ratio is 1.81x. Those
numbers are an illustration of the arithmetic, not a measured result. Record
TTFT, Inter-Token Latency (ITL), and output-token throughput from the AIPerf
table next to the plot.

The [AIPerf plotting guide](https://github.com/ai-dynamo/aiperf/blob/main/docs/tutorials/plot.md)
covers themes and experiment labels. An example frontier image is in the
[client-side section](benchmarking-guide.md#client-side-benchmarking-local) of the
Dynamo Benchmarking guide.

## Multi-Node Deployments

[`disagg-multinode.yaml`](https://github.com/ai-dynamo/dynamo/blob/main/examples/backends/vllm/deploy/disagg-multinode.yaml)
runs prefill and decode as separate workers, each with
`--tensor-parallel-size 2` and `nodeCount: 2`. It is not an eight-prefill,
tensor-parallel-8 decode layout. Apply it only after
[Multinode Orchestration](../../kubernetes/installation/multinode-orchestration.md)
and the
[RDMA setup](../../kubernetes/installation/rdma-setup/overview.md) for the
cluster. KV transfer that falls back from RDMA to TCP shows up as high TTFT.
The disaggregated serving guide describes that symptom under multi-node
transfer.

There is no checked-in local launcher for disaggregated serving across two
machines. [`launch/multi_node_tp.sh`](https://github.com/ai-dynamo/dynamo/blob/main/examples/backends/vllm/launch/multi_node_tp.sh)
is aggregated tensor parallelism (`--head` and `--worker`), not a prefill and
decode split. Workers that must reach another machine's discovery services use
`NATS_SERVER` and `ETCD_ENDPOINTS`. The
[local install guide](../../cli/installation/install-dynamo.mdx) starts those
services.

## Troubleshooting

1. **The client cannot connect.** Confirm `curl --fail http://localhost:8000/health`
   returns success. For Kubernetes, confirm the port-forward is still running
   and the service name ends in `-frontend`. For a local launch, confirm etcd
   and NATS are up (`docker compose -f dev/docker-compose.yml ps`).

2. **The sweep fails as concurrency rises.** Lower the concurrency list. On
   `launch/agg.sh`, raise `MAX_CONCURRENT_SEQS` so `--max-num-seqs` covers the
   sweep.

3. **The Pareto curve is missing.** Re-run `aiperf profile` with
   `--gpu-telemetry` while DCGM is available. TTFT-versus-throughput plots do
   not need that flag.

4. **Disaggregated throughput is worse than aggregated.** Check that NIXL
   selected the intended transport, then follow
   [Advanced Performance Tuning](../../kubernetes/operations/performance-tuning.md).
   On more than one node, start with the
   [RDMA setup](../../kubernetes/installation/rdma-setup/overview.md).

5. **Workers never become ready.** Read the worker logs for a missing
   `--kv-transfer-config` on a prefill worker, a placeholder image tag, or an
   absent `hf-token-secret`. Readiness is `/health`. See
   [Health Checks](../../reference/observability/health-checks.mdx).

## Related

- [Dynamo Benchmarking guide](benchmarking-guide.md) for in-cluster jobs and the AIPerf option list
- [Benchmark a Kubernetes deployment](../../kubernetes/operations/benchmarking-with-aiperf.mdx)
- [Benchmark a local deployment](../../cli/operations/benchmarking-with-aiperf.mdx)
- [Kubernetes observability](../../kubernetes/operations/observability.mdx) and the [metrics catalog](../../reference/observability/metrics-catalog.mdx)
- [Dynamo Recipes](https://github.com/ai-dynamo/dynamo/tree/main/recipes) when a model and GPU already have a `deploy.yaml` and `perf.yaml`
- Archived helpers on the v1.2.1 tag: [`perf.sh`](https://github.com/ai-dynamo/dynamo/blob/v1.2.1/benchmarks/llm/perf.sh), [`plot_pareto.py`](https://github.com/ai-dynamo/dynamo/blob/v1.2.1/benchmarks/llm/plot_pareto.py), and [`nginx.conf`](https://github.com/ai-dynamo/dynamo/blob/v1.2.1/benchmarks/llm/nginx.conf)
