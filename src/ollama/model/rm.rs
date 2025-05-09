use clap::Parser;
use crate::args::Args;
use crate::ollama::model::get_name;
use crate::ollama::init;
use crate::error::RuChatError;

/// Command-line arguments for removing a model from the local Ollama instance.
///
/// This struct defines the arguments required to remove a model
/// from the local Ollama instance, including model details.
#[derive(Parser, Debug, Clone)]
pub struct RmArgs {
    /// Specify the model to remove using the --model or -m flag.
    #[clap(short, long)]
    model: Option<String>,

    /// Alternative positional argument to specify the model to remove.
    positional_model: Option<String>,
}

/// Subcommand to remove a model from the local Ollama instance.
///
/// This function connects to the local Ollama instance, retrieves the specified
/// model, and removes it from the local environment.
///
/// # Parameters
///
/// - `args`: The command-line arguments containing the server information.
/// - `rm_args`: The command-line arguments for the remove operation.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn remove(args: &Args, rm_args: &RmArgs) -> Result<(), RuChatError> {
    let ollama = init(args)?;
    match rm_args.model.as_deref().or(rm_args.positional_model.as_deref()) {
        Some(model) if !model.is_empty() => {
            let model_name = get_name(&ollama, model).await?;
            ollama.delete_model(model_name).await?;
            Ok(())
        }
        _ => Err(RuChatError::ModelError("Model name is required".to_string())),
    }
}
