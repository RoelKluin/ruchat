pub mod args;
pub mod chroma;
pub mod config;
pub mod ollama;

use args::{Args, Commands};
use clap::Parser;
use ollama::{handle_request, Error};

pub async fn run() -> Result<(), Error> {
    env_logger::init();
    let args = Args::parse();
    match &args.command {
        Commands::Query(query_args) => handle_request(Args::parse(), query_args).await,
    }
}
