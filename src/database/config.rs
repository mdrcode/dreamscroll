use sea_orm::DatabaseConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbBackend {
    Sqlite,
    Postgres,
}

#[derive(Clone)]
pub struct DbHandle {
    pub backend: DbBackend,
    pub config: DbConfig,
    pub conn: DatabaseConnection,
}

impl DbHandle {
    pub fn new(conn: DatabaseConnection, config: DbConfig) -> Self {
        Self {
            backend: config.backend(),
            config,
            conn,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DbConfig {
    SqliteFile {
        path: String, // should be "/dir1/file.db" NOT "sqlite://dir1/file.db"
    },
    SqliteMemory,
    Postgres {
        host: String,
        port: u16,
        database: String,
        username: String,
        password: String,
    },
}

impl DbConfig {
    pub fn to_url(&self) -> String {
        match self {
            // ?mode=rwc ensures the database file is created if it doesn't exist
            DbConfig::SqliteFile { path } => format!("sqlite://{}?mode=rwc", path),
            DbConfig::SqliteMemory => "sqlite::memory:".to_string(),
            DbConfig::Postgres {
                host,
                port,
                database,
                username,
                password,
            } => format!(
                "postgres://{}:{}@{}:{}/{}",
                username, password, host, port, database
            ),
        }
    }

    pub fn backend(&self) -> DbBackend {
        match self {
            DbConfig::SqliteFile { .. } => DbBackend::Sqlite,
            DbConfig::SqliteMemory => DbBackend::Sqlite,
            DbConfig::Postgres { .. } => DbBackend::Postgres,
        }
    }
}
