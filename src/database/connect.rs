use sea_orm;

use crate::auth;

use super::*;

pub async fn connect(
    config: &crate::facility::Config,
) -> anyhow::Result<(sea_orm::DatabaseConnection, auth::SessionStoreWrapper)> {
    match config.db_backend.as_str() {
        "sqlite" => {
            let pool = create_sqlite_pool(&config.db_sqlite_url).await?;
            let db_connection = connect_sqlite_db(pool.clone()).await?;
            let session_store = connect_sqlite_session_store(pool.clone()).await?;
            Ok((db_connection, session_store))
        }
        "postgres" => {
            let pool = create_postgres_pool(&config.db_postgres_url).await?;
            let db_connection = connect_postgres_db(pool.clone()).await?;
            let session_store = connect_postgres_session_store(pool.clone()).await?;
            Ok((db_connection, session_store))
        }
        _ => {
            unimplemented!("Unsupported database backend: {}", config.db_backend);
        }
    }
}
