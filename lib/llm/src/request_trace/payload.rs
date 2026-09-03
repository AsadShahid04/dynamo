// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;

use crate::protocols::openai::chat_completions::{
    NvCreateChatCompletionRequest, NvCreateChatCompletionResponse,
};

/// Context key for the allowlisted headers captured at the HTTP layer.
pub const HTTP_HEADERS_CONTEXT_KEY: &str = "request_trace.http.request.headers";

/// True when payload records are being captured and a header allowlist is set.
pub(crate) fn http_header_capture_active() -> bool {
    if !super::config::capture_enabled() {
        return false;
    }
    let policy = super::policy();
    policy.emit_request_payload_records() && !policy.http_header_capture_list.is_empty()
}

/// Collect the allowlisted request headers (case-insensitive, comma-joined on
/// repeats). `None` unless payload capture is active and the allowlist is non-empty.
pub fn capture_http_headers(headers: &HeaderMap) -> Option<BTreeMap<String, String>> {
    if !http_header_capture_active() {
        return None;
    }
    capture_http_headers_with_list(headers, &super::policy().http_header_capture_list)
}

/// Maximum number of headers that can be captured.
const CAPTURE_MAX_ENTRIES: usize = 64;

/// Maximum total bytes (key + value) that can be captured.
const CAPTURE_MAX_TOTAL_BYTES: usize = 64 * 1024;

fn capture_http_headers_with_list(
    headers: &HeaderMap,
    capture_list: &[String],
) -> Option<BTreeMap<String, String>> {
    if capture_list.is_empty() {
        return None;
    }
    let mut out = BTreeMap::new();
    let mut total_bytes = 0usize;

    for name in capture_list {
        if out.len() >= CAPTURE_MAX_ENTRIES {
            break;
        }

        let joined = headers
            .get_all(name.as_str())
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", ");

        if joined.is_empty() {
            continue;
        }

        // Check if this header contains sensitive data
        let value_to_store = if crate::http::service::sensitive::is_sensitive_header(name, &joined)
        {
            crate::http::service::sensitive::REDACTED_PLACEHOLDER.to_string()
        } else {
            joined
        };

        let entry_bytes = name.len() + value_to_store.len();
        if total_bytes + entry_bytes > CAPTURE_MAX_TOTAL_BYTES {
            break;
        }

        total_bytes += entry_bytes;
        out.insert(name.clone(), value_to_store);
    }
    (!out.is_empty()).then_some(out)
}

pub struct RequestPayloadHandle {
    requested_streaming: bool,
    request_id: String,
    model: String,
    event_time: SystemTime,
    request: Arc<NvCreateChatCompletionRequest>,
    http_request_headers: Option<Arc<BTreeMap<String, String>>>,
}

