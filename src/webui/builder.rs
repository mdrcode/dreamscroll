use crate::db::DbHandle;
use crate::webui::r_detail::detail;
use crate::webui::r_index::index;
use crate::webui::r_upload::upload;
use rocket::{
    fs::{FileServer, relative},
    routes,
};
use rocket_dyn_templates::Template;
use std::{fs, sync::Arc};

pub fn build_rocket(db: Arc<DbHandle>) -> rocket::Rocket<rocket::Build> {
    fs::create_dir_all("uploads").unwrap();
    rocket::build()
        .manage(db)
        .attach(Template::fairing())
        .mount("/", routes![index, detail, upload])
        .mount("/uploads", FileServer::from(relative!("localdev/uploads")))
}
