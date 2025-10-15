pub(crate) mod ask;
pub(crate) mod chat;
pub(crate) mod func;
pub(crate) mod model;
pub(crate) mod pipe;
use crate::args::Args;
use crate::error::RuChatError;
use ollama_rs::Ollama;

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
pub(crate) fn init(args: &Args) -> Result<Ollama, RuChatError> {
    if args.verbose {
        println!("Connecting to Ollama server at {}", args.server);
    }
    args.server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| RuChatError::ArgServerError(args.server.to_string()))
}

pub async fn get_model_name(ollama: &Ollama, name: &str) -> Result<String, RuChatError> {
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
