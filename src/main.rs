
/// main function for RuChat
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    ruchat::run().await.map_err(|e| {
        eprintln!("Application error: {}", e);
        e
    })?;
    Ok(())
}
