//! Reset Prefix Cache Endpoint
//!
//! This module provides HTTP endpoint functionality for resetting prefix caches across
//! distributed worker groups. The endpoint allows clearing cached key-value pairs to ensure
//! fresh computation for subsequent requests.

// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use super::{RouteDoc, service_v2};
use axum::{Json, Router, http::Method, response::IntoResponse, routing::post};
use serde_json::json;
use std::sync::Arc;

use dynamo_runtime::{discovery::DiscoveryQuery, pipeline::PushRouter, stream::StreamExt};

/// Endpoint path constant for the reset_prefix_cache operation
pub const RESET_PREFIX_CACHE_ENDPOINT: &str = "reset_prefix_cache";

/// Create the reset_prefix_cache HTTP router
///
/// This function creates an Axum router that handles POST requests to reset prefix caches
/// across all active worker groups in the Dynamo cluster.
///
/// # Arguments
/// * `state` - The shared HTTP service state containing model manager and runtime
/// * `path` - Optional custom path for the endpoint (defaults to "/reset_prefix_cache")
///
/// # Returns
/// A tuple containing:
/// * `Vec<RouteDoc>` - Documentation for the route (method and path)
/// * `Router` - The Axum router configured with the reset_prefix_cache handler
pub fn reset_prefix_cache_router(
    state: Arc<service_v2::State>,
    path: Option<String>,
) -> (Vec<RouteDoc>, Router) {
    let path = path.unwrap_or_else(|| "/reset_prefix_cache".to_string());

    let docs: Vec<RouteDoc> = vec![RouteDoc::new(Method::POST, &path)];

    let router = Router::new()
        .route(&path, post(reset_prefix_cache_handler))
        .with_state(state);

    (docs, router)
}

