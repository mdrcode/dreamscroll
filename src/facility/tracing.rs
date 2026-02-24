use anyhow::Context;
use opentelemetry_gcloud_trace::GcpCloudTraceExporterBuilder;
use tracing_opentelemetry::OpenTelemetryLayer;
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
        let project_id =
            std::env::var("PROJECT_ID").context("PROJECT_ID env var required but not set")?;

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
