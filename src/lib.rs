pub mod agent;
pub(crate) mod arg_utils;
pub mod args;
pub mod chroma;
pub(crate) mod config;
pub mod embed;
pub mod error;
pub(crate) mod io;
pub mod ollama;
pub(crate) mod options;

use args::Args;
use clap::Parser;
use error::RuChatError;

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
    args.handle_request().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Commands;
    use crate::ollama::ask::AskArgs;
    use args::Args;

    #[tokio::test]
    async fn test_server_run_success() {
        // Create a mock Args instance
        let args = Args {
            command: Some(Commands::Ask(AskArgs {
                model: "qwen2.5-coder:14b".to_string(),
                prompt: Some("Testing, please ignore".to_string()),
                output_format: "text".to_string(),
                text_files: None,
                options: None,
                positional_prompt: None,
            })),
            verbose: true,
            server: "localhost".to_string() + ":8080",
        };
        eprintln!("If this errors, your server may also be down.");
        assert!(args.handle_request().await.is_ok());
    }

    #[tokio::test]
    async fn test_no_model_failure() {
        // Create a mock Args instance
        let args = Args {
            command: Some(Commands::Ask(AskArgs {
                model: "NO_MODEL".to_string(),
                prompt: Some("Testing, please ignore".to_string()),
                output_format: "text".to_string(),
                text_files: None,
                options: None,
                positional_prompt: None,
            })),
            verbose: true,
            server: "localhost".to_string() + ":8080",
        };
        assert!(args.handle_request().await.is_err());
    }
}
