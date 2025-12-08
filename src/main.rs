use chrono::{DateTime, Utc};
use rocket::{fs::FileServer, get, http::ContentType, launch, post, routes};
use rocket_multipart_form_data::{
    MultipartFormData, MultipartFormDataField, MultipartFormDataOptions,
};
use std::{fs, path::Path};
use uuid::Uuid;

#[launch]
fn rocket() -> rocket::Rocket<rocket::Build> {
    fs::create_dir_all("uploads").unwrap();
    rocket::build()
        .mount("/", routes![index, upload])
        .mount("/uploads", FileServer::from("uploads"))
}

#[get("/")]
fn index() -> (ContentType, String) {
    // Collect all uploaded images with timestamps
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

    // Sort by timestamp descending (most recent first)
    images.sort_by(|a, b| b.1.cmp(&a.1));

    // Generate HTML for timeline
    let mut images_html = String::new();
    for (filename, datetime) in images {
        images_html.push_str(&format!(
            r#"
            <div style="margin-bottom: 20px; border: 1px solid #ccc; padding: 10px;">
                <img src="/uploads/{}" alt="Uploaded Image" style="max-width: 40%; height: auto; display: block; margin: 0 auto;">
                <p style="text-align: center; margin-top: 10px;">Uploaded at: {}</p>
            </div>
            "#,
            filename,
            datetime.format("%Y-%m-%d %H:%M:%S UTC")
        ));
    }

    (
        ContentType::HTML,
        format!(
            r#"
    <!DOCTYPE html>
    <html>
    <head>
        <title>Image Upload</title>
    </head>
    <body>
        <h1>Upload an Image for AI Analysis</h1>
        <form action="/upload" method="post" enctype="multipart/form-data">
            <input type="file" name="image" accept="image/*" required>
            <br><br>
            <input type="submit" value="Upload and Analyze">
        </form>
        <h2>Recent Uploads</h2>
        {}
    </body>
    </html>
    "#,
            images_html
        ),
    )
}

#[post("/upload", data = "<data>")]
async fn upload(
    content_type: &ContentType,
    data: rocket::data::Data<'_>,
) -> Result<(ContentType, String), rocket::http::Status> {
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

    // Collect all uploaded images with timestamps
    let mut images = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir("uploads").await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(metadata) = entry.metadata().await {
                if let Ok(mtime) = metadata.modified() {
                    let datetime: DateTime<Utc> = mtime.into();
                    let filename = entry.file_name().to_string_lossy().to_string();
                    images.push((filename, datetime));
                }
            }
        }
    }

    // Sort by timestamp descending (most recent first)
    images.sort_by(|a, b| b.1.cmp(&a.1));

    // Generate HTML for timeline
    let mut images_html = String::new();
    for (filename, datetime) in images {
        images_html.push_str(&format!(
            r#"
            <div style="margin-bottom: 20px; border: 1px solid #ccc; padding: 10px;">
                <img src="/uploads/{}" alt="Uploaded Image" style="max-width: 40%; height: auto; display: block; margin: 0 auto;">
                <p style="text-align: center; margin-top: 10px;">Uploaded at: {}</p>
            </div>
            "#,
            filename,
            datetime.format("%Y-%m-%d %H:%M:%S UTC")
        ));
    }

    Ok((
        ContentType::HTML,
        format!(
            r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>Image Timeline</title>
        </head>
        <body>
            <h1>Image Timeline</h1>
            <p><a href="/">Upload another image</a></p>
            {}
        </body>
        </html>
        "#,
            images_html
        ),
    ))
}
