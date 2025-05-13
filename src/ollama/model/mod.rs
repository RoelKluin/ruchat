pub(crate) mod ls;
pub(crate) mod pull;
pub(crate) mod rm;
use crate::error::RuChatError;
use ollama_rs::Ollama;

pub async fn get_name(ollama: &Ollama, name: &str) -> Result<String, RuChatError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ':' || c == '-' || c == '.' || c == '/')
    {
        return Err(RuChatError::ModelError(format!("invalid name: {name}")));
    }
    let model_list = ollama
        .list_local_models()
        .await
        .map_err(|_| RuChatError::ModelError(format!("{name} not found")))?;
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
            Box::pin(get_name(ollama, name)).await
        }
    }
}
