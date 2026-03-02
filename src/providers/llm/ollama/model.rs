use crate::options::get_options;
use crate::{Result, RuChatError};
use clap::Parser;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::{Ollama, models::ModelOptions};

#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub(crate) struct ModelArgs {
    /// Model to (down)load and use.
    #[arg(short, long)]
    model: Vec<String>,

    /// Path to a JSON file to amend default generation options, or a string
    /// representing the options in JSON format.
    #[arg(short, long)]
    options: Option<String>,
}

impl ModelArgs {
    #[cfg(test)]
    pub(crate) fn new(model: &str, options: Option<&str>) -> Self {
        let model = model.split(',').map(|s| s.trim().to_string()).collect();
        let options = options.map(|s| s.to_string());
        Self { model, options }
    }
    pub(super) async fn get_model(
        &self,
        ollama: &Ollama,
        nr: usize,
        default: &str,
    ) -> Result<String> {
        let model = match self.model.get(nr).map(|s| s.as_str()) {
            Some("") => default,
            Some(m) => m,
            None => return Err(RuChatError::NoModelSpecified),
        };
        if model.is_empty() {
            Err(RuChatError::NoModelSpecified)
        } else {
            get_model_name(ollama, model).await
        }
    }
    pub(super) fn get_nr_of_models(&self) -> usize {
        self.model.len()
    }
    pub(crate) async fn get_options(&self) -> Result<ModelOptions> {
        get_options(self.options.as_deref()).await
    }
    pub(crate) async fn build_generation_request(
        &self,
        model: String,
        prompt: String,
    ) -> Result<GenerationRequest<'_>> {
        let options = self.get_options().await?;
        Ok(GenerationRequest::new(model, prompt).options(options))
    }
}

async fn get_model_name(ollama: &Ollama, name: &str) -> Result<String> {
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
            Box::pin(get_model_name(ollama, name)).await
        }
    }
}
