use opentelemetry_gcloud_trace::GcpCloudTraceExporterBuilder;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt};

use super::gcloud_logging_format::GCloudLoggingFormat;

pub fn init_local() {
    use tracing_subscriber::fmt::{format::FmtSpan, time::ChronoLocal};

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

pub async fn init_gcloud(project_id: String) -> anyhow::Result<SdkTracerProvider> {
    // Register the W3C traceparent propagator so incoming Cloud Run request
    // headers can be extracted and used as span parents.
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    // 1. Cloud Trace exporter
    let exporter = GcpCloudTraceExporterBuilder::new(project_id.clone());
    let provider = exporter.create_provider().await?;
    opentelemetry::global::set_tracer_provider(provider.clone());

    // 2. Layers
    let tracer = exporter.install(&provider).await?;
    let telemetry_layer = OpenTelemetryLayer::new(tracer); // spans → Cloud Trace

    // Cloud Logging JSON formatter that reads OTel trace context directly
    let cloud_logging_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .event_format(GCloudLoggingFormat { project_id });

    let subscriber = Registry::default()
        .with(EnvFilter::from_default_env())
        .with(telemetry_layer) // tracing spans → Cloud Trace
        .with(cloud_logging_layer); // events → Cloud Logging

    tracing::subscriber::set_global_default(subscriber)?;

    Ok(provider)
}
