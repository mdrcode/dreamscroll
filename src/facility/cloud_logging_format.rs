use opentelemetry::trace::TraceContextExt;
use std::collections::BTreeMap;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{
    fmt::{self, FormatEvent, FormatFields},
    registry::LookupSpan,
};

/// Cloud Logging (Stackdriver) compatible JSON event formatter that reads
/// trace/span IDs directly from the OpenTelemetry 0.31 context, avoiding the
/// version-mismatch problem with `tracing-stackdriver`'s bundled OTel 0.22.
pub(crate) struct CloudLoggingFormat {
    pub project_id: String,
}

impl<S, N> FormatEvent<S, N> for CloudLoggingFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &fmt::FmtContext<'_, S, N>,
        mut writer: fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let severity = match *event.metadata().level() {
            tracing::Level::ERROR => "ERROR",
            tracing::Level::WARN => "WARNING",
            tracing::Level::INFO => "INFO",
            tracing::Level::DEBUG => "DEBUG",
            tracing::Level::TRACE => "DEBUG",
        };

        // Collect event fields
        let mut fields = BTreeMap::new();
        event.record(&mut JsonFieldVisitor(&mut fields));
        let message = fields.remove("message").unwrap_or(serde_json::Value::Null);

        let mut entry = serde_json::json!({
            "severity": severity,
            "message": message,
            "time": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            "target": event.metadata().target(),
        });

        // Source location for Cloud Logging drill-down
        if let (Some(file), Some(line)) = (event.metadata().file(), event.metadata().line()) {
            entry["logging.googleapis.com/sourceLocation"] = serde_json::json!({
                "file": file,
                "line": line,
            });
        }

        // Extra fields from the event
        if let serde_json::Value::Object(ref mut map) = entry {
            for (k, v) in fields {
                map.insert(k, v);
            }
        }

        // Inject OpenTelemetry trace/span IDs so Cloud Logging correlates
        // log entries with Cloud Trace spans.
        let otel_cx = tracing::Span::current().context();
        let otel_span = otel_cx.span();
        let span_cx = otel_span.span_context();
        if span_cx.is_valid() {
            if let serde_json::Value::Object(ref mut map) = entry {
                map.insert(
                    "logging.googleapis.com/trace".into(),
                    format!("projects/{}/traces/{}", self.project_id, span_cx.trace_id()).into(),
                );
                map.insert(
                    "logging.googleapis.com/spanId".into(),
                    span_cx.span_id().to_string().into(),
                );
            }
        }

        write!(
            writer,
            "{}",
            serde_json::to_string(&entry).map_err(|_| std::fmt::Error)?
        )
    }
}

/// Visitor that serialises tracing event fields into a JSON map.
struct JsonFieldVisitor<'a>(&'a mut BTreeMap<String, serde_json::Value>);

impl tracing::field::Visit for JsonFieldVisitor<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().into(), format!("{:?}", value).into());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().into(), serde_json::json!(value));
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().into(), serde_json::json!(value));
    }
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.0.insert(field.name().into(), serde_json::json!(value));
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().into(), serde_json::json!(value));
    }
}
