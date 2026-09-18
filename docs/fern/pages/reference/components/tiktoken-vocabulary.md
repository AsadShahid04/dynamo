---
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
title: TikToken Vocabulary Files
subtitle: Pre-installed tiktoken vocabulary files for air-gapped deployments
---

## Overview

Dynamo runtime images include pre-installed tiktoken vocabulary files to support air-gapped deployments of gpt-oss models that require the harmony encoding for reasoning and tool call parsing.

## Installation Location

The tiktoken vocabulary files are installed at:

```
/opt/dynamo/tiktoken/harmony/
```

This location is exposed via the `TIKTOKEN_CACHE_DIR` environment variable, which is automatically set in all runtime images (vLLM, SGLang, TensorRT-LLM, and frontend).

## Included Files

The following tiktoken vocabulary files are pre-installed:

- `o200k_base.tiktoken` - The o200k_base encoding used by gpt-oss models
- `tiktoken.model` - Symlink to `o200k_base.tiktoken` for compatibility

## Usage

### Kubernetes Deployments

For gpt-oss deployments on Kubernetes, the tiktoken files are automatically available in the container. No additional init containers or ConfigMaps are required.

Example DynamoGraphDeployment snippet:

```yaml
spec:
  components:
    - name: worker
      type: worker
      podTemplate:
        spec:
          containers:
            - name: main
              image: nvcr.io/nvidia/ai-dynamo/vllm-runtime:1.3.0
              args:
                - --model=openai/gpt-oss-120b
                - --dyn-reasoning-parser=gpt_oss
                - --dyn-tool-call-parser=harmony
              # TIKTOKEN_CACHE_DIR is already set to /opt/dynamo/tiktoken
```

### CLI Deployments

For CLI deployments, the tiktoken files are also pre-installed and automatically discovered:

```bash
python -m dynamo.vllm \
  --model openai/gpt-oss-120b \
  --dyn-reasoning-parser gpt_oss \
  --dyn-tool-call-parser harmony
# TIKTOKEN_CACHE_DIR is already set to /opt/dynamo/tiktoken
```

## Air-Gapped Environments

In air-gapped environments where HuggingFace model downloads are unavailable, the pre-installed tiktoken files ensure that gpt-oss reasoning and tool parsing functionality works without requiring external network access during runtime.

The tiktoken vocabulary files are downloaded during the Docker image build process and are included in all published runtime images, eliminating the need for:

- Custom init containers to download vocabulary files
- ConfigMaps containing large vocabulary data
- Network access to OpenAI's tiktoken blob storage at runtime

## Custom Tiktoken Files

If you need to use custom tiktoken vocabulary files:

1. Override the `TIKTOKEN_CACHE_DIR` environment variable to point to your custom location
2. Mount a volume containing your custom tiktoken files
3. Ensure the files are readable by the `dynamo` user (UID 1000, GID 0)

Example with custom tiktoken files:

```yaml
spec:
  components:
    - name: worker
      podTemplate:
        spec:
          containers:
            - name: main
              env:
                - name: TIKTOKEN_CACHE_DIR
                  value: /custom-tiktoken
              volumeMounts:
                - name: custom-tiktoken
                  mountPath: /custom-tiktoken
          volumes:
            - name: custom-tiktoken
              configMap:
                name: my-custom-tiktoken-files
```

## See Also

- [Tool Call Parsing](/use-cases/tool-calling-and-reasoning/tool-call-parsing)
- [Reasoning Parsing](/use-cases/tool-calling-and-reasoning/reasoning-parsing)
- [gpt-oss-120b Recipe](/recipes/model-recipes/gpt-oss-120b)
