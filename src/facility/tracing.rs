use tracing_subscriber::{
    EnvFilter,
    fmt::{format::FmtSpan, time::ChronoLocal},
};

use super::config::Config;

pub fn init_tracing(config: &Config) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.tracing_max_level.to_string()));

    let timer = ChronoLocal::new("%H:%M:%S%.3f".to_string());

    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(filter)
        //.with_span_events(FmtSpan::NEW)
        .with_span_events(FmtSpan::CLOSE)
        .with_target(true)
        .with_timer(timer)
        .init();
}
