use clap::Parser;
use crate::args::Args;
use crate::ollama::model::get_name;
use crate::ollama::init;
use crate::error::RuChatError;

#[derive(Parser, Debug, Clone)]
pub struct RmArgs {
    /// specify the model to remove using the --model or -m flag
    #[clap(short, long)]
    model: Option<String>,

    /// specify the model to pull using the positional argument
    positional_model: Option<String>,
}

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


