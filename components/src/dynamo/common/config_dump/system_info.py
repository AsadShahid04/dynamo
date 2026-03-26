# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import importlib.metadata
import logging
import platform
import sys
from typing import Any, Dict, Optional, Tuple

logger = logging.getLogger(__name__)


def get_system_info() -> Dict[str, Any]:
    """
    Get comprehensive system information.

    Returns:
        Dictionary containing platform, architecture, processor, hostname,
        and operating system details.

    Note:
        Gracefully handles errors by returning partial information.
    """
    info: Dict[str, Any] = {}

    try:
        info["platform"] = platform.platform()
    except Exception as e:
        logger.warning(f"Failed to get platform: {e}")
        info["platform"] = "unknown"

    try:
        info["architecture"] = platform.architecture()
    except Exception as e:
        logger.warning(f"Failed to get architecture: {e}")
        info["architecture"] = ("unknown", "unknown")

    try:
        info["processor"] = platform.processor() or "unknown"
    except Exception as e:
        logger.warning(f"Failed to get processor: {e}")
        info["processor"] = "unknown"

    try:
        info["hostname"] = platform.node()
    except Exception as e:
        logger.warning(f"Failed to get hostname: {e}")
        info["hostname"] = "unknown"

    try:
        info["os_name"] = platform.system()
        info["os_release"] = platform.release()
        info["os_version"] = platform.version()
    except Exception as e:
        logger.warning(f"Failed to get OS details: {e}")

    return info


def get_runtime_info() -> Dict[str, Any]:
    """
    Get Python runtime information.

    Returns:
        Dictionary containing Python version, executable path, and command-line arguments.

    Note:
        Gracefully handles errors by returning partial information.
    """
    info: Dict[str, Any] = {}

    try:
        info["python_version"] = sys.version
        info["python_version_info"] = {
            "major": sys.version_info.major,
            "minor": sys.version_info.minor,
            "micro": sys.version_info.micro,
        }
    except Exception as e:
        logger.warning(f"Failed to get Python version: {e}")
        info["python_version"] = "unknown"

    try:
        info["python_executable"] = sys.executable
    except Exception as e:
        logger.warning(f"Failed to get Python executable: {e}")
        info["python_executable"] = "unknown"

    try:
        info["command_line_args"] = sys.argv
    except Exception as e:
        logger.warning(f"Failed to get command-line args: {e}")
        info["command_line_args"] = []

    return info


def get_gpu_info() -> Optional[Dict[str, Any]]:
    """
    Get GPU information if available.

    Returns:
        Dictionary containing GPU details if available, None otherwise.
        Attempts to use nvidia-smi via subprocess with XML output format.

    Note:
        This is a best-effort function and returns None if GPU info cannot be obtained.
    """
    try:
        import subprocess
        import xml.etree.ElementTree as ET

        result = subprocess.run(
            [
                "nvidia-smi",
                "-q",
                "-x",
            ],
            capture_output=True,
            text=True,
            timeout=5,
        )
        if result.returncode == 0:
            root = ET.fromstring(result.stdout)

            # Get driver version from root level
            driver_version_elem = root.find("driver_version")
            driver_version = (
                driver_version_elem.text
                if driver_version_elem is not None
                else "unknown"
            )

            gpus = []

            # Parse each GPU element
            for gpu_elem in root.findall("gpu"):
                gpu_info = {}

                # Extract product name
                product_name = gpu_elem.find("product_name")
                if product_name is not None:
                    gpu_info["name"] = product_name.text

                # Extract driver version
                gpu_info["driver_version"] = driver_version

                # Extract memory total
                fb_memory = gpu_elem.find("fb_memory_usage/total")
                if fb_memory is not None:
                    gpu_info["memory_total"] = fb_memory.text

                # Extract board part number
                board_part = gpu_elem.find("board_part_number")
                if board_part is not None:
                    gpu_info["board_part_number"] = board_part.text

                # Extract GPU part number
                gpu_part = gpu_elem.find("gpu_part_number")
                if gpu_part is not None:
                    gpu_info["gpu_part_number"] = gpu_part.text

                if gpu_info:
                    gpus.append(gpu_info)

            return {"gpus": gpus, "count": len(gpus)} if gpus else None
    except (
        FileNotFoundError,
        subprocess.TimeoutExpired,
        ET.ParseError,
        Exception,
    ) as e:
        logger.debug(f"Failed to get GPU info: {e}")

    return None


# Minimum CUDA compute capability required for full Dynamo feature support.
# Ampere (sm_80) introduced async copy, fine-grained sparse maths, and the
# memory pool APIs that Dynamo relies on.  Older architectures (Volta, Turing)
# may still work for basic inference but are not officially supported.
MINIMUM_COMPUTE_CAPABILITY: Tuple[int, int] = (8, 0)


def validate_cuda_compute_capability(
    minimum: Tuple[int, int] = MINIMUM_COMPUTE_CAPABILITY,
) -> None:
    """Log a warning for each GPU whose compute capability is below *minimum*.

    This is a best-effort helper; all exceptions are caught so that it never
    prevents a worker from starting.

    Args:
        minimum: (major, minor) compute capability floor. Defaults to
            ``MINIMUM_COMPUTE_CAPABILITY`` (Ampere, sm_80).
    """
    try:
        import torch

        if not torch.cuda.is_available():
            return

        device_count = torch.cuda.device_count()
        for device_idx in range(device_count):
            try:
                cap = torch.cuda.get_device_capability(device_idx)
                if cap < minimum:
                    device_name = torch.cuda.get_device_name(device_idx)
                    logger.warning(
                        "GPU %d (%s) has compute capability %d.%d which is below "
                        "the minimum supported capability %d.%d. "
                        "Some Dynamo features may not work correctly.",
                        device_idx,
                        device_name,
                        cap[0],
                        cap[1],
                        minimum[0],
                        minimum[1],
                    )
            except Exception as inner_exc:
                logger.debug(
                    "Could not query compute capability for device %d: %s",
                    device_idx,
                    inner_exc,
                )
    except Exception as exc:
        logger.debug("validate_cuda_compute_capability skipped: %s", exc)


def get_package_info() -> Optional[Dict[str, Any]]:
    """
    Get package information.

    Returns:
        Dictionary containing installed packages and their versions.
    """

    packages = {}
    for package in importlib.metadata.distributions():
        packages[package.name] = package.version

    return packages
