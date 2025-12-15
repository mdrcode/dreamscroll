use sea_orm::{ConnectionTrait, DbErr, Statement};

pub async fn run_migrations(db: &impl ConnectionTrait) -> Result<(), DbErr> {
    // Create captures table
    db.execute(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        r#"
        CREATE TABLE IF NOT EXISTS captures (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uuid TEXT UNIQUE NOT NULL,
            created_at TEXT NOT NULL
        )
        "#
        .to_string(),
    ))
    .await?;

    // Create media table
    db.execute(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        r#"
        CREATE TABLE IF NOT EXISTS media (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            filename TEXT NOT NULL,
            capture_id INTEGER,
            FOREIGN KEY (capture_id) REFERENCES captures(id)
        )
        "#
        .to_string(),
    ))
    .await?;

    Ok(())
}
