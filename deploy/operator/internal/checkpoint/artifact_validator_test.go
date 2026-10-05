/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

package checkpoint

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

func TestEnsureArtifactValidatorInitContainer(t *testing.T) {
	tests := []struct {
		name                string
		pod                 *corev1.PodTemplateSpec
		checkpointName      string
		checkpointStorePath string
		validatorImage      string
		wantErr             bool
		validateFunc        func(t *testing.T, pod *corev1.PodTemplateSpec)
	}{
		{
			name: "adds validator init container to empty pod",
			pod: &corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{},
			},
			checkpointName:      "test-checkpoint",
			checkpointStorePath: "/checkpoint-store",
			validatorImage:      "busybox:latest",
			wantErr:             false,
			validateFunc: func(t *testing.T, pod *corev1.PodTemplateSpec) {
				require.Len(t, pod.Spec.InitContainers, 1)
				validator := pod.Spec.InitContainers[0]
				assert.Equal(t, CheckpointArtifactValidatorInitContainerName, validator.Name)
				assert.Equal(t, "busybox:latest", validator.Image)
				assert.Equal(t, []string{"/bin/sh", "-c"}, validator.Command)
				require.Len(t, validator.Args, 1)
				assert.Contains(t, validator.Args[0], "test-checkpoint")
				assert.Contains(t, validator.Args[0], "Checkpoint artifact not found")
				assert.Len(t, validator.VolumeMounts, 1)
				assert.Equal(t, "checkpoint-store", validator.VolumeMounts[0].Name)
				assert.Equal(t, "/checkpoint-store", validator.VolumeMounts[0].MountPath)
				assert.True(t, validator.VolumeMounts[0].ReadOnly)
			},
		},
		{
			name: "updates existing validator init container",
			pod: &corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{
					InitContainers: []corev1.Container{
						{
							Name:  CheckpointArtifactValidatorInitContainerName,
							Image: "old-image:v1",
						},
					},
				},
			},
			checkpointName:      "new-checkpoint",
			checkpointStorePath: "/custom-path",
			validatorImage:      "new-image:v2",
			wantErr:             false,
			validateFunc: func(t *testing.T, pod *corev1.PodTemplateSpec) {
				require.Len(t, pod.Spec.InitContainers, 1)
				validator := pod.Spec.InitContainers[0]
				assert.Equal(t, "new-image:v2", validator.Image)
				assert.Contains(t, validator.Args[0], "new-checkpoint")
				assert.Contains(t, validator.Args[0], "/custom-path")
			},
		},
		{
			name: "preserves other init containers",
			pod: &corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{
					InitContainers: []corev1.Container{
						{
							Name:  "other-init",
							Image: "other:v1",
						},
					},
				},
			},
			checkpointName:      "test-checkpoint",
			checkpointStorePath: "/checkpoint-store",
			validatorImage:      "busybox:latest",
			wantErr:             false,
			validateFunc: func(t *testing.T, pod *corev1.PodTemplateSpec) {
				require.Len(t, pod.Spec.InitContainers, 2)
				assert.Equal(t, CheckpointArtifactValidatorInitContainerName, pod.Spec.InitContainers[0].Name)
				assert.Equal(t, "other-init", pod.Spec.InitContainers[1].Name)
			},
		},
		{
			name:                "returns error for nil pod",
			pod:                 nil,
			checkpointName:      "test-checkpoint",
			checkpointStorePath: "/checkpoint-store",
			validatorImage:      "busybox:latest",
			wantErr:             true,
		},
		{
			name: "returns error for empty checkpoint name",
			pod: &corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{},
			},
			checkpointName:      "",
			checkpointStorePath: "/checkpoint-store",
			validatorImage:      "busybox:latest",
			wantErr:             true,
		},
		{
			name: "returns error for empty validator image",
			pod: &corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{},
			},
			checkpointName:      "test-checkpoint",
			checkpointStorePath: "/checkpoint-store",
			validatorImage:      "",
			wantErr:             true,
		},
		{
			name: "uses default path when empty",
			pod: &corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{},
			},
			checkpointName:      "test-checkpoint",
			checkpointStorePath: "",
			validatorImage:      "busybox:latest",
			wantErr:             false,
			validateFunc: func(t *testing.T, pod *corev1.PodTemplateSpec) {
				require.Len(t, pod.Spec.InitContainers, 1)
				validator := pod.Spec.InitContainers[0]
				assert.Contains(t, validator.Args[0], DefaultCheckpointStorePath)
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := EnsureArtifactValidatorInitContainer(
				tt.pod,
				tt.checkpointName,
				tt.checkpointStorePath,
				tt.validatorImage,
			)

			if tt.wantErr {
				assert.Error(t, err)
				return
			}

			require.NoError(t, err)
			if tt.validateFunc != nil {
				tt.validateFunc(t, tt.pod)
			}
		})
	}
}

