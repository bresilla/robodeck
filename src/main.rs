#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("uimap: starting single-binary server");
    robo::run().await
}
