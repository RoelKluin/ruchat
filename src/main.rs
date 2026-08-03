/// main function for RuChat
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // RUCHAT_LOG_FORMAT=json switches to newline-delimited JSON events (for
    // log aggregators); anything else, including unset, keeps the default
    // human-readable format. Level filtering (RUST_LOG) works the same in
    // either case.
    if std::env::var("RUCHAT_LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }

    ruchat::run().await.map_err(|e| {
        eprintln!("Application error:\n{e}");
        e
    })?;
    Ok(())
}
