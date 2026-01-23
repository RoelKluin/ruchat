use crate::error::{Result, RuChatError};
use crate::io::Io;
use crate::ollama::OllamaArgs;
use clap::Parser;
use ollama_rs::{Ollama, generation::completion::request::GenerationRequest, models::ModelOptions};
use tokio_stream::StreamExt;
use crate::prompt::PromptArgs;

const DEFAULT_MODEL: &str = "qwen2.5vl:latest";

/// Command-line arguments for asking a question to a model.
///
/// This struct defines the arguments required to ask a question
/// to a model, including model details, prompt, and input options.
#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub struct AskArgs {
    /// Request a certain output format, the default leaves the text as is.
    #[arg(short, long, default_value_t = String::from("text"))]
    pub(crate) output_format: String,

    #[command(flatten)]
    pub(crate) prompt_args: PromptArgs,

    #[command(flatten)]
    pub(crate) ollama_args: OllamaArgs,
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
        let mut prompt = match self.prompt_args.get_prompt() {
            Ok(p) => p,
            Err(RuChatError::NoPromptProvided) => {
                let mut input = String::new();
                if end_marker == "" { // indicates user mode
                    cio.write_line("Enter your question (empty line to finish):").await?;
                }
                while let Ok(line) = cio.read_line().await {
                    if line == end_marker {
                        break;
                    }
                    input += line.as_str();
                }
                input
            },
            Err(e) => return Err(e),
        };
        if self.output_format != "text" {
            prompt.push_str("\nGenerate your response in valid ");
            prompt.push_str(&self.output_format);
            prompt.push_str(" output format.\n");
        }
        let ollama = self.ollama_args.init()?;
        let model = self.ollama_args.get_model(&ollama, "").await?;
        let request = self
            .ollama_args
            .build_generation_request(model, prompt)
            .await?;
        let mut stream = ollama.generate_stream(request).await?;
        while let Some(res) = stream.next().await {
            let responses = res?;
            for resp in responses {
                cio.write_line(&resp.response).await?;
            }
        }
        Ok(())
    }
}
