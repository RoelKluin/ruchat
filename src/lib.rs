pub mod args;
pub mod chroma;
pub mod config;
pub mod ollama;

use anyhow::{anyhow, Result};
use args::Args;
use clap::Parser;
use ollama::handle_request;

pub async fn run() -> Result<()> {
    env_logger::init();
    handle_request(Args::parse())
        .await
        .map_err(|e| anyhow!("{e}"))
}
