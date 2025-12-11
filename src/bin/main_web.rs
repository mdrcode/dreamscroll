#[rocket::launch]
fn rocket() -> rocket::Rocket<rocket::Build> {
    dreamspot::web::builder::build_rocket()
}
