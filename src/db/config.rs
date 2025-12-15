use sea_orm::DatabaseConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone)]
pub enum DatabaseConfig {
    SqliteFile {
        path: String, // should file.db NOT sqlite://file.db
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

impl DatabaseConfig {
    /// Convert config to database URL string
    pub fn to_url(&self) -> String {
        match self {
            DatabaseConfig::SqliteFile { path } => format!("sqlite://{}", path),
            DatabaseConfig::SqliteMemory => "sqlite::memory:".to_string(),
            DatabaseConfig::Postgres {
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

    /// Get the database backend type
    pub fn backend(&self) -> DatabaseBackend {
        match self {
            DatabaseConfig::SqliteFile { .. } | DatabaseConfig::SqliteMemory => {
                DatabaseBackend::Sqlite
            }
            DatabaseConfig::Postgres { .. } => DatabaseBackend::Postgres,
        }
    }
}

/// Database context holding connection and config
pub struct DbContext {
    pub conn: DatabaseConnection,
    pub config: DatabaseConfig,
}

impl DbContext {
    pub fn new(conn: DatabaseConnection, config: DatabaseConfig) -> Self {
        Self { conn, config }
    }

    pub fn backend(&self) -> DatabaseBackend {
        self.config.backend()
    }
}
