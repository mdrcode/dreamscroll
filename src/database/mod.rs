mod first_user;
pub use first_user::check_first_user;

mod check_users;
pub use check_users::check_users;

mod postgres;
pub use postgres::{connect, make_url_from_config};

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
