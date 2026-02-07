mod connect;
pub use connect::*;

mod connect_sqlite;
pub use connect_sqlite::*;

mod connect_postgres;
pub use connect_postgres::*;

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
