/*
 * SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

package controller

import (
	"testing"

	"github.com/ai-dynamo/dynamo/deploy/operator/api/v1beta1"
)

func dgdWithComponents(components ...v1beta1.DynamoComponentDeploymentSharedSpec) *v1beta1.DynamoGraphDeployment {
	return &v1beta1.DynamoGraphDeployment{
		Spec: v1beta1.DynamoGraphDeploymentSpec{
			Components: components,
		},
	}
}

func comp(name string, t v1beta1.ComponentType, sa *v1beta1.ScalingAdapter) v1beta1.DynamoComponentDeploymentSharedSpec {
	return v1beta1.DynamoComponentDeploymentSharedSpec{
		ComponentName:  name,
		ComponentType:  t,
		ScalingAdapter: sa,
	}
}

func scalingAdapterEnabled(dgd *v1beta1.DynamoGraphDeployment, componentName string) bool {
	for i := range dgd.Spec.Components {
		if dgd.Spec.Components[i].ComponentName == componentName {
			return dgd.Spec.Components[i].ScalingAdapter != nil
		}
	}
	return false
}

// A planner present in the deployment opts every worker-class component
// (worker/prefill/decode) into the scalingAdapter, while frontend, epp, and the
// planner itself are left untouched.
func TestEnablePlannerScalingAdapters_EnablesWorkersWhenPlannerPresent(t *testing.T) {
	dgd := dgdWithComponents(
		comp("frontend", v1beta1.ComponentTypeFrontend, nil),
		comp("planner", v1beta1.ComponentTypePlanner, nil),
		comp("prefill", v1beta1.ComponentTypePrefill, nil),
		comp("decode", v1beta1.ComponentTypeDecode, nil),
		comp("epp", v1beta1.ComponentTypeEPP, nil),
	)

	got := enablePlannerScalingAdapters(dgd)
	if got != 2 {
		t.Fatalf("expected 2 components opted in, got %d", got)
	}

	for _, name := range []string{"prefill", "decode"} {
		if !scalingAdapterEnabled(dgd, name) {
			t.Errorf("expected component %q to have scalingAdapter enabled", name)
		}
	}
	for _, name := range []string{"frontend", "planner", "epp"} {
		if scalingAdapterEnabled(dgd, name) {
			t.Errorf("component %q should not have scalingAdapter enabled", name)
		}
	}
}

// An aggregated deployment with a single worker component is handled too.
func TestEnablePlannerScalingAdapters_AggregatedWorker(t *testing.T) {
	dgd := dgdWithComponents(
		comp("frontend", v1beta1.ComponentTypeFrontend, nil),
		comp("planner", v1beta1.ComponentTypePlanner, nil),
		comp("worker", v1beta1.ComponentTypeWorker, nil),
	)

	if got := enablePlannerScalingAdapters(dgd); got != 1 {
		t.Fatalf("expected 1 component opted in, got %d", got)
	}
	if !scalingAdapterEnabled(dgd, "worker") {
		t.Error("expected worker component to have scalingAdapter enabled")
	}
}

// Without a planner component nothing is changed.
func TestEnablePlannerScalingAdapters_NoPlannerLeavesUntouched(t *testing.T) {
	dgd := dgdWithComponents(
		comp("frontend", v1beta1.ComponentTypeFrontend, nil),
		comp("worker", v1beta1.ComponentTypeWorker, nil),
	)

	if got := enablePlannerScalingAdapters(dgd); got != 0 {
		t.Fatalf("expected 0 components opted in without a planner, got %d", got)
	}
	if scalingAdapterEnabled(dgd, "worker") {
		t.Error("worker should not be opted in when no planner is present")
	}
}

// A component that already declares a scalingAdapter is not double-counted and
// its existing (possibly user-provided) value is preserved.
func TestEnablePlannerScalingAdapters_PreservesExisting(t *testing.T) {
	existing := &v1beta1.ScalingAdapter{}
	dgd := dgdWithComponents(
		comp("planner", v1beta1.ComponentTypePlanner, nil),
		comp("prefill", v1beta1.ComponentTypePrefill, existing),
		comp("decode", v1beta1.ComponentTypeDecode, nil),
	)

	if got := enablePlannerScalingAdapters(dgd); got != 1 {
		t.Fatalf("expected only the previously-unset component to be opted in, got %d", got)
	}
	// The pre-existing pointer must be untouched.
	for i := range dgd.Spec.Components {
		if dgd.Spec.Components[i].ComponentName == "prefill" && dgd.Spec.Components[i].ScalingAdapter != existing {
			t.Error("existing scalingAdapter pointer on prefill was replaced")
		}
	}
	if !scalingAdapterEnabled(dgd, "decode") {
		t.Error("expected decode component to be opted in")
	}
}

// A nil deployment is handled gracefully.
func TestEnablePlannerScalingAdapters_NilDGD(t *testing.T) {
	if got := enablePlannerScalingAdapters(nil); got != 0 {
		t.Fatalf("expected 0 for nil DGD, got %d", got)
	}
}
