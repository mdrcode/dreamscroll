use std::sync::Arc;

use dreamspot::db;

#[rocket::launch]
async fn rocket() -> rocket::Rocket<rocket::Build> {
    let db_config = db::DbConfig::SqliteFile {
        path: "localdev/dreamspot.db".to_string(),
    };

    let db_handle = db::connect(db_config).await.unwrap();
    let db_handle = Arc::new(db_handle);

    db::run_migrations(&db_handle).await.unwrap();

    dreamspot::webui::builder::build_rocket(db_handle)
}
