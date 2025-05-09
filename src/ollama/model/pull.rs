use clap::Parser;
use crate::args::Args;
use crate::ollama::model::get_name;
use crate::ollama::init;
use crate::error::RuChatError;

/// Command-line arguments for pulling a model from the main Ollama server.
///
/// This struct defines the arguments required to pull a model
/// from the main Ollama server, including model details.
#[derive(Parser, Debug, Clone)]
pub struct PullArgs {
    /// Specify the model to pull using the --model or -m flag.
    #[clap(short, long)]
    model: Option<String>,

    /// Alternative positional argument to specify the model to pull.
    positional_model: Option<String>,
}

/// Subcommand to pull a model from the main Ollama server.
///
/// This function connects to the Ollama server, retrieves the specified
/// model, and pulls it to the local environment.
///
/// # Parameters
///
/// - `args`: The command-line arguments containing the server information.
/// - `pull_args`: The command-line arguments for the pull operation.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn pull(args: &Args, pull_args: &PullArgs) -> Result<(), RuChatError> {
    let ollama = init(args)?;
    match pull_args.model.as_deref().or(pull_args.positional_model.as_deref()) {
        Some(model) if !model.is_empty() => {
            let model_name = get_name(&ollama, model).await?;
            ollama.pull_model(model_name, false).await?;
            Ok(())
        }
        _ => Err(RuChatError::ModelError("Model name is required".to_string())),
    }
}
