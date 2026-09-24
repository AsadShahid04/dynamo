// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use dynamo_protocols::types::{ChatCompletionStreamOptions, CompletionUsage};

use crate::protocols::common::{
    extensions::{NvExt, NvExtResponseFieldSelection},
    llm_backend::BackendOutput,
    timing::RequestTracker,
};

/// Configuration options for the [`DeltaGenerator`], controlling response behavior.
#[derive(Debug, Clone, Default)]
pub struct DeltaGeneratorOptions {
    /// Determines whether token usage statistics should be included in the response.
    pub enable_usage: bool,
    /// Determines whether continuous usage statistics should be included in the response.
    pub continuous_usage_stats: bool,
    /// Determines whether log probabilities should be included in the response.
    pub enable_logprobs: bool,
    /// When true, logprob token fields use "token_id:<id>" format instead of decoded text.
    pub return_tokens_as_token_ids: bool,
    /// Determines which nvext response fields may be emitted for this request.
    pub response_fields: NvExtResponseFieldSelection,
}

impl DeltaGeneratorOptions {
    pub fn new(
        stream_options: Option<&ChatCompletionStreamOptions>,
        return_tokens_as_token_ids: Option<bool>,
        enable_logprobs: bool,
        nvext: Option<&NvExt>,
    ) -> Self {
        let response_fields = NvExtResponseFieldSelection::from_nvext(nvext);
        DeltaGeneratorOptions {
            enable_usage: stream_options.is_some_and(|opts| opts.include_usage),
            continuous_usage_stats: stream_options.is_some_and(|opts| opts.continuous_usage_stats),
            enable_logprobs,
            response_fields,
            return_tokens_as_token_ids: return_tokens_as_token_ids.unwrap_or(false),
        }
    }
}

/// State and lifecycle behavior shared by the chat and text completion delta generators.
pub(crate) struct DeltaGeneratorState {
    id: String,
    object: String,
    created: u32,
    model: String,
    system_fingerprint: Option<String>,
    usage: CompletionUsage,
    options: DeltaGeneratorOptions,
    tracker: Arc<RequestTracker>,
    /// The original prompt token count (ISL) set at request start. Once set,
    /// this pins the client-facing `prompt_tokens` value to prevent backend-
    /// reported prompt lengths from overwriting it after migration. When a
    /// worker fails mid-stream, RetryManager appends already-generated tokens
    /// to the retry request; the new worker sees those as part of its prompt
    /// and inflates `prompt_tokens`. Pinning the original ISL ensures the client
    /// sees the true prompt length, not the replayed prompt + completion prefix.
    original_isl: Option<u32>,
}

