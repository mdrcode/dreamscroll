use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

use super::config::Config;

pub fn init_tracing(config: &Config) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.tracing_max_level.to_string()));

    tracing_subscriber::fmt()
        .compact()
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_env_filter(filter)
        .init();
}
