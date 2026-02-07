use sea_orm;

use crate::{auth, facility};

use super::*;

pub async fn connect(
    config: &facility::Config,
) -> anyhow::Result<(sea_orm::DatabaseConnection, auth::SessionStoreWrapper)> {
    match config.db_backend {
        DbBackend::Sqlite => {
            let pool = create_sqlite_pool(&config.db_url_sqlite).await?;
            let db_connection = connect_sqlite_db(pool.clone()).await?;
            let session_store = connect_sqlite_session_store(pool.clone()).await?;
            Ok((db_connection, session_store))
        }
        DbBackend::Postgres => {
            let url = config
                .db_url_postgres
                .as_ref()
                .expect("DB Backend is postgres but no url");
            let pool = create_postgres_pool(url).await?;
            let db_connection = connect_postgres_db(pool.clone()).await?;
            let session_store = connect_postgres_session_store(pool.clone()).await?;
            Ok((db_connection, session_store))
        }
    }
}
