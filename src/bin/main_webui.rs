#[rocket::launch]
fn rocket() -> rocket::Rocket<rocket::Build> {
    dreamspot::webui::builder::build_rocket()
}
