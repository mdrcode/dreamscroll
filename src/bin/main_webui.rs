use dreamspot::db::{DbConfig, db_connect, db_prepare, db_run_migrations};

#[rocket::launch]
async fn rocket() -> rocket::Rocket<rocket::Build> {
    let db_config = DbConfig::SqliteFile {
        path: "localdev/dreamspot.db".to_string(),
    };

    db_prepare(&db_config).await.unwrap();

    let db_context = db_connect(db_config).await.unwrap();

    db_run_migrations(&db_context).await.unwrap();

    dreamspot::webui::builder::build_rocket(db_context)
}
