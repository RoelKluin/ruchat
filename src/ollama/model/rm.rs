use clap::Parser;
use crate::args::Args;
use crate::ollama::model::get_name;
use crate::ollama::init;
use crate::error::RuChatError;

/// Remove a model from the local Ollama instance
#[derive(Parser, Debug, Clone)]
pub struct RmArgs {
    /// specify the model to remove using the --model or -m flag
    #[clap(short, long)]
    model: Option<String>,

    /// alternative positional argument to specify the model to remove
    positional_model: Option<String>,
}

/// Subcommand to remove a model from the local Ollama instance
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


