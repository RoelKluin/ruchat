pub(crate) mod manager;
pub(crate) mod team;
pub(crate) mod worker;

pub(crate) use team::Team;
use ollama_rs::models::ModelOptions;
use ollama_rs::generation::completion::GenerationResponseStream;
use ollama_rs::error::OllamaError;
use ollama_rs::{Ollama, generation::completion::request::GenerationRequest};
pub struct Agent {
    model: String,
    options: ModelOptions,
    system_prompt: String,
}

impl Agent {
    pub(crate) fn new(
        model: String,
        options: ModelOptions,
        system_prompt: String,
    ) -> Self {
        Self {
            model,
            options,
            system_prompt,
        }
    }

    pub(crate) async fn query_stream(
        &self,
        ollama: &Ollama,
        context: &str,
        user_prompt: &str
    ) -> Result<GenerationResponseStream, OllamaError> {
        let full_prompt = format!(
            "SYSTEM: {}\n\nCONTEXT:\n{}\n\nUSER: {}",
            self.system_prompt, context, user_prompt
        );
        let request = GenerationRequest::new(self.model.clone(), full_prompt)
            .options(self.options.clone());

        ollama.generate_stream(request).await
    }
}

