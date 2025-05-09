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

/// Runs the RuChat application.
///
/// This function initializes the environment logger, parses command-line
/// arguments, and handles the request based on the provided arguments.
///
/// # Returns
///
/// This function returns a `Result` indicating success or failure. On success,
/// it returns `Ok(())`. On failure, it returns an `Err` containing a `RuChatError`.
///
/// # Errors
///
/// This function will return an error if the command-line arguments cannot be
/// parsed or if handling the request fails.
pub async fn run() -> Result<(), RuChatError> {
    env_logger::init();
    let args = Args::parse();
    handle_request(&args).await
}
