pub(crate) mod ask;
pub(crate) mod chat;
pub(crate) mod func;
pub(crate) mod model;
pub(crate) mod pipe;
use crate::args::Args;
use crate::error::{Result, RuChatError};
use crate::ollama::model::get_name;
use crate::options::get_options;
use clap::Parser;
use ollama_rs::{models::ModelOptions, Ollama};

const DEFAULT_MODEL: &str = "qwen2.5vl:latest";

#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub struct OllamaArgs {
    /// Model to (down)load and use.
    #[arg(short, long)]
    pub(crate) model: Option<String>,

    /// Path to a JSON file to amend default generation options, or a string
    /// representing the options in JSON format.
    #[arg(short, long)]
    pub(crate) options: Option<String>,

    /// Specify the model using a positional argument.
    pub(crate) positional_model: Option<String>,
}

impl OllamaArgs {
    pub async fn get_model(&self, ollama: &Ollama) -> Result<String> {
        // Determine the initial model name
        match self.model.as_deref().or(self.positional_model.as_deref()) {
            Some(model) if !model.is_empty() => get_name(&ollama, model).await,
            _ => Ok(DEFAULT_MODEL.to_string()),
        }
    }
    pub async fn get_options(&self) -> Result<ModelOptions> {
        get_options(self.options.as_deref()).await
    }
}

/// Initializes a connection to an Ollama server.
///
/// This function parses the server address and port from the provided
/// arguments and establishes a connection to the Ollama server.
///
/// # Parameters
///
/// - `args`: The command-line arguments containing the server information.
///
/// # Returns
///
/// A `Result` containing the `Ollama` client or a `RuChatError`.
pub(crate) fn init(args: &Args) -> Result<Ollama> {
    if args.verbose {
        println!("Connecting to Ollama server at {}", args.server);
    }
    args.server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| RuChatError::ArgServerError(args.server.to_string()))
}

pub async fn get_model_name(ollama: &Ollama, name: &str) -> Result<String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ':' || c == '-' || c == '.' || c == '/')
    {
        return Err(RuChatError::InvalidModelName(name.to_string()));
    }
    let model_list = ollama
        .list_local_models()
        .await
        .map_err(|_| RuChatError::ModelNotFound(name.to_string()))?;
    let model = model_list.iter().find(|m| {
        if name.contains(":") {
            m.name == name
        } else {
            m.name.starts_with(name)
        }
    });

    match model {
        Some(model) => Ok(model.name.clone()),
        None => {
            ollama.pull_model(name.to_string(), false).await?;
            Box::pin(get_model_name(ollama, name)).await
        }
    }
}
