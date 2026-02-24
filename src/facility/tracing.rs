use opentelemetry_gcloud_trace::GcpCloudTraceExporterBuilder;
use opentelemetry_sdk::trace::Tracer;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_stackdriver::{CloudTraceConfiguration, layer};
use tracing_subscriber::{
    EnvFilter, Registry,
    fmt::{format::FmtSpan, time::ChronoLocal},
    layer::SubscriberExt,
};

pub async fn init_tracing() -> anyhow::Result<()> {
    let env_filter = EnvFilter::from_default_env();

    if std::env::var("K_SERVICE").is_ok() {
        let project_id = "mdrcode".to_string(); // TODO env var

        // 1. Cloud Trace exporter
        let exporter = GcpCloudTraceExporterBuilder::new(project_id.clone());
        let provider = exporter.create_provider().await?;
        let tracer = exporter.install(&provider).await?;
        opentelemetry::global::set_tracer_provider(provider.clone());

        // 2. Layers
        let telemetry_layer = OpenTelemetryLayer::new(tracer); // sends spans to Cloud Trace

        let stackdriver_layer = layer().with_cloud_trace(CloudTraceConfiguration {
            project_id: project_id.clone(),
        });

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
            .with_env_filter(env_filter)
            .with_span_events(FmtSpan::CLOSE)
            .with_target(true)
            .with_timer(timer)
            .init();

        tracing::info!("Initialized tracing for local development.");
    }

    Ok(())
}