impl DeltaGeneratorState {
    pub(crate) fn new(
        id: String,
        object: String,
        model: String,
        options: DeltaGeneratorOptions,
    ) -> Self {
        let now_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap() // cannot fail because UNIX_EPOCH is in the past
            .as_secs();
        // Casting from `u64` to `u32` could lead to precision loss after `u32::MAX`,
        // but this will not be an issue until 2106.
        let created: u32 = now_time.try_into().expect("timestamp exceeds u32::MAX");

        let usage = CompletionUsage {
            completion_tokens: 0,
            prompt_tokens: 0,
            total_tokens: 0,
            completion_tokens_details: None,
            prompt_tokens_details: None,
        };

        // Always create request tracker for per-worker metrics (TTFT, ITL per worker_id).
        // `response_fields` only controls which nvext fields are returned to the client;
        // the tracker still records timing/ITL internally for metrics.
        let tracker = Arc::new(RequestTracker::new());

        Self {
            id,
            object,
            created,
            model,
            system_fingerprint: None,
            usage,
            options,
            tracker,
            original_isl: None,
        }
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn object(&self) -> &str {
        &self.object
    }

    pub(crate) fn created(&self) -> u32 {
        self.created
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn system_fingerprint(&self) -> Option<&String> {
        self.system_fingerprint.as_ref()
    }

    pub(crate) fn options(&self) -> &DeltaGeneratorOptions {
        &self.options
    }

    pub(crate) fn tracker(&self) -> Arc<RequestTracker> {
        self.tracker.clone()
    }

    pub(crate) fn tracker_ref(&self) -> &Arc<RequestTracker> {
        &self.tracker
    }

    pub(crate) fn update_isl(&mut self, isl: u32) {
        // Pin the original ISL on first update. This prevents backend-reported
        // prompt_tokens from overwriting the true client-facing prompt length
        // after a migration retry, where the backend sees replayed completion
        // tokens as part of its prompt.
        if self.original_isl.is_none() {
            self.original_isl = Some(isl);
        }
        self.usage.prompt_tokens = isl;
    }

    pub(crate) fn update_usage_from_backend_output(&mut self, output: &BackendOutput) {
        // Aggregate token usage even if usage tracking is disabled for metrics tracking.
        // SAFETY: Casting from `usize` to `u32` could lead to precision loss after `u32::MAX`,
        // but this will not be an issue until context lengths exceed 4_294_967_295.
        let token_length: u32 = output
            .token_ids
            .len()
            .try_into()
            .expect("token_ids length exceeds u32::MAX");

        self.usage.completion_tokens += token_length;

        // If the backend provides completion_usage, use it to update usage stats.
        // This is critical for prompt embeddings where prompt_tokens comes from
        // the embedding sequence length computed by the worker.
        if let Some(completion_usage) = output.completion_usage.as_ref() {
            // If we have a pinned original ISL (set via update_isl before any backend
            // output), preserve it as the client-facing prompt_tokens. The backend may
            // report an inflated prompt length after migration because RetryManager
            // appends already-delivered tokens to the retry request. Those tokens are
            // already counted in completion_tokens, so overwriting prompt_tokens here
            // would double-count them.
            if self.original_isl.is_none() {
                self.usage.prompt_tokens = completion_usage.prompt_tokens;
            }
            self.usage.completion_tokens = self
                .usage
                .completion_tokens
                .max(completion_usage.completion_tokens);

            if let Some(prompt_details) = completion_usage.prompt_tokens_details.as_ref() {
                self.usage.prompt_tokens_details = Some(prompt_details.clone());
            }

            if let Some(completion_details) = completion_usage.completion_tokens_details.as_ref() {
                self.usage.completion_tokens_details = Some(completion_details.clone());
            }
        }
    }

    pub(crate) fn get_isl(&self) -> u32 {
        self.usage.prompt_tokens
    }

    pub(crate) fn get_usage(&self) -> CompletionUsage {
        let mut usage = self.usage.clone();
        usage.total_tokens = usage.prompt_tokens.saturating_add(usage.completion_tokens);
        usage
    }

    pub(crate) fn is_usage_enabled(&self) -> bool {
        self.options.enable_usage
    }

    pub(crate) fn is_continuous_usage_enabled(&self) -> bool {
        self.options.continuous_usage_stats
    }
}

/// Enables usage tracking for non-streaming requests to comply with OpenAI API specification.
///
/// According to OpenAI API spec, non-streaming chat completion responses (stream=false)
/// must always include usage statistics. This method ensures `stream_options.include_usage`
/// is set to `true` for non-streaming requests.
pub(crate) fn enable_usage_for_nonstreaming(
    stream_options: &mut Option<ChatCompletionStreamOptions>,
    original_stream_flag: bool,
) {
    if original_stream_flag {
        return;
    }
    // For non-streaming requests (stream=false), enable usage
    stream_options
        .get_or_insert_with(|| ChatCompletionStreamOptions {
            include_usage: true,
            continuous_usage_stats: false,
        })
        .include_usage = true;
}

/// Enables usage statistics regardless of the request's `include_usage` value.
pub(crate) fn force_include_usage(stream_options: &mut Option<ChatCompletionStreamOptions>) {
    stream_options
        .get_or_insert_with(|| ChatCompletionStreamOptions {
            include_usage: true,
            continuous_usage_stats: false,
        })
        .include_usage = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::common::llm_backend::BackendOutput;

    #[test]
    fn force_include_usage_inserts_missing_options() {
        let mut options = None;

        force_include_usage(&mut options);

        let options = options.expect("stream options should be inserted");
        assert!(options.include_usage);
        assert!(!options.continuous_usage_stats);
    }

    #[test]
    fn force_include_usage_overrides_false_and_preserves_siblings() {
        let mut options = Some(ChatCompletionStreamOptions {
            include_usage: false,
            continuous_usage_stats: true,
        });

        force_include_usage(&mut options);

        let options = options.expect("stream options should remain present");
        assert!(options.include_usage);
        assert!(options.continuous_usage_stats);
    }

    #[test]
    fn update_isl_pins_original_prompt_tokens() {
        let mut state = DeltaGeneratorState::new(
            "test-id".to_string(),
            "chat.completion".to_string(),
            "test-model".to_string(),
            DeltaGeneratorOptions::default(),
        );

        // First update_isl call should pin the original ISL
        state.update_isl(6);
        assert_eq!(state.original_isl, Some(6));
        assert_eq!(state.usage.prompt_tokens, 6);

        // Subsequent update_isl calls should not change the pinned original ISL
        state.update_isl(18);
        assert_eq!(state.original_isl, Some(6), "original ISL must remain pinned");
        assert_eq!(
            state.usage.prompt_tokens, 18,
            "usage.prompt_tokens can still be updated"
        );
    }

    #[test]
    fn update_usage_preserves_pinned_isl_after_migration() {
        let mut state = DeltaGeneratorState::new(
            "test-id".to_string(),
            "chat.completion".to_string(),
            "test-model".to_string(),
            DeltaGeneratorOptions::default(),
        );

        // Simulate initial ISL update (6-token prompt)
        state.update_isl(6);
        assert_eq!(state.original_isl, Some(6));
        assert_eq!(state.usage.prompt_tokens, 6);

        // Simulate backend output that inflates prompt_tokens after migration.
        // The worker sees the original 6 prompt tokens + 12 replayed completion
        // tokens = 18 total, and reports prompt_tokens=18.
        let backend_usage = CompletionUsage {
            prompt_tokens: 18, // inflated due to replayed tokens
            completion_tokens: 10,
            total_tokens: 28,
            completion_tokens_details: None,
            prompt_tokens_details: None,
        };
        let output = BackendOutput {
            token_ids: vec![100, 101, 102],
            tokens: vec![],
            text: None,
            cum_log_probs: None,
            log_probs: None,
            top_logprobs: None,
            finish_reason: None,
            stop_reason: None,
            index: None,
            completion_usage: Some(backend_usage),
            disaggregated_params: None,
            encoder_result: None,
            worker_trace_link: None,
            engine_data: None,
            routing_data: None,
        };

        state.update_usage_from_backend_output(&output);

        // The original ISL (6) must be preserved as client-facing prompt_tokens
        assert_eq!(
            state.usage.prompt_tokens, 6,
            "prompt_tokens must stay pinned to original ISL after migration"
        );
        assert_eq!(state.usage.completion_tokens, 10);
        assert_eq!(state.get_usage().total_tokens, 16); // 6 + 10
    }

    #[test]
    fn update_usage_allows_backend_prompt_tokens_without_pinned_isl() {
        let mut state = DeltaGeneratorState::new(
            "test-id".to_string(),
            "chat.completion".to_string(),
            "test-model".to_string(),
            DeltaGeneratorOptions::default(),
        );

        // No update_isl call, so original_isl remains None
        assert_eq!(state.original_isl, None);

        // Backend-reported prompt_tokens should be accepted when original_isl is None
        let backend_usage = CompletionUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            completion_tokens_details: None,
            prompt_tokens_details: None,
        };
        let output = BackendOutput {
            token_ids: vec![1, 2, 3],
            tokens: vec![],
            text: None,
            cum_log_probs: None,
            log_probs: None,
            top_logprobs: None,
            finish_reason: None,
            stop_reason: None,
            index: None,
            completion_usage: Some(backend_usage),
            disaggregated_params: None,
            encoder_result: None,
            worker_trace_link: None,
            engine_data: None,
            routing_data: None,
        };

        state.update_usage_from_backend_output(&output);

        // Without a pinned ISL, backend-reported prompt_tokens should be accepted
        assert_eq!(
            state.usage.prompt_tokens, 100,
            "backend prompt_tokens should be accepted when ISL is not pinned"
        );
        assert_eq!(state.usage.completion_tokens, 50);
    }
}

