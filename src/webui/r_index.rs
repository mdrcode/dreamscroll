use crate::common::collect_images;
use rocket::get;
use rocket_dyn_templates::{Template, context};

#[get("/")]
pub fn index() -> Template {
    // Collect all uploaded images with timestamps
    let mut images = collect_images();

    // Sort by timestamp descending (most recent first)
    images.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Template::render("index", context! { images })
}
