use rocket::{http::ContentType, post, response::Redirect};
use rocket_multipart_form_data::{
    MultipartFormData, MultipartFormDataField, MultipartFormDataOptions,
};
use std::path::Path;
use uuid::Uuid;

#[post("/upload", data = "<data>")]
pub async fn upload(
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
