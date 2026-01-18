use tracing_subscriber::EnvFilter;

use super::config::Config;

pub fn init_tracing(config: &Config) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.tracing_max_level.to_string()));

    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_file(true)
        .with_line_number(true)
        .with_env_filter(filter)
        .init();
}
