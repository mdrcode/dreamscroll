#[rocket::launch]
fn rocket() -> rocket::Rocket<rocket::Build> {
    transitive::web::build_rocket()
}
