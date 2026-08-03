pub(crate) mod ask;
pub(crate) mod func;
mod model;
pub(super) mod server;
use crate::Result;
use clap::Parser;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::Ollama;
use serde::Deserialize;
use serde_json::Value;

pub(crate) use model::{get_dynamic_history_limit, ModelArgs};
pub(crate) use server::ServerArgs;

#[derive(Parser, Debug, Clone, Default, PartialEq, Deserialize)]
pub(crate) struct OllamaArgs {
    #[command(flatten)]
    server: ServerArgs,

    #[command(flatten)]
    model: ModelArgs,
}

impl OllamaArgs {
    /// see [ServerArgs::init]
    pub(crate) async fn init(&self, default: &str, cfg: &Value) -> Result<(Ollama, Vec<String>)> {
        let mut server = self.server.clone();
        if let Some(s) = cfg.get("ollama").or_else(|| cfg.get("server")) {
            server.update_from_json(s)?;
        }

        let ollama = server.init()?;
        let mut models = Vec::new();
        // At least one slot even if zero `-m`/`--model` flags were given — otherwise this loop
        // never runs at all, `models` stays empty, and a caller with a real `default` (e.g.
        // `EmbedArgs::embed_with_metadata_items`'s "all-minilm:l6-v2") still fails downstream
        // with "no model found" instead of ever getting a chance to use that default. Callers
        // with no sensible default (`delete_model`/`pull`, `default = ""`) still correctly
        // error via `get_model`'s own `model.is_empty()` check.
        for nr in 0..self.model.get_nr_of_models().max(1) {
            let model = self.model.get_model(&ollama, nr, default).await?;
            models.push(model);
        }
        Ok((ollama, models))
    }
    /// Subcommand to remove a model from the local Ollama instance.
    ///
    /// This function connects to the local Ollama instance, retrieves the specified
    /// model, and removes it from the local environment.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn delete_model(&self, cfg: &Value) -> Result<()> {
        let (ollama, models) = self.init("", cfg).await?;
        ollama.delete_model(models[0].clone()).await?;
        Ok(())
    }
    /// Subcommand to pull a model from the main Ollama server.
    ///
    /// This function connects to the Ollama server, retrieves the specified
    /// model, and pulls it to the local environment.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn pull(&self, cfg: &Value) -> Result<()> {
        let (ollama, models) = self.init("", cfg).await?;
        ollama.pull_model(models[0].clone(), false).await?;
        Ok(())
    }
    pub(crate) fn init_server(&self) -> Result<Ollama> {
        self.server.init()
    }
    /// see [ModelArgs::build_generation_request]
    pub(crate) async fn build_generation_request(
        &self,
        model: String,
        prompt: String,
        cfg: &Value,
    ) -> Result<GenerationRequest<'_>> {
        self.model
            .build_generation_request(model, prompt, cfg)
            .await
    }
}
