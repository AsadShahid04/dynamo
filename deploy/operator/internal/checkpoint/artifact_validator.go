/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

package checkpoint

import (
	"context"
	"fmt"
	"os"
	"path/filepath"

	corev1 "k8s.io/api/core/v1"
	"sigs.k8s.io/controller-runtime/pkg/log"
)

const (
	// CheckpointArtifactValidatorInitContainerName is the name of the init container
	// that validates checkpoint artifacts exist before restore.
	CheckpointArtifactValidatorInitContainerName = "checkpoint-artifact-validator"

	// DefaultCheckpointStorePath is the default mount path for the PVC-backed checkpoint store
	DefaultCheckpointStorePath = "/checkpoint-store"
)

// ArtifactValidationConfig holds configuration for checkpoint artifact validation
type ArtifactValidationConfig struct {
	// CheckpointStorePath is the path where checkpoint artifacts are stored
	CheckpointStorePath string
	// ValidatorImage is the container image to use for the validator init container
	ValidatorImage string
}

// EnsureArtifactValidatorInitContainer adds or updates the checkpoint artifact validator
// init container in the pod spec to verify the checkpoint artifact exists before restore.
func EnsureArtifactValidatorInitContainer(
	pod *corev1.PodTemplateSpec,
	checkpointName string,
	checkpointStorePath string,
	validatorImage string,
) error {
	if pod == nil {
		return fmt.Errorf("pod template is nil")
	}
	if checkpointName == "" {
		return fmt.Errorf("checkpoint name is empty")
	}
	if checkpointStorePath == "" {
		checkpointStorePath = DefaultCheckpointStorePath
	}
	if validatorImage == "" {
		return fmt.Errorf("validator image is empty")
	}

	// Create the validation script that checks for the checkpoint artifact
	validationScript := fmt.Sprintf(`#!/bin/sh
set -e

CHECKPOINT_NAME="%s"
CHECKPOINT_STORE="%s"
ARTIFACT_PATH="${CHECKPOINT_STORE}/${CHECKPOINT_NAME}"

echo "Validating checkpoint artifact existence for: ${CHECKPOINT_NAME}"
echo "Checking path: ${ARTIFACT_PATH}"

if [ ! -e "${ARTIFACT_PATH}" ]; then
    echo "ERROR: Checkpoint artifact not found at ${ARTIFACT_PATH}" >&2
    echo "ERROR: The PodSnapshot ${CHECKPOINT_NAME} shows Ready=true but its artifact is missing from disk." >&2
    echo "ERROR: This can happen if the checkpoint was deleted manually or by an agent restart." >&2
    echo "ERROR: Cannot proceed with restore. Failing loudly to prevent indefinite hang." >&2
    exit 1
fi

if [ ! -r "${ARTIFACT_PATH}" ]; then
    echo "ERROR: Checkpoint artifact at ${ARTIFACT_PATH} is not readable" >&2
    exit 1
fi

echo "SUCCESS: Checkpoint artifact verified at ${ARTIFACT_PATH}"
exit 0
`, checkpointName, checkpointStorePath)

	// Find or create the validator init container
	var validator *corev1.Container
	for i := range pod.Spec.InitContainers {
		if pod.Spec.InitContainers[i].Name == CheckpointArtifactValidatorInitContainerName {
			validator = &pod.Spec.InitContainers[i]
			break
		}
	}

	if validator == nil {
		pod.Spec.InitContainers = append([]corev1.Container{
			{
				Name:  CheckpointArtifactValidatorInitContainerName,
				Image: validatorImage,
			},
		}, pod.Spec.InitContainers...)
		validator = &pod.Spec.InitContainers[0]
	}

	// Configure the validator container
	validator.Image = validatorImage
	validator.Command = []string{"/bin/sh", "-c"}
	validator.Args = []string{validationScript}
	validator.RestartPolicy = nil // Init containers don't support restartPolicy

	// Ensure the checkpoint store volume is mounted
	hasVolumeMount := false
	for _, vm := range validator.VolumeMounts {
		if vm.MountPath == checkpointStorePath {
			hasVolumeMount = true
			break
		}
	}
	if !hasVolumeMount {
		validator.VolumeMounts = append(validator.VolumeMounts, corev1.VolumeMount{
			Name:      "checkpoint-store",
			MountPath: checkpointStorePath,
			ReadOnly:  true,
		})
	}

	return nil
}

// ValidateArtifactExists checks if a checkpoint artifact exists at the given path.
// This function is intended for use in test environments or direct validation scenarios.
func ValidateArtifactExists(ctx context.Context, checkpointStorePath, checkpointName string) error {
	logger := log.FromContext(ctx)

	if checkpointStorePath == "" {
		checkpointStorePath = DefaultCheckpointStorePath
	}

	artifactPath := filepath.Join(checkpointStorePath, checkpointName)
	logger.Info("Validating checkpoint artifact", "path", artifactPath)

	info, err := os.Stat(artifactPath)
	if os.IsNotExist(err) {
		return fmt.Errorf("checkpoint artifact not found at %s: PodSnapshot may show Ready=true but artifact is missing from disk", artifactPath)
	}
	if err != nil {
		return fmt.Errorf("failed to stat checkpoint artifact at %s: %w", artifactPath, err)
	}

	if info.IsDir() {
		return fmt.Errorf("checkpoint artifact at %s is a directory, expected a file", artifactPath)
	}

	logger.Info("Checkpoint artifact validated successfully", "path", artifactPath, "size", info.Size())
	return nil
}
