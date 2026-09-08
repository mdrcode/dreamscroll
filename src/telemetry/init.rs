use opentelemetry_gcloud_trace::GcpCloudTraceExporterBuilder;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt};

use super::gcloud_logging_format::GCloudLoggingFormat;

/// Local dev: compact human-readable output (no OTel, no GCP).
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

/// Cloud Run: two subscriber layers — spans (→ Cloud Trace) and events
/// (→ Cloud Logging JSON, correlated via the trace/span IDs from the OTel context).
pub async fn init_gcloud(project_id: String) -> anyhow::Result<SdkTracerProvider> {
    // Register the W3C traceparent propagator so incoming Cloud Run request
    // headers can be extracted and used as span parents.
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    // Build a tracer provider whose span processor pushes spans to the Cloud
    // Trace exporter (this is where the real exporter lives).
    let builder = GcpCloudTraceExporterBuilder::new(project_id.clone());
    let provider = builder.create_provider().await?;
    opentelemetry::global::set_tracer_provider(provider.clone());

    // Borrow a Tracer from that provider (with an instrumentation scope) and
    // hand it to the OTel layer, which mirrors each `tracing` span into OTel.
    let tracer = builder.install(&provider).await?;
    let span_layer = OpenTelemetryLayer::new(tracer);

    // Cloud Logging JSON formatter that reads OTel trace context directly
    let event_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .event_format(GCloudLoggingFormat { project_id });

    let subscriber = Registry::default()
        .with(EnvFilter::from_default_env())
        .with(span_layer) // spans → Cloud Trace
        .with(event_layer); // events → Cloud Logging

    tracing::subscriber::set_global_default(subscriber)?;

    Ok(provider)
}