/// Handle POST requests to reset prefix caches
///
/// This handler processes requests to clear prefix caches on all active worker instances.
/// It iterates through each worker group, discovers instances with the reset_prefix_cache endpoint,
/// and sends reset requests to each instance.
///
/// The response includes lists of successfully cleared workers and workers that failed.
/// If no active worker groups are found or the runtime cannot be created, an appropriate
/// error message is returned in the JSON response.
///
/// # Arguments
/// * `state` - The HTTP service state with model manager and runtime
///
/// # Returns
/// A JSON response containing:
/// * `cleared_workers` - Array of successfully reset worker instances with their responses
/// * `failed_workers` - Array of workers that failed with error messages
async fn reset_prefix_cache_handler(
    axum::extract::State(state): axum::extract::State<Arc<service_v2::State>>,
) -> impl IntoResponse {
    let model_entries = state.manager().get_model_entries();

    // if there are no active workers
    if model_entries.is_empty() {
        return Json(serde_json::json!({
            "message": "No active worker groups found"
        }));
    }

    let distributed = match state.runtime() {
        Some(runtime) => runtime,
        None => {
            return Json(serde_json::json!({
                "message": "Failed to create distributed runtime",
            }));
        }
    };

    let mut cleared_workers = Vec::new();
    let mut failed_workers = Vec::new();

    /// Helper function to categorize and format worker result
    ///
    /// This function creates a JSON entry for a worker's operation result and adds it
    /// to either the success or failure list based on the outcome.
    ///
    /// # Arguments
    /// * `success` - Whether the operation succeeded
    /// * `name` - The worker instance name
    /// * `status` - Human-readable status message
    /// * `ns` - Kubernetes namespace of the worker
    /// * `comp` - Component name in the namespace
    /// * `message` - Optional message (response on success, error details on failure)
    /// * `cleared` - Mutable vector to append successful results to
    /// * `failed` - Mutable vector to append failed results to
    fn add_worker_result(
        success: bool,
        name: String,
        status: &str,
        ns: &str,
        comp: &str,
        message: Option<String>,
        cleared: &mut Vec<serde_json::Value>,
        failed: &mut Vec<serde_json::Value>,
    ) {
        let mut result = json!({
            "name": name,
            "endpoint": format!("{}/{}/{}", ns, comp, RESET_PREFIX_CACHE_ENDPOINT),
            "status": status,
        });
        if success {
            if let Some(m) = message {
                result["response"] = json!(m);
            }
            cleared.push(result);
        } else {
            if let Some(m) = message {
                result["error"] = json!(m);
            }
            failed.push(result);
        }
    }

    // create client for each model entry
    for entry in &model_entries {
        let namespace = &entry.endpoint_id.namespace;
        let component = &entry.endpoint_id.component;
        let entry_name = entry.name.to_string();

        tracing::debug!("Processing worker group: {}/{}", namespace, component);

        let namespace_obj = match distributed.namespace(namespace) {
            Ok(ns) => ns,
            Err(e) => {
                add_worker_result(
                    false,
                    entry_name,
                    "Failed to get namespace",
                    namespace,
                    component,
                    Some(e.to_string()),
                    &mut cleared_workers,
                    &mut failed_workers,
                );
                continue;
            }
        };

        let component_obj = match namespace_obj.component(component) {
            Ok(comp) => comp,
            Err(e) => {
                add_worker_result(
                    false,
                    entry_name,
                    "Failed to get component",
                    namespace,
                    component,
                    Some(e.to_string()),
                    &mut cleared_workers,
                    &mut failed_workers,
                );
                continue;
            }
        };

        let endpoint: dynamo_runtime::component::Endpoint =
            component_obj.endpoint(RESET_PREFIX_CACHE_ENDPOINT);

        let client = match endpoint.client().await {
            Ok(c) => c,
            Err(e) => {
                add_worker_result(
                    false,
                    entry_name,
                    "Failed to get client",
                    namespace,
                    component,
                    Some(e.to_string()),
                    &mut cleared_workers,
                    &mut failed_workers,
                );
                continue;
            }
        };

        let router = match PushRouter::<(), serde_json::Value>::from_client(
            client.clone(),
            Default::default(),
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                add_worker_result(
                    false,
                    entry_name,
                    "Failed to create router",
                    namespace,
                    component,
                    Some(e.to_string()),
                    &mut cleared_workers,
                    &mut failed_workers,
                );
                continue;
            }
        };

        let discovery_client = distributed.discovery();
        let discovery_key = DiscoveryQuery::Endpoint {
            namespace: namespace.clone(),
            component: component.clone(),
            endpoint: RESET_PREFIX_CACHE_ENDPOINT.to_string(),
        };

        let discovery_instances = match discovery_client.list(discovery_key).await {
            Ok(instances) => instances,
            Err(e) => {
                add_worker_result(
                    false,
                    entry_name,
                    "Failed to get instances for worker group",
                    namespace,
                    component,
                    Some(e.to_string()),
                    &mut cleared_workers,
                    &mut failed_workers,
                );
                continue;
            }
        };

        if discovery_instances.is_empty() {
            add_worker_result(
                false,
                entry_name,
                "No instances found for reset_prefix_cache endpoint",
                namespace,
                component,
                None,
                &mut cleared_workers,
                &mut failed_workers,
            );
            continue;
        }

        let instances_filtered: Vec<dynamo_runtime::component::Instance> = discovery_instances
            .into_iter()
            .filter_map(|di| match di {
                dynamo_runtime::discovery::DiscoveryInstance::Endpoint(instance) => Some(instance),
                _ => None,
            })
            .collect();

        for instance in &instances_filtered {
            let instance_name = format!("{}-instance-{}", entry.name, instance.id());
            match router.direct(().into(), instance.id()).await {
                Ok(mut stream) => match stream.next().await {
                    Some(response) => {
                        add_worker_result(
                            true,
                            instance_name,
                            "Successfully reset prefix cache for instance",
                            namespace,
                            component,
                            Some(response.to_string()),
                            &mut cleared_workers,
                            &mut failed_workers,
                        );
                    }
                    None => {
                        add_worker_result(
                            false,
                            instance_name,
                            "No response from instance",
                            namespace,
                            component,
                            None,
                            &mut cleared_workers,
                            &mut failed_workers,
                        );
                    }
                },
                Err(e) => {
                    add_worker_result(
                        false,
                        instance_name,
                        "Failed to send request for instance",
                        namespace,
                        component,
                        Some(e.to_string()),
                        &mut cleared_workers,
                        &mut failed_workers,
                    );
                }
            }
        }
    }

    Json(serde_json::json!({
        "cleared_workers": cleared_workers,
        "failed_workers": failed_workers
    }))
}
