use crate::cli::prompt::PromptArgs;
use crate::io::Io;
use crate::ollama::OllamaArgs;
use crate::{Result, RuChatError};
use clap::Parser;
use ollama_rs::{Ollama, generation::completion::request::GenerationRequest, models::ModelOptions};
use tokio_stream::StreamExt;
use tokio_stream::Stream;
use std::pin::Pin;
use ollama_rs::generation::completion::GenerationResponse;
use crate::orchestrator::{Orchestrator, AgentConfig};
use futures_util::TryStreamExt;
use std::collections::HashMap;

type LlamaStream = Pin<Box<dyn Stream<Item = Result<Vec<GenerationResponse>>> + Send>>;

const DEFAULT_MODEL: &str = "qwen2.5vl:latest";

/// Command-line arguments for asking a question to a model.
///
/// This struct defines the arguments required to ask a question
/// to a model, including model details, prompt, and input options.
#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub(crate) struct AskArgs {
    is_agentic: bool,

    /// Request a certain output format, the default leaves the text as is.
    #[arg(short, long, default_value_t = String::from("text"))]
    output_format: String,

    #[command(flatten)]
    prompt: PromptArgs,

    #[command(flatten)]
    ollama: OllamaArgs,
}

// Reusable generation logic for Agents
pub(crate) async fn generate_oneshot(
    ollama: &Ollama,
    model: &str,
    prompt: &str,
    options: Option<ModelOptions>,
) -> Result<String> {
    // Resolve model name if strictly needed, or trust the Agent's config
    // For safety, we verify the model exists or use default if empty, similar to pipe
    let model_name = if model.is_empty() {
        DEFAULT_MODEL.to_string()
    } else {
        model.to_string()
    };

    let request =
        GenerationRequest::new(model_name, prompt.to_string()).options(options.unwrap_or_default());

    // We collect the stream here because agents need the full context for post-processing
    let mut stream = ollama.generate_stream(request).await?;
    let mut buffer = String::new();

    while let Some(responses) = stream.next().await.transpose()? {
        for resp in responses {
            buffer.push_str(&resp.response);
        }
    }
    Ok(buffer)
}

impl AskArgs {
    /// The ask command handles prompted questions with context using a model.
    ///
    /// This function connects to a model using the provided arguments,
    /// generates a response to the specified prompt, and outputs the response.
    ///
    /// # Parameters
    ///
    /// - `end_marker`: The marker indicating the end of user or stdin input.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn ask(&self, end_marker: &str) -> Result<()> {
        let mut cio = Io::new();
        let mut prompt = match self.prompt.get_prompt() {
            Ok(p) => p,
            Err(RuChatError::NoPromptProvided) => {
                let mut input = String::new();
                if end_marker.is_empty() {
                    // indicates user mode
                    cio.write_line("Enter your question (empty line to finish):")
                        .await?;
                }
                while let Ok(line) = cio.read_line().await {
                    if line == end_marker {
                        cio.write_error_line("End marker received, finishing input...")
                            .await?;
                        break;
                    }
                    input += line.as_str();
                }
                input
            }
            Err(e) => return Err(e),
        };
        if self.output_format != "text" {
            prompt.push_str("\nGenerate your response in valid ");
            prompt.push_str(&self.output_format);
            prompt.push_str(" output format.\n");
        }
        let (ollama, model) = self.ollama.init("").await?;

        let mut stream: LlamaStream = if self.is_agentic {
            let mut config = HashMap::new();
            config.insert("Architect".to_string(), AgentConfig::new(model[0].clone(), 0.7, "You are an Architect. Plan the solution.".into()));
            config.insert("Worker".to_string(), AgentConfig::new(model[0].clone(), 0.7, "You are a Worker. Implement the code.".into()));
            config.insert("Critic".to_string(), AgentConfig::new(model[0].clone(), 0.7, "You are a Critic. Respond with APPROVED or feedback.".into()));
            let orchestrator = Orchestrator::new(config, ollama)?;
            Box::pin(orchestrator.run_task_stream(prompt, 3))
        } else {
            // ... existing single-shot logic ...
            let request = self
                .ollama
                .build_generation_request(model[0].clone(), prompt)
                .await?;
             Box::pin(ollama.generate_stream(request).await
                .map(|res| res.map_err(RuChatError::OllamaError))
                .map_err(RuChatError::OllamaError)?)
        };
        while let Some(res) = stream.next().await {
            let responses = res?;
            for resp in responses {
                cio.write_line(&resp.response).await?;
            }
        }
        Ok(())
    }
}
