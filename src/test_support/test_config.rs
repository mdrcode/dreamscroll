use crate::config;

pub fn load() -> anyhow::Result<config::Config> {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(config::populate_env_if_test_or_dev);

    config::make()
}
