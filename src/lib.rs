pub mod args;
pub mod chat_io;
pub mod chroma;
pub mod config;
pub mod ollama;
pub mod ollama_error;

use args::Args;
use clap::Parser;
use ollama::handle_request;
use ollama_error::Error;

pub async fn run() -> Result<(), Error> {
    env_logger::init();
    handle_request(Args::parse()).await
}
