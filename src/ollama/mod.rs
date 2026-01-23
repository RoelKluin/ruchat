mod ask;
mod chat;
pub(super) mod func;
mod model;
pub(super) mod server;
use crate::error::Result;
use clap::Parser;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::{Ollama, models::ModelOptions};

pub(super) use ask::*;
pub(super) use chat::*;
pub(super) use func::{func};
pub(super) use model::ModelArgs;
pub(crate) use server::ServerArgs;

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
        let (ollama, model) = self.init("").await?;
        ollama.delete_model(model).await?;
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
    pub(super) async fn pull(&self) -> Result<()> {
        let (ollama, model) = self.init("").await?;
        ollama.pull_model(model, false).await?;
        Ok(())
    }
    pub fn init_server(&self) -> Result<Ollama> {
        self.server_args.init()
    }
    /// see [ServerArgs::init]
    pub async fn init(&self, default: &str) -> Result<(Ollama, String)> {
        let ollama = self.init_server()?;
        self.model_args.get_model(&ollama, default).await.map(|model| (ollama, model))
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

