use anyhow::Context;
use axum::http;
use opentelemetry::trace::TraceContextExt;
use opentelemetry_gcloud_trace::GcpCloudTraceExporterBuilder;
use tower_http::trace::TraceLayer;
use tracing_opentelemetry::{OpenTelemetryLayer, OpenTelemetrySpanExt};
use tracing_stackdriver::{CloudTraceConfiguration, layer};
use tracing_subscriber::{
    EnvFilter, Registry,
    fmt::{format::FmtSpan, time::ChronoLocal},
    layer::SubscriberExt,
};

pub async fn init_tracing() -> anyhow::Result<()> {
    if std::env::var("K_SERVICE").is_ok() {
        // Running within Cloud Run, so enable Cloud Trace and Stackdriver
        // for integrated tracing + logging with trace/span IDs.

        // Extract project_id manually because config is not available yet
        let project_id = std::env::var("GCLOUD_PROJECT_ID")
            .context("GCLOUD_PROJECT_ID env var required but not set")?;

        // Register the W3C traceparent propagator so incoming Cloud Run request
        // headers can be extracted and used as span parents.
        opentelemetry::global::set_text_map_propagator(
            opentelemetry_sdk::propagation::TraceContextPropagator::new(),
        );

        // 1. Cloud Trace exporter
        let exporter = GcpCloudTraceExporterBuilder::new(project_id.clone());
        let provider = exporter.create_provider().await?;
        let tracer = exporter.install(&provider).await?;
        opentelemetry::global::set_tracer_provider(provider.clone());

        // 2. Layers
        let telemetry_layer = OpenTelemetryLayer::new(tracer); // spans to Cloud Trace

        let stackdriver_layer = layer()
            .with_writer(std::io::stderr)
            .with_cloud_trace(CloudTraceConfiguration { project_id });

        let subscriber = Registry::default()
            .with(EnvFilter::from_default_env())
            .with(telemetry_layer) // tracing spans → Cloud Trace
            .with(stackdriver_layer); // events + spans → Cloud Logging (with traceId/spanId)

        tracing::subscriber::set_global_default(subscriber)?;
    } else {
        // Local dev: compact, human-readable
        let timer = ChronoLocal::new("%H:%M:%S%.3f".to_string());

        tracing_subscriber::fmt()
            .compact()
            .with_env_filter(EnvFilter::from_default_env())
            .with_span_events(FmtSpan::CLOSE)
            .with_target(true)
            .with_timer(timer)
            .init();

        tracing::info!("Initialized tracing for local development.");
    }

    Ok(())
}

struct HeaderExtractor<'a>(&'a http::HeaderMap);

impl opentelemetry::propagation::Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

pub fn add_trace_propagation_layer(router: axum::Router) -> axum::Router {
    router.layer(
        TraceLayer::new_for_http().make_span_with(|request: &http::Request<_>| {
            // Extract the W3C traceparent header injected by Cloud Run so that
            // our spans are children of the infrastructure-level request trace.
            let parent_cx = opentelemetry::global::get_text_map_propagator(|prop| {
                prop.extract(&HeaderExtractor(request.headers()))
            });
            let span = tracing::info_span!("http_request");

            let _ = span.set_parent(parent_cx);
            span
        }),
    )
}

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
