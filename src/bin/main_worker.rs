#[tokio::main]
async fn main() {
    transitive::worker::main_loop().await;
}
