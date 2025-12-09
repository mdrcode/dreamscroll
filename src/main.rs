use chrono::{DateTime, Utc};
use rocket::{fs::FileServer, get, http::ContentType, launch, post, response::Redirect, routes};
use rocket_dyn_templates::{Template, context};
use rocket_multipart_form_data::{
    MultipartFormData, MultipartFormDataField, MultipartFormDataOptions,
};
use serde::Serialize;
use std::{fs, path::Path};
use uuid::Uuid;

#[derive(Serialize)]
struct ImageInfo {
    filename: String,
    timestamp: String,
}

fn collect_images() -> Vec<(String, DateTime<Utc>)> {
    let mut images = Vec::new();
    if let Ok(mut entries) = std::fs::read_dir("uploads") {
        while let Some(entry_result) = entries.next() {
            if let Ok(entry) = entry_result {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(mtime) = metadata.modified() {
                        let datetime: DateTime<Utc> = mtime.into();
                        let filename = entry.file_name().to_string_lossy().to_string();
                        images.push((filename, datetime));
                    }
                }
            }
        }
    }
    images
}

#[launch]
fn rocket() -> rocket::Rocket<rocket::Build> {
    fs::create_dir_all("uploads").unwrap();
    rocket::build()
        .attach(Template::fairing())
        .mount("/", routes![index, upload])
        .mount("/uploads", FileServer::from("uploads"))
}

#[get("/")]
fn index() -> Template {
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

#[post("/upload", data = "<data>")]
async fn upload(
    content_type: &ContentType,
    data: rocket::data::Data<'_>,
) -> Result<Redirect, rocket::http::Status> {
    let options = MultipartFormDataOptions::with_multipart_form_data_fields(vec![
        MultipartFormDataField::file("image").size_limit(10 * 1024 * 1024), // 10MB limit
    ]);

    let multipart_form_data = match MultipartFormData::parse(content_type, data, options).await {
        Ok(data) => data,
        Err(_) => return Err(rocket::http::Status::BadRequest),
    };

    let image_field = match multipart_form_data.files.get("image") {
        Some(field) => field,
        None => return Err(rocket::http::Status::BadRequest),
    };

    let file = match image_field.get(0) {
        Some(file) => file,
        None => return Err(rocket::http::Status::BadRequest),
    };

    // Generate unique filename
    let filename = format!("{}.jpg", Uuid::new_v4());
    let upload_path = Path::new("uploads").join(&filename);

    // Create uploads directory if it doesn't exist
    if let Err(_) = tokio::fs::create_dir_all("uploads").await {
        return Err(rocket::http::Status::InternalServerError);
    }

    // Copy the file to persistent storage
    if let Err(_) = tokio::fs::copy(&file.path, &upload_path).await {
        return Err(rocket::http::Status::InternalServerError);
    }

    // Redirect to home page to show the timeline
    Ok(Redirect::to("/"))
}
