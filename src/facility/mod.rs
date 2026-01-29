mod check_first_users;
pub use check_first_users::check_first_users;

mod config;
pub use config::{Config, make_config};

mod tracing;
pub use tracing::init_tracing;

pub enum Env {
    LocalDev,
    Production,
}
