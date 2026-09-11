# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Test that mocker model card is gated on startup completion (issue #14686).
"""

import asyncio
import pytest
import time
from dynamo.llm import make_engine, run_input, EntrypointArgs, ModelRuntimeConfig
from dynamo.mocker import MockEngineArgs, WorkerType


@pytest.mark.asyncio
async def test_mocker_model_card_waits_for_startup():
    """
    Verify that model card is NOT published until startup completes.

    Tests the fix for https://github.com/ai-dynamo/dynamo/issues/14686
    where mocker would list models in /v1/models before --startup-time completed.
    """
    from dynamo.common.utils.runtime import create_runtime

    startup_time_secs = 2.0

    # Create runtime and mocker with simulated startup
    runtime, loop = create_runtime(
        discovery_backend="mem",
        request_plane="tcp",
        event_plane="zmq"
    )

    engine_args = MockEngineArgs(
        engine_type="vllm",
        num_gpu_blocks=64,
        block_size=16,
        max_model_len=1024,
        startup_time=startup_time_secs,
        worker_type=WorkerType.Decode,
    )

    runtime_config = ModelRuntimeConfig()
    runtime_config.total_kv_blocks = 64

    entrypoint_args = EntrypointArgs(
        engine_type="mocker",
        model_name="test-model",
        model_path=None,
        endpoint_id="dyn://test.backend.generate",
        mocker_engine_args=engine_args,
        runtime_config=runtime_config,
        kv_cache_block_size=16,
        is_prefill=False,
        is_decode=True,
    )

    # Create the engine (this returns immediately but startup runs async)
    engine_config = await make_engine(runtime, entrypoint_args)

    # Immediately check: model should NOT be ready yet
    # (This is where the bug was - model would appear immediately)
    start = time.time()

    # Wait a bit to let any premature registration happen
    await asyncio.sleep(0.5)

    # Check model readiness through the runtime's model manager
    # The model should NOT be listed as ready during startup
    from dynamo.llm import get_model_manager
    manager = get_model_manager()

    # During startup window, model should either not exist or not be ready
    model = manager.get_committed_model("test-model")
    if model is not None:
        # If model exists, verify it's not ready to serve yet
        assert not model.is_ready_to_serve(), \
            f"Model should not be ready during startup (elapsed: {time.time() - start:.1f}s < {startup_time_secs}s)"

    # Wait for startup to complete
    await asyncio.sleep(startup_time_secs - 0.5 + 0.5)  # Total: startup_time_secs + 0.5

    # Now model should be ready
    model = manager.get_committed_model("test-model")
    assert model is not None, "Model should exist after startup"
    assert model.is_ready_to_serve(), \
        f"Model should be ready after startup completes (elapsed: {time.time() - start:.1f}s >= {startup_time_secs}s)"

    # Cleanup
    runtime.shutdown()


@pytest.mark.asyncio
async def test_mocker_disagg_decode_with_startup_delay():
    """
    Test disagg decode worker with startup delay doesn't advertise model prematurely.

    Specific test for the disaggregated serving scenario mentioned in issue #14686.
    """
    from dynamo.common.utils.runtime import create_runtime

    startup_time_secs = 5.0

    runtime, loop = create_runtime(
        discovery_backend="mem",
        request_plane="tcp",
        event_plane="zmq"
    )

    # Decode worker configuration matching the issue scenario
    decode_args = MockEngineArgs(
        engine_type="vllm",
        num_gpu_blocks=256,
        block_size=16,
        max_model_len=2048,
        startup_time=startup_time_secs,
        worker_type=WorkerType.Decode,
    )

    runtime_config = ModelRuntimeConfig()
    runtime_config.total_kv_blocks = 256

    entrypoint_args = EntrypointArgs(
        engine_type="mocker",
        model_name="disagg-test-model",
        model_path=None,
        endpoint_id="dyn://test.backend.generate",
        mocker_engine_args=decode_args,
        runtime_config=runtime_config,
        kv_cache_block_size=16,
        is_prefill=False,
        is_decode=True,
    )

    engine_config = await make_engine(runtime, entrypoint_args)

    # Record start time
    start = time.time()

    # Immediately after engine creation, model should not be ready
    await asyncio.sleep(0.2)

    from dynamo.llm import get_model_manager
    manager = get_model_manager()

    # Verify model is not ready during startup
    elapsed = time.time() - start
    if elapsed < startup_time_secs:
        model = manager.get_committed_model("disagg-test-model")
        if model is not None:
            assert not model.is_ready_to_serve(), \
                f"Disagg decode model should not be ready during startup (elapsed: {elapsed:.1f}s)"

    # Wait for full startup
    remaining = startup_time_secs - elapsed + 0.5
    if remaining > 0:
        await asyncio.sleep(remaining)

    # Verify model is ready after startup
    model = manager.get_committed_model("disagg-test-model")
    assert model is not None, "Disagg decode model should exist"
    assert model.is_ready_to_serve(), "Disagg decode model should be ready after startup"

    # Cleanup
    runtime.shutdown()
