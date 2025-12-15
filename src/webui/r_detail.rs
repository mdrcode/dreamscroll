use crate::common::collect_images;
use rocket::get;
use rocket_dyn_templates::{Template, context};

#[get("/detail/<filename>")]
pub fn detail(filename: String) -> Option<Template> {
    // Find the image with the matching filename
    let images = collect_images();
    let image = images.into_iter().find(|img| img.filename == filename)?;

    Some(Template::render("detail", context! { image }))
}
