use dreamspot::db::{DbConfig, setup_database};

#[rocket::launch]
async fn rocket() -> rocket::Rocket<rocket::Build> {
    let db_config = DbConfig::SqliteFile {
        path: "localdev/dreamspot.db".to_string(),
    };

    let db_ctx = setup_database(db_config)
        .await
        .expect("Failed to initialize database");

    dreamspot::webui::builder::build_rocket(db_ctx)
}
