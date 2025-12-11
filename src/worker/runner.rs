use crate::core::collect_images;

pub async fn main_loop() {
    println!("Worker process starting...");

    // Example: Process images periodically
    loop {
        let images = collect_images();
        println!("Found {} images to process", images.len());

        // TODO: Add your worker logic here
        // - Process images
        // - Call external APIs
        // - Update database/storage with results

        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }
}
