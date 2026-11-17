# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Base imports stay free of AISimulate; missing the extra fails with install instructions."""

from __future__ import annotations

import os
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

from dynamo.common.optional_aisimulate import (
    AISIMULATE_EXTRA,
    INSTALL_HINT,
    import_aisimulate,
)

pytestmark = [
    pytest.mark.core,
    pytest.mark.gpu_0,
    pytest.mark.parallel,
    pytest.mark.pre_merge,
    pytest.mark.unit,
]

ROOT = Path(__file__).resolve().parents[2]
COMPONENTS = ROOT / "components" / "src"

# Modules that serving and planner startup import. None of them may import AISimulate.
_BASE_MODULES = (
    "dynamo.common.optional_aisimulate",
    "dynamo.common.configuration.groups.ais_perf_args",
    "dynamo.planner.config.planner_config",
    "dynamo.planner.core.perf_model.engine_query",
    "dynamo.planner.core.perf_model.ais_adapter",
    "dynamo.planner.core.perf_model",
    "dynamo.planner.core.state_machine",
)

_STUB_CORE = """
import sys
import types
from pathlib import Path

_core = types.ModuleType("dynamo._core")
_core.get_reasoning_parser_names = lambda: []
_core.get_tool_parser_names = lambda: []
sys.modules["dynamo._core"] = _core

_runtime = types.ModuleType("dynamo.runtime")
_runtime_logging = types.ModuleType("dynamo.runtime.logging")
_runtime_logging.configure_dynamo_logging = lambda *args, **kwargs: None
sys.modules["dynamo.runtime"] = _runtime
sys.modules["dynamo.runtime.logging"] = _runtime_logging

# groups/__init__.py imports the runtime arg group, which needs the Rust
# extension. Register the package so ais_perf_args can load on its own.
_groups_name = "dynamo.common.configuration.groups"
_groups = types.ModuleType(_groups_name)
_groups.__path__ = [
    str(Path("components/src/dynamo/common/configuration/groups").resolve())
]
_groups.__package__ = _groups_name
sys.modules[_groups_name] = _groups

# planner/__init__.py imports connectors. The scaling state itself does not.
_planner_name = "dynamo.planner"
_planner = types.ModuleType(_planner_name)
_planner.__path__ = [str(Path("components/src/dynamo/planner").resolve())]
_planner.__package__ = _planner_name
sys.modules[_planner_name] = _planner
"""

_BLOCK_AIS = """
import importlib.abc
import sys

class WithoutAIS(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if fullname.split(".")[0] in {"aisimulate", "aisimulate_core"}:
            raise ModuleNotFoundError("AIS runtime is unavailable", name=fullname)
        return None

sys.meta_path.insert(0, WithoutAIS())
"""


def _pythonpath() -> dict[str, str]:
    env = os.environ.copy()
    env["PYTHONPATH"] = str(COMPONENTS)
    return env


def _run(
    script: str, timeout: int = 60, *, stub_core: bool = False
) -> subprocess.CompletedProcess[str]:
    source = (
        (_STUB_CORE if stub_core else "") + _BLOCK_AIS + "\n" + textwrap.dedent(script)
    )
    return subprocess.run(
        [sys.executable, "-c", source],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=timeout,
        env=_pythonpath(),
        check=False,
    )


def test_install_hint_names_the_extra() -> None:
    assert AISIMULATE_EXTRA == "aisimulate"
    assert 'pip install "ai-dynamo[aisimulate]"' in INSTALL_HINT


def test_import_aisimulate_names_the_extra_when_blocked() -> None:
    result = _run(
        """
        from dynamo.common.optional_aisimulate import import_aisimulate

        try:
            import_aisimulate("aisimulate_core.sdk")
        except ModuleNotFoundError as error:
            assert (error.name or "").split(".")[0] == "aisimulate_core", error.name
            assert 'ai-dynamo[aisimulate]' in str(error), error
        else:
            raise SystemExit("expected ModuleNotFoundError")
        """
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_base_modules_do_not_import_aisimulate() -> None:
    modules = ", ".join(repr(name) for name in _BASE_MODULES)
    result = _run(
        f"""
        import importlib

        for name in ({modules}):
            importlib.import_module(name)
        loaded = [
            name
            for name in sys.modules
            if name == "aisimulate"
            or name.startswith("aisimulate.")
            or name == "aisimulate_core"
            or name.startswith("aisimulate_core.")
        ]
        assert loaded == [], loaded
        """,
        stub_core=True,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_ais_perf_config_requires_the_extra_with_a_clear_error() -> None:
    result = _run(
        """
        import argparse

        from dynamo.common.configuration.groups.ais_perf_args import (
            AisPerfArgGroup,
            AisPerfConfigBase,
        )

        parser = argparse.ArgumentParser()
        AisPerfArgGroup().add_arguments(parser)
        config = AisPerfConfigBase.from_cli_args(
            parser.parse_args(
                [
                    "--ais-backend",
                    "vllm",
                    "--ais-system",
                    "h200_sxm",
                    "--ais-model-path",
                    "example/model",
                ]
            )
        )
        try:
            config.ais_perf_kwargs()
        except ModuleNotFoundError as error:
            assert (error.name or "").split(".")[0] == "aisimulate_core", error
            assert 'pip install "ai-dynamo[aisimulate]"' in str(error), error
        else:
            raise SystemExit("expected ModuleNotFoundError")
        """,
        stub_core=True,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_import_aisimulate_rejects_unrelated_missing_modules(monkeypatch) -> None:
    def explode(name: str):
        raise ModuleNotFoundError("missing jax", name="jax")

    monkeypatch.setattr(
        "dynamo.common.optional_aisimulate.importlib.import_module", explode
    )
    with pytest.raises(ModuleNotFoundError, match="missing jax") as raised:
        import_aisimulate("aisimulate")
    assert raised.value.name == "jax"
    assert "ai-dynamo[aisimulate]" not in str(raised.value)
