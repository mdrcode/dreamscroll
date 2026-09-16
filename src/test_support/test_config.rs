use crate::config;

pub fn load() -> anyhow::Result<config::Config> {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(config::load_local_files);

    config::make()
}
