use sea_orm::DatabaseConnection;

#[derive(Clone)]
pub struct DbHandle {
    pub conn: DatabaseConnection,
}

impl DbHandle {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}
