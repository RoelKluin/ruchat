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
    pub fn init(&self) -> Result<Ollama> {
        self.server_args.init()
    }

    pub async fn get_model(&self, ollama: &Ollama, default: &str) -> Result<String> {
        self.model_args.get_model(ollama, default).await
    }
    pub async fn get_options(&self) -> Result<ModelOptions> {
        self.model_args.get_options().await
    }
    pub async fn build_generation_request(
        &self,
        model: String,
        prompt: String,
    ) -> Result<GenerationRequest<'_>> {
        self.model_args.build_generation_request(model, prompt).await
    }
}

