use sea_orm::DatabaseConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbBackend {
    Sqlite,
    Postgres,
}

pub struct DbContext {
    pub conn: DatabaseConnection,
    pub config: DbConfig,
}

impl DbContext {
    pub fn new(conn: DatabaseConnection, config: DbConfig) -> Self {
        Self { conn, config }
    }

    pub fn backend(&self) -> DbBackend {
        self.config.backend()
    }
}

#[derive(Debug, Clone)]
pub enum DbConfig {
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

impl DbConfig {
    /// Convert config to database URL string
    pub fn to_url(&self) -> String {
        match self {
            DbConfig::SqliteFile { path } => format!("sqlite://{}", path),
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

    /// Get the database backend type
    pub fn backend(&self) -> DbBackend {
        match self {
            DbConfig::SqliteFile { .. } | DbConfig::SqliteMemory => DbBackend::Sqlite,
            DbConfig::Postgres { .. } => DbBackend::Postgres,
        }
    }
}
