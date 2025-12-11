use crate::core::{ImageInfo, collect_images};
use rocket::get;
use rocket_dyn_templates::{Template, context};

#[get("/")]
pub fn index() -> Template {
    // Collect all uploaded images with timestamps
    let mut images = collect_images();

    // Sort by timestamp descending (most recent first)
    images.sort_by(|a, b| b.1.cmp(&a.1));

    // Convert to ImageInfo structs for template
    let image_infos: Vec<ImageInfo> = images
        .iter()
        .map(|(filename, datetime)| ImageInfo {
            filename: filename.clone(),
            timestamp: datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        })
        .collect();

    Template::render("index", context! { images: image_infos })
}
