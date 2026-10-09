/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

package configmapcache

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/cache"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/ai-dynamo/dynamo/deploy/operator/internal/consts"
)

func TestProjectStripsPayloadsFromAllConfigMaps(t *testing.T) {
	largeData := map[string]string{
		"key1": string(make([]byte, 1024*1024)), // 1 MiB
		"key2": "smaller value",
	}
	largeBinary := map[string][]byte{
		"binary": make([]byte, 512*1024), // 512 KiB
	}

	t.Run("unrelated ConfigMap has payload stripped", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{
				Name:            "unrelated",
				Namespace:       "default",
				UID:             types.UID("unrelated-uid"),
				ResourceVersion: "1",
				Labels:          map[string]string{"app": "something-else"},
				Annotations:     map[string]string{"note": "preserve"},
				ManagedFields:   []metav1.ManagedFieldsEntry{{Manager: "kubectl"}},
			},
			Data:       largeData,
			BinaryData: largeBinary,
		}

		got := Project(cm)

		assert.Equal(t, "unrelated", got.Name)
		assert.Equal(t, "default", got.Namespace)
		assert.Equal(t, types.UID("unrelated-uid"), got.UID)
		assert.Equal(t, map[string]string{"app": "something-else"}, got.Labels)
		assert.Equal(t, map[string]string{"note": "preserve"}, got.Annotations)
		assert.Nil(t, got.ManagedFields, "ManagedFields should be stripped")
		assert.Nil(t, got.Data, "Data should be stripped")
		assert.Nil(t, got.BinaryData, "BinaryData should be stripped")
	})

	t.Run("DGD-labeled ConfigMap has payload stripped", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{
				Name:      "dgd-config",
				Namespace: "inference",
				Labels: map[string]string{
					consts.KubeLabelDynamoGraphDeploymentName: "my-dgd",
				},
			},
			Data:       largeData,
			BinaryData: largeBinary,
		}

		got := Project(cm)

		assert.Equal(t, "dgd-config", got.Name)
		assert.Nil(t, got.Data, "DGD ConfigMap Data should be stripped from cache")
		assert.Nil(t, got.BinaryData, "DGD ConfigMap BinaryData should be stripped from cache")
	})

	t.Run("LPX-labeled ConfigMap has payload stripped", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{
				Name:      "lpx-runtime",
				Namespace: "inference",
				Labels: map[string]string{
					"lpx.nvidia.com/deployment-uid": "lpx-deployment-uid",
				},
			},
			Data: largeData,
		}

		got := Project(cm)

		assert.Equal(t, "lpx-runtime", got.Name)
		assert.Nil(t, got.Data, "LPX ConfigMap Data should be stripped from cache")
	})

	t.Run("DGDR output ConfigMap has payload stripped", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{
				Name:      "dgdr-output-my-request",
				Namespace: "inference",
			},
			Data: largeData,
		}

		got := Project(cm)

		assert.Equal(t, "dgdr-output-my-request", got.Name)
		assert.Nil(t, got.Data, "DGDR output ConfigMap Data should be stripped from cache")
	})

	t.Run("projection is idempotent", func(t *testing.T) {
		cm := &corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{
				Name:      "test",
				Namespace: "default",
			},
			Data: largeData,
		}

		first := Project(cm)
		second := Project(first)

		assert.Equal(t, first, second, "projection must be idempotent")
	})
}

func TestTransformLeavesOtherObjectsUnchanged(t *testing.T) {
	pod := &corev1.Pod{ObjectMeta: metav1.ObjectMeta{Name: "pod"}}
	got, err := Transform(pod)
	require.NoError(t, err)
	assert.Same(t, pod, got)
}

func TestConfigureAddsConfigMapEntry(t *testing.T) {
	options := cache.Options{}
	require.NoError(t, Configure(&options))

	configMapEntries := 0
	for obj, byObject := range options.ByObject {
		if _, ok := obj.(*corev1.ConfigMap); !ok {
			continue
		}
		configMapEntries++
		require.NotNil(t, byObject.Transform)
	}
	assert.Equal(t, 1, configMapEntries)
}

func TestConfigure(t *testing.T) {
	configMapKey := &corev1.ConfigMap{}
	podKey := &corev1.Pod{}
	namespaces := map[string]cache.Config{"inference": {}}
	options := cache.Options{
		DefaultNamespaces: namespaces,
		ByObject: map[client.Object]cache.ByObject{
			configMapKey: {Namespaces: namespaces},
			podKey:       {},
		},
	}

	require.NoError(t, Configure(&options))
	require.NoError(t, Configure(&options), "configuration must be idempotent")

	configMapEntries := 0
	for obj, byObject := range options.ByObject {
		if _, ok := obj.(*corev1.ConfigMap); !ok {
			continue
		}
		configMapEntries++
		assert.Equal(t, namespaces, byObject.Namespaces)
		require.NotNil(t, byObject.Transform)
	}
	assert.Equal(t, 1, configMapEntries)
	_, podStillPresent := options.ByObject[podKey]
	assert.True(t, podStillPresent)
}

func TestConfigureRejectsDuplicateConfigMapEntries(t *testing.T) {
	options := cache.Options{ByObject: map[client.Object]cache.ByObject{
		&corev1.ConfigMap{}: {},
		&corev1.ConfigMap{}: {},
	}}
	assert.ErrorContains(t, Configure(&options), "multiple ConfigMap cache configurations")
}
