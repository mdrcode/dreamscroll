// Unfortunately, tower_sessions_sqlx_store does not provide a unified trait
// for both SqliteStore and PostgresStore, so we wrap them in an enum to
// provide a unified interface.

#[derive(Debug, Clone)]
pub enum SessionStoreWrapper {
    Sqlite(tower_sessions_sqlx_store::SqliteStore),
    Postgres(tower_sessions_sqlx_store::PostgresStore),
}

#[async_trait::async_trait]
impl tower_sessions::SessionStore for SessionStoreWrapper {
    async fn create(
        &self,
        record: &mut tower_sessions::session::Record,
    ) -> tower_sessions::session_store::Result<()> {
        match self {
            Self::Sqlite(store) => store.create(record).await,
            Self::Postgres(store) => store.create(record).await,
        }
    }

    async fn save(
        &self,
        record: &tower_sessions::session::Record,
    ) -> tower_sessions::session_store::Result<()> {
        match self {
            Self::Sqlite(store) => store.save(record).await,
            Self::Postgres(store) => store.save(record).await,
        }
    }

    async fn load(
        &self,
        session_id: &tower_sessions::session::Id,
    ) -> tower_sessions::session_store::Result<Option<tower_sessions::session::Record>> {
        match self {
            Self::Sqlite(store) => store.load(session_id).await,
            Self::Postgres(store) => store.load(session_id).await,
        }
    }

    async fn delete(
        &self,
        session_id: &tower_sessions::session::Id,
    ) -> tower_sessions::session_store::Result<()> {
        match self {
            Self::Sqlite(store) => store.delete(session_id).await,
            Self::Postgres(store) => store.delete(session_id).await,
        }
    }
}
