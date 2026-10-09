/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

// Package configmapcache defines the ConfigMap representation retained by the shared
// controller-runtime cache to prevent OOMs from large unrelated ConfigMap payloads.
package configmapcache

import (
	"fmt"

	corev1 "k8s.io/api/core/v1"
	"sigs.k8s.io/controller-runtime/pkg/cache"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/ai-dynamo/dynamo/deploy/operator/internal/consts"
)

// Configure installs a label selector and Transform on the ConfigMap cache to retain
// only Dynamo-related ConfigMaps with payloads. Non-Dynamo ConfigMaps are stripped
// to metadata only.
func Configure(options *cache.Options) error {
	if options == nil {
		return nil
	}
	if options.ByObject == nil {
		options.ByObject = make(map[client.Object]cache.ByObject)
	}

	var configMapKey client.Object
	for obj := range options.ByObject {
		if _, ok := obj.(*corev1.ConfigMap); !ok {
			continue
		}
		if configMapKey != nil {
			return fmt.Errorf("multiple ConfigMap cache configurations are not supported")
		}
		configMapKey = obj
	}

	// Configure the cache to cache all ConfigMaps but transform non-Dynamo ones to drop payloads.
	// Controllers must use the APIReader when they need full ConfigMap contents for Dynamo ConfigMaps.
	if configMapKey == nil {
		options.ByObject[&corev1.ConfigMap{}] = cache.ByObject{
			Transform: Transform,
		}
		return nil
	}

	cmOptions := options.ByObject[configMapKey]
	cmOptions.Transform = Transform
	options.ByObject[configMapKey] = cmOptions
	return nil
}

// Transform strips ConfigMap payloads (Data and BinaryData) from non-Dynamo ConfigMaps
// before they enter the shared informer cache. Dynamo ConfigMaps are identified by
// the presence of Dynamo-specific labels or name prefixes.
func Transform(obj any) (any, error) {
	cm, ok := obj.(*corev1.ConfigMap)
	if !ok {
		return obj, nil
	}
	return Project(cm), nil
}

// Project reduces ConfigMap in place to the cached representation.
//
// For non-Dynamo ConfigMaps, only metadata is retained. For Dynamo ConfigMaps
// (identified by labels or naming patterns), full contents are preserved.
// Controllers reading Dynamo ConfigMap Data must use the manager APIReader
// to ensure they get the full contents.
func Project(cm *corev1.ConfigMap) *corev1.ConfigMap {
	if cm == nil {
		return nil
	}

	// Identify Dynamo ConfigMaps by label presence
	labels := cm.GetLabels()
	if labels != nil {
		// DGD-owned ConfigMaps have this label
		if _, ok := labels[consts.KubeLabelDynamoGraphDeploymentName]; ok {
			// This is a Dynamo ConfigMap, but we still strip payloads from cache.
			// Controllers must use APIReader for full contents.
			return projectMetadataOnly(cm)
		}
		// LPX-owned ConfigMaps have this label
		if _, ok := labels["lpx.nvidia.com/deployment-uid"]; ok {
			return projectMetadataOnly(cm)
		}
	}

	// DGDR output ConfigMaps use a naming pattern
	if len(cm.Name) >= 12 && cm.Name[:12] == "dgdr-output-" {
		return projectMetadataOnly(cm)
	}

	// For all other ConfigMaps, strip payloads
	return projectMetadataOnly(cm)
}

func projectMetadataOnly(cm *corev1.ConfigMap) *corev1.ConfigMap {
	// Retain all metadata (labels, annotations, owner references, identity, finalizers, deletion state)
	// but discard ManagedFields and payload data
	cm.ManagedFields = nil
	cm.Data = nil
	cm.BinaryData = nil
	return cm
}
