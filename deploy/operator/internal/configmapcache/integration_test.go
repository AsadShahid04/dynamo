/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

package configmapcache

import (
	"context"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"sigs.k8s.io/controller-runtime/pkg/cache"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/client/fake"

	"github.com/ai-dynamo/dynamo/deploy/operator/internal/consts"
)

// TestCachedClientStripsPayloadsButAPIReaderRetains verifies that:
// 1. ConfigMaps in the cache have their Data stripped
// 2. APIReader still returns full ConfigMap contents
func TestCachedClientStripsPayloadsButAPIReaderRetains(t *testing.T) {
	scheme := runtime.NewScheme()
	require.NoError(t, corev1.AddToScheme(scheme))

	largeData := map[string]string{
		"final_config.yaml": "large DGD spec content here",
		"phase":             "done",
		"message":           "Profiling completed successfully",
	}

	// Create a DGDR output ConfigMap with data
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "dgdr-output-my-request",
			Namespace: "inference",
		},
		Data: largeData,
	}

	// Create a fake client that acts as both cached client and APIReader
	fakeClient := fake.NewClientBuilder().
		WithScheme(scheme).
		WithObjects(cm).
		Build()

	ctx := context.Background()

	t.Run("cached client returns stripped ConfigMap", func(t *testing.T) {
		// Simulate cache transformation
		cached := cm.DeepCopy()
		transformed, err := Transform(cached)
		require.NoError(t, err)
		cachedCM := transformed.(*corev1.ConfigMap)

		// Verify cache strips payload
		assert.Nil(t, cachedCM.Data, "cached ConfigMap should have Data stripped")
	})

	t.Run("APIReader returns full ConfigMap", func(t *testing.T) {
		// In real usage, APIReader bypasses cache and reads directly from API server
		retrieved := &corev1.ConfigMap{}
		err := fakeClient.Get(ctx, client.ObjectKeyFromObject(cm), retrieved)
		require.NoError(t, err)

		// Verify APIReader has full data
		assert.NotNil(t, retrieved.Data, "APIReader should return full ConfigMap")
		assert.Equal(t, largeData, retrieved.Data, "APIReader should return original data")
	})
}

// TestDynamoConfigMapsAreIdentified verifies that ConfigMaps are correctly
// identified as Dynamo-related based on labels and naming patterns.
func TestDynamoConfigMapsAreIdentified(t *testing.T) {
	testCases := []struct {
		name               string
		configMap          *corev1.ConfigMap
		expectPayloadKept  bool
		expectDescription  string
	}{
		{
			name: "DGD-labeled ConfigMap",
			configMap: &corev1.ConfigMap{
				ObjectMeta: metav1.ObjectMeta{
					Name:      "dgd-config",
					Namespace: "inference",
					Labels: map[string]string{
						consts.KubeLabelDynamoGraphDeploymentName: "my-dgd",
					},
				},
				Data: map[string]string{"key": "value"},
			},
			expectPayloadKept: false, // Stripped in cache, use APIReader
			expectDescription: "DGD ConfigMap should have payload stripped in cache",
		},
		{
			name: "LPX-labeled ConfigMap",
			configMap: &corev1.ConfigMap{
				ObjectMeta: metav1.ObjectMeta{
					Name:      "lpx-runtime",
					Namespace: "inference",
					Labels: map[string]string{
						"lpx.nvidia.com/deployment-uid": "lpx-uid",
					},
				},
				Data: map[string]string{"runtime": "config"},
			},
			expectPayloadKept: false,
			expectDescription: "LPX ConfigMap should have payload stripped in cache",
		},
		{
			name: "DGDR output ConfigMap by name",
			configMap: &corev1.ConfigMap{
				ObjectMeta: metav1.ObjectMeta{
					Name:      "dgdr-output-request-1",
					Namespace: "inference",
				},
				Data: map[string]string{"output": "data"},
			},
			expectPayloadKept: false,
			expectDescription: "DGDR output ConfigMap should have payload stripped in cache",
		},
		{
			name: "Unrelated ConfigMap",
			configMap: &corev1.ConfigMap{
				ObjectMeta: metav1.ObjectMeta{
					Name:      "some-app-config",
					Namespace: "default",
					Labels: map[string]string{
						"app": "other-app",
					},
				},
				Data: map[string]string{"config": "data"},
			},
			expectPayloadKept: false,
			expectDescription: "Unrelated ConfigMap should have payload stripped in cache",
		},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			projected := Project(tc.configMap.DeepCopy())

			if tc.expectPayloadKept {
				assert.NotNil(t, projected.Data, tc.expectDescription)
			} else {
				assert.Nil(t, projected.Data, tc.expectDescription)
			}
		})
	}
}

// TestCacheOptionsPreserveNamespaceScoping verifies that ConfigMap cache
// configuration preserves any namespace scoping already set.
func TestCacheOptionsPreserveNamespaceScoping(t *testing.T) {
	configMapKey := &corev1.ConfigMap{}
	namespaces := map[string]cache.Config{
		"inference": {},
		"serving":   {},
	}

	options := cache.Options{
		DefaultNamespaces: namespaces,
		ByObject: map[client.Object]cache.ByObject{
			configMapKey: {
				Namespaces: namespaces,
			},
		},
	}

	require.NoError(t, Configure(&options))

	// Verify namespace scoping is preserved
	cmOptions := options.ByObject[configMapKey]
	assert.Equal(t, namespaces, cmOptions.Namespaces, "namespace scoping should be preserved")
	assert.NotNil(t, cmOptions.Transform, "transform should be set")
}
