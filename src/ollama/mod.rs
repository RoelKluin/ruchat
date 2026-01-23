pub(crate) mod ask;
pub(crate) mod chat;
pub(crate) mod func;
pub(crate) mod model;
pub(crate) mod server;
use crate::error::Result;
use crate::ollama::model::ModelArgs;
use clap::Parser;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::{Ollama, models::ModelOptions};
use server::ServerArgs;

#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub struct OllamaArgs {
    #[command(flatten)]
    server_args: ServerArgs,

    #[command(flatten)]
    model_args: ModelArgs,
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
    pub(super) async fn rm(&self) -> Result<()> {
        let ollama = self.server_args.init()?;
        let model = self.model_args.get_model(&ollama, "").await?;
        ollama.delete_model(model).await?;
        Ok(())
    }
    /// see [ServerArgs::init]
    pub fn init(&self) -> Result<Ollama> {
        self.server_args.init()
    }
    /// see [ModelArgs::get_model]
    pub async fn get_model(&self, ollama: &Ollama, default: &str) -> Result<String> {
        self.model_args.get_model(ollama, default).await
    }
    /// see [ModelArgs::get_options]
    pub async fn get_options(&self) -> Result<ModelOptions> {
        self.model_args.get_options().await
    }
    /// see [ModelArgs::build_generation_request]
    pub async fn build_generation_request(
        &self,
        model: String,
        prompt: String,
    ) -> Result<GenerationRequest<'_>> {
        self.model_args.build_generation_request(model, prompt).await
    }
}

