use opentelemetry::trace::TraceContextExt;
use std::collections::BTreeMap;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{
    fmt::{self, FormatEvent, FormatFields},
    registry::LookupSpan,
};

/// Cloud Logging (Stackdriver) compatible JSON event formatter.
///
/// Hand-rolled because the off-the-shelf `tracing-stackdriver` crate pins an
/// ancient OpenTelemetry 0.22 while the rest of our stack (and the
/// `opentelemetry-gcloud-trace` exporter) are on 0.31+ — pulling it in would
/// silently break log↔trace correlation. So we implement the ~20% we need:
/// JSON shape, GCP severity vocabulary, and the `logging.googleapis.com/*`
/// correlation keys.
pub(crate) struct GCloudLoggingFormat {
    pub project_id: String,
}

/// Map `tracing`'s level vocabulary onto Cloud Logging's `LogSeverity` strings.
///
/// Two deliberate folds: `tracing::Level::WARN` → `"WARNING"` (Cloud Logging
/// spells it with the "ING"), and `TRACE` → `"DEBUG"` (Cloud Logging has no
/// `TRACE`; an unknown value would be downgraded to `DEFAULT`, hiding the logs).
fn severity_to_gcp(level: tracing::Level) -> &'static str {
    match level {
        tracing::Level::ERROR => "ERROR",
        tracing::Level::WARN => "WARNING",
        tracing::Level::INFO => "INFO",
        tracing::Level::DEBUG => "DEBUG",
        tracing::Level::TRACE => "DEBUG",
    }
}

impl<S, N> FormatEvent<S, N> for GCloudLoggingFormat
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
        let severity = severity_to_gcp(*event.metadata().level());

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
        if span_cx.is_valid()
            && let serde_json::Value::Object(ref mut map) = entry
        {
            map.insert(
                "logging.googleapis.com/trace".into(),
                format!("projects/{}/traces/{}", self.project_id, span_cx.trace_id()).into(),
            );
            map.insert(
                "logging.googleapis.com/spanId".into(),
                span_cx.span_id().to_string().into(),
            );
        }

        writeln!(
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

#[cfg(test)]
mod tests {
    use super::severity_to_gcp;
    use tracing::Level;

    #[test]
    fn severity_maps_each_tracing_level() {
        assert_eq!(severity_to_gcp(Level::ERROR), "ERROR");
        assert_eq!(severity_to_gcp(Level::WARN), "WARNING");
        assert_eq!(severity_to_gcp(Level::INFO), "INFO");
        assert_eq!(severity_to_gcp(Level::DEBUG), "DEBUG");
    }

    #[test]
    fn severity_folds_trace_into_debug() {
        // Cloud Logging has no TRACE severity; emitting an unknown value would
        // be downgraded to DEFAULT, hiding the logs beneath DEBUG.
        assert_eq!(severity_to_gcp(Level::TRACE), "DEBUG");
    }

    #[test]
    fn severity_warn_uses_cloud_logging_spelling() {
        // Cloud Logging spells it "WARNING"; `tracing` spells it "WARN".
        assert_eq!(severity_to_gcp(Level::WARN), "WARNING");
    }
}
