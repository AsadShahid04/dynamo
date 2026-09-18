#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# Downloads harmony/gpt-oss tiktoken vocabulary files for air-gapped deployments.
# These files are required for gpt-oss reasoning/tool parsing when HuggingFace
# model downloads are unavailable (air-gapped environments).

set -euo pipefail

TIKTOKEN_DIR="${1:-/opt/dynamo/tiktoken/harmony}"
TIKTOKEN_BASE_URL="https://openaipublic.blob.core.windows.net/encodings"

echo "Downloading harmony tiktoken files to ${TIKTOKEN_DIR}..."
mkdir -p "${TIKTOKEN_DIR}"

# Download o200k_base encoding (used by gpt-oss models)
curl -fsSL "${TIKTOKEN_BASE_URL}/o200k_base.tiktoken" -o "${TIKTOKEN_DIR}/o200k_base.tiktoken"

# Verify download succeeded
if [ ! -f "${TIKTOKEN_DIR}/o200k_base.tiktoken" ]; then
    echo "ERROR: Failed to download o200k_base.tiktoken" >&2
    exit 1
fi

# Create a symlink for convenience (tiktoken.model is a common lookup name)
ln -sf o200k_base.tiktoken "${TIKTOKEN_DIR}/tiktoken.model"

echo "Successfully downloaded harmony tiktoken files:"
ls -lh "${TIKTOKEN_DIR}"