func TestValidateArtifactExists(t *testing.T) {
	ctx := context.Background()

	t.Run("succeeds when artifact exists", func(t *testing.T) {
		// Create a temporary directory and file
		tmpDir := t.TempDir()
		checkpointName := "test-checkpoint"
		artifactPath := filepath.Join(tmpDir, checkpointName)
		err := os.WriteFile(artifactPath, []byte("checkpoint data"), 0644)
		require.NoError(t, err)

		// Validate
		err = ValidateArtifactExists(ctx, tmpDir, checkpointName)
		assert.NoError(t, err)
	})

	t.Run("fails when artifact does not exist", func(t *testing.T) {
		tmpDir := t.TempDir()
		checkpointName := "missing-checkpoint"

		err := ValidateArtifactExists(ctx, tmpDir, checkpointName)
		require.Error(t, err)
		assert.Contains(t, err.Error(), "checkpoint artifact not found")
		assert.Contains(t, err.Error(), "PodSnapshot may show Ready=true but artifact is missing from disk")
	})

	t.Run("fails when artifact is a directory", func(t *testing.T) {
		tmpDir := t.TempDir()
		checkpointName := "checkpoint-dir"
		artifactPath := filepath.Join(tmpDir, checkpointName)
		err := os.Mkdir(artifactPath, 0755)
		require.NoError(t, err)

		err = ValidateArtifactExists(ctx, tmpDir, checkpointName)
		require.Error(t, err)
		assert.Contains(t, err.Error(), "is a directory")
	})

	t.Run("uses default path when empty", func(t *testing.T) {
		checkpointName := "test-checkpoint"

		// This will fail because DefaultCheckpointStorePath likely doesn't exist
		err := ValidateArtifactExists(ctx, "", checkpointName)
		assert.Error(t, err)
	})
}

func TestArtifactValidatorInitContainerContent(t *testing.T) {
	t.Run("generated script validates checkpoint existence", func(t *testing.T) {
		pod := &corev1.PodTemplateSpec{
			ObjectMeta: metav1.ObjectMeta{
				Name: "test-pod",
			},
			Spec: corev1.PodSpec{},
		}

		checkpointName := "my-checkpoint"
		err := EnsureArtifactValidatorInitContainer(
			pod,
			checkpointName,
			"/checkpoint-store",
			"busybox:latest",
		)
		require.NoError(t, err)

		validator := pod.Spec.InitContainers[0]
		script := validator.Args[0]

		// Verify the script contains expected content
		assert.Contains(t, script, "CHECKPOINT_NAME=\"my-checkpoint\"")
		assert.Contains(t, script, "CHECKPOINT_STORE=\"/checkpoint-store\"")
		assert.Contains(t, script, "if [ ! -e \"${ARTIFACT_PATH}\" ]")
		assert.Contains(t, script, "ERROR: Checkpoint artifact not found")
		assert.Contains(t, script, "PodSnapshot")
		assert.Contains(t, script, "Ready=true but its artifact is missing from disk")
		assert.Contains(t, script, "exit 1")
		assert.Contains(t, script, "if [ ! -r \"${ARTIFACT_PATH}\" ]")
		assert.Contains(t, script, "SUCCESS: Checkpoint artifact verified")
	})
}
