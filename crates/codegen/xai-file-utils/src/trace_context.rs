//! Local-only compatibility shims for the removed distributed-tracing
//! bridge. Spans stay in-process: no W3C `traceparent`/`tracestate` headers
//! are produced or consumed, and no context is propagated to remote services.

use reqwest::header::HeaderMap;

/// Formerly extracted the current span's W3C `traceparent`. Tracing is now
/// local-only, so there is never a propagable trace context.
pub fn current_traceparent() -> Option<String> {
    None
}

/// Formerly injected `traceparent`/`tracestate` headers into the request.
/// Now returns the builder unchanged.
pub fn inject_trace_context_into_request(
    builder: reqwest::RequestBuilder,
) -> reqwest::RequestBuilder {
    builder
}

/// Formerly returned trace-context headers (traceparent, tracestate) for the
/// current span. Now always empty.
pub(crate) fn trace_context_headers() -> HeaderMap {
    HeaderMap::new()
}

/// Formerly created a span parented to `_meta.traceparent`. Now returns an
/// ordinary local span; the metadata is ignored.
pub fn span_from_meta_traceparent(
    meta: &serde_json::Map<String, serde_json::Value>,
) -> tracing::Span {
    let _ = meta;
    tracing::info_span!("acp_dispatch")
}

/// Formerly linked the current span to a W3C `traceparent` carried inside a
/// JSON `_meta` object. Now a no-op.
pub fn link_current_span_to_meta(meta: &serde_json::Value) {
    let _ = meta;
}

#[cfg(test)]
mod tests {
    #[test]
    fn implementation_has_no_otel_bridge() {
        let source = include_str!("trace_context.rs");
        let forbidden = format!("opente{}", "lemetry");
        let count = source.matches(&forbidden).count();
        // Exactly one occurrence: the assertion message below.
        assert_eq!(count, 1, "opentelemetry bridge survived");
    }

    #[test]
    fn current_traceparent_is_always_none() {
        let span = tracing::info_span!("some_span");
        let _entered = span.enter();
        assert_eq!(super::current_traceparent(), None);
    }

    #[test]
    fn request_builder_never_gains_trace_headers() {
        let span = tracing::info_span!("some_span");
        let _entered = span.enter();
        let request = super::inject_trace_context_into_request(
            reqwest::Client::new().get("https://example.invalid"),
        )
        .build()
        .unwrap();
        assert!(!request.headers().contains_key("traceparent"));
        assert!(!request.headers().contains_key("tracestate"));
    }

    #[test]
    fn request_builder_preserves_existing_headers() {
        let request = super::inject_trace_context_into_request(
            reqwest::Client::new()
                .get("https://example.invalid")
                .header("x-custom-header", "custom-value")
                .header("authorization", "Bearer token123"),
        )
        .build()
        .unwrap();
        assert_eq!(
            request
                .headers()
                .get("x-custom-header")
                .map(|v| v.to_str().unwrap()),
            Some("custom-value")
        );
        assert_eq!(
            request
                .headers()
                .get("authorization")
                .map(|v| v.to_str().unwrap()),
            Some("Bearer token123")
        );
    }

    #[test]
    fn trace_context_headers_is_empty() {
        let span = tracing::info_span!("some_span");
        let _entered = span.enter();
        assert!(super::trace_context_headers().is_empty());
    }

    #[test]
    fn meta_span_and_linking_are_local_no_ops() {
        let meta = serde_json::json!({
            "traceparent": "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
        });
        let map = meta.as_object().unwrap();
        // Must not panic and must return an ordinary local span.
        let _span = super::span_from_meta_traceparent(map);
        super::link_current_span_to_meta(&meta);
    }
}
