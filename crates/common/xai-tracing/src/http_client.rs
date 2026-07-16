use http::HeaderMap;

/// Formerly injected W3C `traceparent`/`tracestate` headers from the active
/// span into outbound HTTP requests. Tracing is now local-only, so this is a
/// no-op kept for API compatibility: the headers are left untouched.
pub fn attach_trace_to_http_request(headers: &mut HeaderMap) {
    let _ = headers;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implementation_has_no_otel_bridge() {
        let source = include_str!("http_client.rs");
        let forbidden = format!("opente{}", "lemetry");
        let count = source.matches(&forbidden).count();
        // Exactly one occurrence: the assertion message below.
        assert_eq!(count, 1, "opentelemetry bridge survived");
    }

    #[test]
    fn attach_trace_never_adds_headers() {
        let span = tracing::info_span!("http_request");
        let _enter = span.enter();
        let mut headers = HeaderMap::new();
        attach_trace_to_http_request(&mut headers);
        assert!(headers.is_empty());
    }
}
