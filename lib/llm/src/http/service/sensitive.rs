// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Sensitive header detection for metadata extraction and request tracing.

/// Standard redaction placeholder for sensitive values.
pub const REDACTED_PLACEHOLDER: &str = "<redacted>";

/// Header names that carry credentials and must never be captured unredacted.
const SENSITIVE_HEADER_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "x-auth-token",
    "x-access-token",
];

/// Returns `true` if the header name or value indicates a credential.
///
/// This check is non-overridable: even if a header name is explicitly allowlisted,
/// it will be redacted if it matches the denylist or carries a bearer token.
pub fn is_sensitive_header(raw_key: &str, raw_value: &str) -> bool {
    let value: &str = raw_value.trim_start();

    // Check if the header name is in the denylist
    if SENSITIVE_HEADER_NAMES
        .iter()
        .any(|name| raw_key.eq_ignore_ascii_case(name))
    {
        return true;
    }

    // Check if the value contains a bearer token
    value
        .get(.."bearer ".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bearer "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_header_is_sensitive() {
        assert!(is_sensitive_header("authorization", "Bearer token"));
        assert!(is_sensitive_header("Authorization", "Bearer token"));
        assert!(is_sensitive_header("AUTHORIZATION", "any value"));
    }

    #[test]
    fn proxy_authorization_header_is_sensitive() {
        assert!(is_sensitive_header("proxy-authorization", "Basic token"));
        assert!(is_sensitive_header("Proxy-Authorization", "any value"));
    }

    #[test]
    fn cookie_headers_are_sensitive() {
        assert!(is_sensitive_header("cookie", "session=abc"));
        assert!(is_sensitive_header("Cookie", "token=xyz"));
        assert!(is_sensitive_header("set-cookie", "auth=123"));
        assert!(is_sensitive_header("Set-Cookie", "value"));
    }

    #[test]
    fn api_key_headers_are_sensitive() {
        assert!(is_sensitive_header("x-api-key", "key123"));
        assert!(is_sensitive_header("X-API-Key", "key456"));
        assert!(is_sensitive_header("api-key", "key789"));
        assert!(is_sensitive_header("API-Key", "value"));
    }

    #[test]
    fn auth_token_headers_are_sensitive() {
        assert!(is_sensitive_header("x-auth-token", "token123"));
        assert!(is_sensitive_header("X-Auth-Token", "token456"));
        assert!(is_sensitive_header("x-access-token", "token789"));
        assert!(is_sensitive_header("X-Access-Token", "value"));
    }

    #[test]
    fn bearer_token_in_any_header_is_sensitive() {
        assert!(is_sensitive_header("x-custom-header", "Bearer secret"));
        assert!(is_sensitive_header("x-token", "bearer secret"));
        assert!(is_sensitive_header("x-auth", "BEARER secret"));
        assert!(is_sensitive_header("x-credential", "  Bearer token"));
    }

    #[test]
    fn non_sensitive_headers_pass_through() {
        assert!(!is_sensitive_header("x-request-id", "abc-123"));
        assert!(!is_sensitive_header("x-tenant", "acme"));
        assert!(!is_sensitive_header("content-type", "application/json"));
        assert!(!is_sensitive_header("x-custom", "some value"));
    }

    #[test]
    fn non_bearer_values_in_non_credential_headers_pass_through() {
        assert!(!is_sensitive_header("x-token-type", "JWT"));
        assert!(!is_sensitive_header("x-auth-method", "OAuth2"));
        assert!(!is_sensitive_header(
            "x-description",
            "This is not a bearer token"
        ));
    }
}
