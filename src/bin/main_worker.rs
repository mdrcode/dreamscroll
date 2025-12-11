#[tokio::main]
async fn main() {
    dreamspot::worker::main_loop().await;
}
