use serde::Deserialize;

#[derive(Default, Deserialize)]
pub struct PostgresConfig {
    pub user: String,
    pub password: String,
    pub host_port: String, // e.g. "localhost:5432" or "db:5432"
    pub db: String,

    pub connection_params: Option<String>, // e.g. "sslmode=require"
}
