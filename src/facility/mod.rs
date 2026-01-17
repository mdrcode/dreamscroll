use tracing_subscriber::EnvFilter;

mod config;
pub use config::{Config, make_config};

pub enum Env {
    LocalDev,
    Production,
}

pub fn init_tracing(config: &Config) {
    tracing_subscriber::fmt()
        .pretty()
        .without_time()
        .with_max_level(config.tracing_max_level)
        .with_env_filter(EnvFilter::from_default_env())
        .init();
}
