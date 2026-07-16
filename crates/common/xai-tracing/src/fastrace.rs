use fastrace::prelude::*;
use std::borrow::Cow;

pub fn current_trace_id() -> Option<String> {
    SpanContext::current_local_parent().map(|current| current.encode_w3c_traceparent())
}

pub fn local_or_random_span_ctx() -> SpanContext {
    SpanContext::current_local_parent().unwrap_or_else(SpanContext::random)
}

pub fn enter_span_with_traceparent(name: impl Into<Cow<'static, str>>, traceparent: &str) -> Span {
    if let Some(span_ctx) = SpanContext::decode_w3c_traceparent(traceparent) {
        Span::root(name, span_ctx)
    } else {
        Span::enter_with_local_parent(name)
    }
}
