/// main function for RuChat
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Err(e) = ruchat::run().await {
        eprintln!("Error: {}", e);
    }
    Ok(())
}