impl RequestPayloadHandle {
    pub fn streaming(&self) -> bool {
        self.requested_streaming
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Publish one request trace payload record. Consumes the handle to enforce
    /// exactly one payload record per request. `response` is `None` on client
    /// cancel / gateway timeout / aggregation failure; the record still carries
    /// the request so those cases remain inspectable.
    pub fn emit(self, response: Option<Arc<NvCreateChatCompletionResponse>>) {
        super::record::emit_request_payload(
            super::RequestTracePayload {
                request_id: self.request_id,
                endpoint: "openai.chat_completion".to_string(),
                model: self.model,
                request: Some(self.request),
                response,
                http_request_headers: self.http_request_headers,
                payload_complete: true,
                payload_drop_reason: None,
            },
            unix_time_ms(self.event_time),
        );
    }
}

pub fn create_handle(
    req: &NvCreateChatCompletionRequest,
    request_id: &str,
    http_request_headers: Option<Arc<BTreeMap<String, String>>>,
) -> Option<RequestPayloadHandle> {
    let policy = super::policy();
    // `capture_enabled()` is `policy.enabled && CAPTURE_ACTIVE`: it additionally
    // requires request trace initialization, so a stale payload handle cannot be
    // created before/after the request trace lifecycle.
    create_handle_with_config(
        req,
        request_id,
        super::config::capture_enabled(),
        policy.emit_request_payload_records(),
        http_request_headers,
    )
}

fn unix_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn create_handle_with_config(
    req: &NvCreateChatCompletionRequest,
    request_id: &str,
    enabled: bool,
    emit_request_payload: bool,
    http_request_headers: Option<Arc<BTreeMap<String, String>>>,
) -> Option<RequestPayloadHandle> {
    if !enabled || !emit_request_payload {
        return None;
    }
    let requested_streaming = req.inner.stream.unwrap_or(false);
    let model = req.inner.model.clone();

    Some(RequestPayloadHandle {
        requested_streaming,
        request_id: request_id.to_string(),
        model,
        // Snapshot the pristine inbound request (before the preprocessor
        // overrides stream/usage) and stamp arrival time on the producing
        // thread, so the record reflects what the client sent and when.
        event_time: SystemTime::now(),
        request: Arc::new(req.clone()),
        http_request_headers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn create_test_request(model: &str, store: bool) -> NvCreateChatCompletionRequest {
        let json = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "test"}],
            "store": store
        });
        serde_json::from_value(json).expect("Failed to create test request")
    }

    fn create_test_response(content: &str) -> NvCreateChatCompletionResponse {
        let json = serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }]
        });
        serde_json::from_value(json).expect("Failed to create test response")
    }

    #[test]
    fn request_payload_records_emit_even_when_store_is_false() {
        let request = create_test_request("test-model", false);
        let handle = create_handle_with_config(&request, "test-id", true, true, None);

        assert!(
            handle.is_some(),
            "request_payload records should create a handle even with store=false"
        );
    }

    #[test]
    fn request_payload_records_disabled_skips_store_true_payloads() {
        let request = create_test_request("test-model", true);
        let handle = create_handle_with_config(&request, "test-id", true, false, None);

        assert!(
            handle.is_none(),
            "request_payload records disabled should skip payloads even with store=true"
        );
    }

    #[test]
    fn capture_http_headers_records_only_allowlisted() {
        let capture_list = vec!["x-request-id".to_string(), "nvcf-function-id".to_string()];

        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", "abc-123".parse().unwrap());
        headers.insert("NVCF-Function-Id", "fn-9".parse().unwrap());
        headers.insert("authorization", "Bearer secret".parse().unwrap());

        let captured = capture_http_headers_with_list(&headers, &capture_list)
            .expect("allowlisted headers are captured");
        assert_eq!(
            captured.get("x-request-id").map(String::as_str),
            Some("abc-123")
        );
        assert_eq!(
            captured.get("nvcf-function-id").map(String::as_str),
            Some("fn-9")
        );
        assert!(
            !captured.contains_key("authorization"),
            "non-allowlisted header must never be captured"
        );
    }

    #[test]
    fn capture_http_headers_empty_list_captures_nothing() {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", "abc-123".parse().unwrap());

        assert!(
            capture_http_headers_with_list(&headers, &[]).is_none(),
            "empty allowlist must capture nothing"
        );
    }

    #[test]
    fn capture_http_headers_joins_repeated_headers() {
        let capture_list = vec!["x-tag".to_string()];

        let mut headers = HeaderMap::new();
        headers.append("x-tag", "a".parse().unwrap());
        headers.append("x-tag", "b".parse().unwrap());

        let captured = capture_http_headers_with_list(&headers, &capture_list)
            .expect("repeated header is captured");
        assert_eq!(captured.get("x-tag").map(String::as_str), Some("a, b"));
    }

    #[test]
    fn capture_http_headers_omits_repeated_empty_values() {
        let capture_list = vec!["x-tag".to_string()];

        let mut headers = HeaderMap::new();
        headers.append("x-tag", "".parse().unwrap());
        headers.append("x-tag", "".parse().unwrap());

        assert!(
            capture_http_headers_with_list(&headers, &capture_list).is_none(),
            "repeated empty values must be omitted, not joined into \", \""
        );
    }

    #[test]
    fn capture_http_headers_skips_empty_values_when_joining() {
        let capture_list = vec!["x-tag".to_string()];

        let mut headers = HeaderMap::new();
        headers.append("x-tag", "".parse().unwrap());
        headers.append("x-tag", "tenant-a".parse().unwrap());

        let captured = capture_http_headers_with_list(&headers, &capture_list)
            .expect("non-empty value is captured");
        assert_eq!(captured.get("x-tag").map(String::as_str), Some("tenant-a"));
    }

    #[test]
    fn capture_http_headers_redacts_authorization_header() {
        let capture_list = vec!["authorization".to_string(), "x-request-id".to_string()];

        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret-token".parse().unwrap());
        headers.insert("x-request-id", "abc-123".parse().unwrap());

        let captured =
            capture_http_headers_with_list(&headers, &capture_list).expect("headers captured");

        assert_eq!(
            captured.get("authorization").map(String::as_str),
            Some(crate::http::service::sensitive::REDACTED_PLACEHOLDER),
            "authorization header must be redacted even when allowlisted"
        );
        assert_eq!(
            captured.get("x-request-id").map(String::as_str),
            Some("abc-123"),
            "non-sensitive header must pass through unmodified"
        );
    }

    #[test]
    fn capture_http_headers_redacts_bearer_token_in_custom_header() {
        let capture_list = vec!["x-custom-auth".to_string(), "x-tenant".to_string()];

        let mut headers = HeaderMap::new();
        headers.insert("x-custom-auth", "Bearer custom-token".parse().unwrap());
        headers.insert("x-tenant", "acme".parse().unwrap());

        let captured =
            capture_http_headers_with_list(&headers, &capture_list).expect("headers captured");

        assert_eq!(
            captured.get("x-custom-auth").map(String::as_str),
            Some(crate::http::service::sensitive::REDACTED_PLACEHOLDER),
            "bearer token in custom header must be redacted"
        );
        assert_eq!(
            captured.get("x-tenant").map(String::as_str),
            Some("acme"),
            "non-sensitive header must pass through"
        );
    }

    #[test]
    fn capture_http_headers_redacts_all_sensitive_header_names() {
        let capture_list = vec![
            "authorization".to_string(),
            "proxy-authorization".to_string(),
            "cookie".to_string(),
            "set-cookie".to_string(),
            "x-api-key".to_string(),
            "api-key".to_string(),
            "x-auth-token".to_string(),
            "x-access-token".to_string(),
            "x-safe".to_string(),
        ];

        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Basic token".parse().unwrap());
        headers.insert("proxy-authorization", "Basic proxy".parse().unwrap());
        headers.insert("cookie", "session=xyz".parse().unwrap());
        headers.insert("set-cookie", "auth=abc".parse().unwrap());
        headers.insert("x-api-key", "key123".parse().unwrap());
        headers.insert("api-key", "key456".parse().unwrap());
        headers.insert("x-auth-token", "token789".parse().unwrap());
        headers.insert("x-access-token", "token000".parse().unwrap());
        headers.insert("x-safe", "safe-value".parse().unwrap());

        let captured =
            capture_http_headers_with_list(&headers, &capture_list).expect("headers captured");

        for sensitive_header in &[
            "authorization",
            "proxy-authorization",
            "cookie",
            "set-cookie",
            "x-api-key",
            "api-key",
            "x-auth-token",
            "x-access-token",
        ] {
            assert_eq!(
                captured.get(*sensitive_header).map(String::as_str),
                Some(crate::http::service::sensitive::REDACTED_PLACEHOLDER),
                "{} header must be redacted",
                sensitive_header
            );
        }

        assert_eq!(
            captured.get("x-safe").map(String::as_str),
            Some("safe-value"),
            "non-sensitive header must pass through"
        );
    }

    #[test]
    fn capture_http_headers_respects_entry_limit() {
        use axum::http::HeaderName;
        
        let mut capture_list = Vec::new();
        for i in 0..100 {
            capture_list.push(format!("x-header-{}", i));
        }

        let mut headers = HeaderMap::new();
        for i in 0..100 {
            let name = format!("x-header-{}", i);
            let value = format!("value-{}", i);
            headers.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }

        let captured =
            capture_http_headers_with_list(&headers, &capture_list).expect("headers captured");

        assert_eq!(
            captured.len(),
            CAPTURE_MAX_ENTRIES,
            "must not exceed entry limit"
        );
    }

    #[test]
    fn capture_http_headers_respects_byte_limit() {
        let large_value = "a".repeat(CAPTURE_MAX_TOTAL_BYTES);
        let capture_list = vec!["x-large".to_string(), "x-next".to_string()];

        let mut headers = HeaderMap::new();
        headers.insert("x-large", large_value.parse().unwrap());
        headers.insert("x-next", "value".parse().unwrap());

        let captured =
            capture_http_headers_with_list(&headers, &capture_list).expect("headers captured");

        assert!(
            captured.contains_key("x-large"),
            "first header within limit must be captured"
        );
        assert!(
            !captured.contains_key("x-next"),
            "second header exceeding limit must be dropped"
        );
    }

    #[test]
    fn capture_http_headers_non_sensitive_passes_through_unmodified() {
        let capture_list = vec![
            "x-request-id".to_string(),
            "x-tenant".to_string(),
            "content-type".to_string(),
        ];

        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", "req-123".parse().unwrap());
        headers.insert("x-tenant", "acme-corp".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());

        let captured =
            capture_http_headers_with_list(&headers, &capture_list).expect("headers captured");

        assert_eq!(
            captured.get("x-request-id").map(String::as_str),
            Some("req-123")
        );
        assert_eq!(
            captured.get("x-tenant").map(String::as_str),
            Some("acme-corp")
        );
        assert_eq!(
            captured.get("content-type").map(String::as_str),
            Some("application/json")
        );
    }

    /// Test-only constructor. `create_handle` gates on env vars + a cached
    /// `OnceLock` policy, which is too brittle for a focused bus-roundtrip test.
    impl RequestPayloadHandle {
        pub(crate) fn for_test(request_id: &str, model: &str, streaming: bool) -> Self {
            Self {
                requested_streaming: streaming,
                request_id: request_id.to_string(),
                model: model.to_string(),
                event_time: SystemTime::now(),
                request: Arc::new(create_test_request(model, true)),
                http_request_headers: None,
            }
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn emit_publishes_one_combined_record_with_and_without_response() {
        // Exercises the contract: `emit` publishes exactly one request trace
        // payload record. With a response present the record carries both
        // request and response; with `None` it carries the request only.
        crate::request_trace::init_bus_for_test(8);
        let mut rx = crate::request_trace::subscribe();

        RequestPayloadHandle::for_test("payload-test-req-ok", "test-model", true)
            .emit(Some(Arc::new(create_test_response("hello"))));
        RequestPayloadHandle::for_test("payload-test-req-cancel", "test-model", true).emit(None);

        let mut records = HashMap::new();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while records.len() < 2 {
                let record = rx.recv().await.expect("record receives ok");
                if record.event_type != crate::request_trace::RequestTraceEventType::RequestPayload
                {
                    continue;
                }
                let Some(payload) = record.payload.as_ref() else {
                    continue;
                };
                if matches!(
                    payload.request_id.as_str(),
                    "payload-test-req-ok" | "payload-test-req-cancel"
                ) {
                    records.insert(payload.request_id.clone(), record);
                }
            }
        })
        .await
        .expect("expected request payload records before timeout");

        let first = records
            .remove("payload-test-req-ok")
            .expect("payload-test-req-ok record");
        let second = records
            .remove("payload-test-req-cancel")
            .expect("payload-test-req-cancel record");

        assert_eq!(
            first.event_type,
            crate::request_trace::RequestTraceEventType::RequestPayload
        );
        let first_payload = first.payload.as_ref().expect("first payload");
        assert_eq!(first_payload.request_id, "payload-test-req-ok");
        assert!(first_payload.request.is_some());
        assert!(first_payload.response.is_some());

        assert_eq!(
            second.event_type,
            crate::request_trace::RequestTraceEventType::RequestPayload
        );
        let second_payload = second.payload.as_ref().expect("second payload");
        assert_eq!(second_payload.request_id, "payload-test-req-cancel");
        assert!(second_payload.request.is_some());
        assert!(second_payload.response.is_none());
    }
}
