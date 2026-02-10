mod cli;
mod core;
mod prompt;
mod providers;
mod tui;
mod utils;

use args::Args;
use clap::Parser;
pub(crate) use cli::{args, options, serde};
pub(crate) use core::{agent, chat::tree};
use error::Result;
pub(crate) use providers::llm::ollama;
pub(crate) use providers::vector::chroma;
pub(crate) use tui::io;
pub(crate) use utils::error;

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
pub async fn run() -> Result<()> {
    env_logger::init();
    let args = Args::parse();
    args.handle_request().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Commands;
    use crate::ollama::ask::AskArgs;
    use crate::ollama::OllamaArgs;
    use crate::prompt::PromptArgs;
    use args::Args;

    #[tokio::test]
    async fn test_server_run_success() {
        // Create a mock Args instance
        let args = Args {
            command: Some(Commands::Ask(AskArgs {
                output_format: "text".to_string(),
                prompt_args: PromptArgs {
                    prompt: Some("Testing, please ignore".to_string()),
                    ..Default::default()
                },
                ollama_args: OllamaArgs {
                    model: Some("qwen2.5-coder:14b".to_string()),
                    ..Default::default()
                },
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
                output_format: "text".to_string(),
                prompt_args: PromptArgs {
                    prompt: Some("Testing, please ignore".to_string()),
                    ..Default::default()
                },
                ollama_args: OllamaArgs {
                    model: Some("NO_MODEL".to_string()),
                    ..Default::default()
                },
            })),
            verbose: true,
            server: "localhost".to_string() + ":8080",
        };
        assert!(args.handle_request().await.is_err());
    }
}
