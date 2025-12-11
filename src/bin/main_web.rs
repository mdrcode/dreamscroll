use rocket::{fs::FileServer, launch, routes};
use rocket_dyn_templates::Template;
use std::fs;
use transitive::web::index::index;
use transitive::web::upload::upload;

#[launch]
fn rocket() -> rocket::Rocket<rocket::Build> {
    fs::create_dir_all("uploads").unwrap();
    rocket::build()
        .attach(Template::fairing())
        .mount("/", routes![index, upload])
        .mount("/uploads", FileServer::from("uploads"))
}
