use clap::Parser;
use crate::args::Args;
use crate::ollama::model::get_name;
use crate::ollama::init;
use crate::error::RuChatError;

/// Pull a model from the main Ollama server
#[derive(Parser, Debug, Clone)]
pub struct PullArgs {
    /// specify the model to pull using the --model or -m flag
    #[clap(short, long)]
    model: Option<String>,

    /// alternative positional argument to specify the model to pull
    positional_model: Option<String>,
}

/// subcommand to pull a model from the main Ollama server
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
