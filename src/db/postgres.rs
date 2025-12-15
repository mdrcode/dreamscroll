use sea_orm::{ConnectionTrait, DbErr, Statement};

pub async fn run_migrations(db: &impl ConnectionTrait) -> Result<(), DbErr> {
    // Create captures table
    db.execute(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        CREATE TABLE IF NOT EXISTS captures (
            id SERIAL PRIMARY KEY,
            uuid TEXT UNIQUE NOT NULL,
            created_at TIMESTAMP NOT NULL
        )
        "#
        .to_string(),
    ))
    .await?;

    // Create media table
    db.execute(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        CREATE TABLE IF NOT EXISTS media (
            id SERIAL PRIMARY KEY,
            filename TEXT NOT NULL,
            capture_id INTEGER REFERENCES captures(id)
        )
        "#
        .to_string(),
    ))
    .await?;

    Ok(())
}
