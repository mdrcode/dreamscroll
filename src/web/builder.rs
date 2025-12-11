use crate::web::r_index::index;
use crate::web::r_upload::upload;
use rocket::{fs::FileServer, routes};
use rocket_dyn_templates::Template;
use std::fs;

pub fn build_rocket() -> rocket::Rocket<rocket::Build> {
    fs::create_dir_all("uploads").unwrap();
    rocket::build()
        .attach(Template::fairing())
        .mount("/", routes![index, upload])
        .mount("/uploads", FileServer::from("uploads"))
}
