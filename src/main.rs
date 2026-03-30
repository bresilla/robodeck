#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("uimap: starting robo backend");
    println!("uimap: deck is the wasm frontend workspace member under tools/uimap/deck");
    robo::run().await
}
