pub(crate) mod ask;
pub(crate) mod chat;
pub(crate) mod func;
mod model;
pub(super) mod server;
use crate::cli::config::ConfigArgs;
use crate::Result;
use clap::Parser;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::Ollama;
use serde::Deserialize;

pub(crate) use model::{get_dynamic_history_limit, ModelArgs};
pub(crate) use server::ServerArgs;

#[derive(Parser, Debug, Clone, Default, PartialEq, Deserialize)]
pub(crate) struct OllamaArgs {
    #[command(flatten)]
    server: ServerArgs,

    #[command(flatten)]
    model: ModelArgs,

    #[command(flatten)]
    config: ConfigArgs,
}

impl OllamaArgs {
    /// see [ServerArgs::init]
    pub(crate) async fn init(&self, default: &str) -> Result<(Ollama, Vec<String>)> {
        let mut cfg = self.config.load().await?;
        self.config.merge_into(cfg.clone(), &mut cfg); // ensure profile applied

        // Apply config to server and model (existing update_from_json)
        if let Some(s) = cfg.get("ollama").or_else(|| cfg.get("server")) {
            let mut server = self.server.clone();
            server.update_from_json(s)?;
        }

        let ollama = self.server.init()?;
        let mut models = Vec::new();
        for nr in 0..self.model.get_nr_of_models() {
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
    pub(crate) async fn delete_model(&self) -> Result<()> {
        let (ollama, models) = self.init("").await?;
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
    pub(crate) async fn pull(&self) -> Result<()> {
        let (ollama, models) = self.init("").await?;
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
    ) -> Result<GenerationRequest<'_>> {
        self.model.build_generation_request(model, prompt).await
    }
}
