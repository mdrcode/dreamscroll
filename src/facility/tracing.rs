use super::cloud_logging_format::CloudLoggingFormat;
use axum::http;
use opentelemetry::trace::TraceContextExt;
use opentelemetry_gcloud_trace::GcpCloudTraceExporterBuilder;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tower_http::trace::TraceLayer;
use tracing_opentelemetry::{OpenTelemetryLayer, OpenTelemetrySpanExt};
use tracing_subscriber::{
    EnvFilter, Registry,
    fmt::{self, format::FmtSpan, time::ChronoLocal},
    layer::SubscriberExt,
};

pub fn init_tracing_local() {
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

pub async fn init_tracing_gcloud(project_id: String) -> anyhow::Result<SdkTracerProvider> {
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
    let telemetry_layer = OpenTelemetryLayer::new(tracer); // spans → Cloud Trace

    // Cloud Logging JSON formatter that reads OTel trace context directly
    let cloud_logging_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .event_format(CloudLoggingFormat { project_id });

    let subscriber = Registry::default()
        .with(EnvFilter::from_default_env())
        .with(telemetry_layer) // tracing spans → Cloud Trace
        .with(cloud_logging_layer); // events → Cloud Logging (with traceId/spanId)

    tracing::subscriber::set_global_default(subscriber)?;

    return Ok(provider);
}

struct AxumHeaderExtractor<'a>(&'a http::HeaderMap);

impl opentelemetry::propagation::Extractor for AxumHeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

pub fn add_axum_trace_propagation(router: axum::Router) -> axum::Router {
    router.layer(
        TraceLayer::new_for_http().make_span_with(|request: &http::Request<_>| {
            // Extract the W3C traceparent header injected by Cloud Run so that
            // our spans are children of the infrastructure-level request traces
            let parent_cx = opentelemetry::global::get_text_map_propagator(|prop| {
                prop.extract(&AxumHeaderExtractor(request.headers()))
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
