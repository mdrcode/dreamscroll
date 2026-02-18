use sea_orm;

use crate::{auth, facility};

use super::*;

pub async fn connect(
    config: &facility::Config,
) -> anyhow::Result<(sea_orm::DatabaseConnection, auth::SessionStoreWrapper)> {
    let db_tuple = match config.db_backend {
        DbBackend::Sqlite => {
            let url = config
                .db_url_sqlite
                .as_ref()
                .expect("DB Backend is sqlite but no url");
            let pool = create_sqlite_pool(url).await?;
            let db_connection = connect_sqlite_db(pool.clone()).await?;
            let session_store = connect_sqlite_session_store(pool.clone()).await?;
            (db_connection, session_store)
        }
        DbBackend::Postgres => {
            let url = config
                .db_url_postgres
                .as_ref()
                .expect("DB Backend is postgres but no url");
            let pool = create_postgres_pool(url).await?;
            let db_connection = connect_postgres_db(pool.clone()).await?;
            let session_store = connect_postgres_session_store(pool.clone()).await?;
            (db_connection, session_store)
        }
    };

    tracing::info!(
        "Successfully connected to database backend: {:?}",
        config.db_backend
    );

    Ok(db_tuple)
}
