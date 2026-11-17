# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Optional AISimulate dependency (``pip install "ai-dynamo[aisimulate]"``)."""

from __future__ import annotations

import importlib
from types import ModuleType
from typing import NoReturn

AISIMULATE_EXTRA = "aisimulate"
INSTALL_HINT = (
    "AISimulate is not installed. Simulation, replay, AISimulate-backed "
    "profiling, and Planner performance models require the optional extra: "
    'pip install "ai-dynamo[aisimulate]"'
)
_ROOTS = frozenset({"aisimulate", "aisimulate_core"})


def missing_aisimulate(exc: ModuleNotFoundError) -> bool:
    """True when ``exc`` is a missing AISimulate distribution, not a nested import."""
    root = (exc.name or "").split(".", 1)[0]
    return root in _ROOTS


def reraise_missing_aisimulate(exc: ModuleNotFoundError) -> NoReturn:
    """Re-raise ``exc`` with install instructions when AISimulate itself is absent."""
    if not missing_aisimulate(exc):
        raise exc
    raise ModuleNotFoundError(
        f"{INSTALL_HINT} (missing {exc.name})",
        name=exc.name,
    ) from exc


def import_aisimulate(module: str) -> ModuleType:
    """Import ``module`` or raise ``ModuleNotFoundError`` naming the extra."""
    try:
        return importlib.import_module(module)
    except ModuleNotFoundError as exc:
        reraise_missing_aisimulate(exc)
