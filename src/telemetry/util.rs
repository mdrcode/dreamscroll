use opentelemetry::trace::TraceContextExt;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Reads the current OTel trace ID (via `tracing` → `tracing-opentelemetry`'s
/// span extension), e.g. to surface in error responses for trace correlation.
pub fn current_trace_id() -> Option<String> {
    let context = tracing::Span::current().context();
    let span = context.span();
    let span_context = span.span_context();

    if span_context.is_valid() {
        Some(span_context.trace_id().to_string())
    } else {
        None
    }
}
