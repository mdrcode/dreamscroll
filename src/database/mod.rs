mod first_user;
pub use first_user::check_first_user;

mod check_users;
pub use check_users::check_users;

mod postgres;
pub use postgres::{connect, make_url_from_config};

mod task_run_indexes;
pub use task_run_indexes::sync_task_run_indexes;

// Unclear if this is needed? Just a wrapper for now

#[derive(Clone)]
pub struct DbHandle {
    pub conn: sea_orm::DatabaseConnection,
}

impl DbHandle {
    pub fn new(conn: sea_orm::DatabaseConnection) -> Self {
        Self { conn }
    }
}
