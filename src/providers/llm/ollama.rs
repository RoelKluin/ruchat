pub(crate) mod ask;
pub(crate) mod chat;
pub(crate) mod func;
mod model;
pub(super) mod server;
use crate::Result;
use clap::Parser;
use ollama_rs::Ollama;
use ollama_rs::generation::completion::request::GenerationRequest;
use serde::Deserialize;

pub(crate) use model::ModelArgs;
pub(crate) use server::ServerArgs;

#[derive(Parser, Debug, Clone, Default, PartialEq, Deserialize)]
pub(crate) struct OllamaArgs {
    #[command(flatten)]
    server: ServerArgs,

    #[command(flatten)]
    model: ModelArgs,
}

impl OllamaArgs {
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
    /// see [ServerArgs::init]
    pub(crate) async fn init(&self, default: &str) -> Result<(Ollama, Vec<String>)> {
        let ollama = self.init_server()?;
        let mut models = Vec::new();
        for nr in 0..self.model.get_nr_of_models() {
            let model = self.model.get_model(&ollama, nr, default).await?;
            models.push(model);
        }
        Ok((ollama, models))
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
