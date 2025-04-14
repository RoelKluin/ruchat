pub(crate) mod args;
pub(crate) mod io;
pub(crate) mod chroma;
pub(crate) mod ollama;
pub(crate) mod subcommand;
pub(crate) mod config;
pub(crate) mod error;
pub(crate) mod embed;

use args::Args;
use clap::Parser;
use error::RuChatError;
use subcommand::handle_request;

pub async fn run() -> Result<(), RuChatError> {
    env_logger::init();
    let args = Args::parse();
    handle_request(&args).await
}
