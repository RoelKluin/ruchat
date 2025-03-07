use crate::args::{Args, Commands};
use crate::error::Error;
use crate::ollama_chat::chat;
use crate::ollama_query::query;
use ollama_rs::Ollama;

pub async fn get_model_name(ollama: &Ollama, name: &str) -> Result<String, Error> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ':' || c == '-' || c == '.')
    {
        return Err(Error::InvalidModelName(name.to_string()));
    }
    let model_list = ollama
        .list_local_models()
        .await
        .map_err(|_| Error::ModelNotFound(name.to_string()))?;
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
            ollama
                .pull_model(name.to_string(), false)
                .await
                .map_err(Error::OllamaError)?;
            Box::pin(get_model_name(ollama, name)).await
        }
    }
}

pub async fn handle_request(args: Args) -> Result<(), Error> {
    let server = &args.server;
    let ollama: Ollama = server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| Error::ArgServerError(server.to_string()))?;

    match args.command {
        Commands::Query(ref query_args) => query(ollama, &args, query_args).await?,
        Commands::Chat(ref chat_args) => chat(ollama, &args, chat_args).await?,
    }
    Ok(())
}
