mod first_user;
pub use first_user::check_first_user;

mod check_users;
pub use check_users::check_users;

mod postgres;
pub use postgres::{connect, connect_postgres_db, make_url_from_config};

mod sync_schema;
pub use sync_schema::{
    REQUIRED_SCHEMA_VERSION, check_schema_version, sync_schema, sync_schema_dry_run,
};

mod status;
pub use status::{Status, status};

mod indexes;

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
