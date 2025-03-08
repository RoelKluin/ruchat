pub mod args;
pub mod chat_io;
pub mod chroma;
pub mod config;
pub mod error;
pub mod ollama;
pub mod ollama_ask;
pub mod ollama_chat;
pub mod ollama_embed;
pub mod ollama_func;
pub mod ollama_func_struct;

use args::Args;
use clap::Parser;
use error::RuChatError;
use ollama::handle_request;

pub async fn run() -> Result<(), RuChatError> {
    env_logger::init();
    let args = Args::parse();
    handle_request(&args).await
}
