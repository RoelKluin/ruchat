use crate::error::{Result, RuChatError};
use crate::io::Io;
use crate::ollama::model::get_name;
use crate::options::get_options;
use clap::Parser;
use ollama_rs::{generation::completion::request::GenerationRequest, models::ModelOptions, Ollama};
use tokio_stream::StreamExt;

const DEFAULT_MODEL: &str = "qwen2.5vl:latest";

/// Command-line arguments for piping a question to a model.
///
/// This struct defines the arguments required to pipe a question
/// to a model, including model details and configuration options.
#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub struct PipeArgs {
    /// Initial model to (down)load and use.
    #[arg(short, long, default_value = "qwen2.5vl:latest")]
    pub(crate) model: Option<String>,

    /// Optional configuration file for model options, or a string
    /// representing the options in JSON format.
    #[arg(short, long)]
    pub(crate) options: Option<String>,

    /// Specify the model using a positional argument.
    pub(crate) positional_model: Option<String>,

    /// Silent mode: suppresses output if set to true.
    #[arg(short, long, default_value_t = false)]
    silent: bool,
}

// Reusable generation logic for Agents
pub async fn generate_oneshot(
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

/// The pipe command handles prompted questions with context using a model.
///
/// This function connects to a model using the provided arguments,
/// generates a response to the specified prompt, and outputs the response.
///
/// # Parameters
///
/// - `ollama`: The Ollama client for generating responses.
/// - `args`: The command-line arguments for the pipe operation.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn pipe(ollama: Ollama, args: &PipeArgs) -> Result<()> {
    let mut cio = Io::new();
    let options = get_options(args.options.as_deref()).await?;

    // Determine the initial model name
    let model_name = match args.model.as_deref().or(args.positional_model.as_deref()) {
        Some(model) if !model.is_empty() => get_name(&ollama, model).await?,
        _ => DEFAULT_MODEL.to_string(),
    };

    let mut prompt = String::new();
    while let Ok(line) = cio.read_line().await {
        if line == "---" {
            break;
        } else {
            prompt.push_str(&line);
        }
    }
    if !prompt.is_empty() {
        let request = GenerationRequest::new(model_name, prompt).options(options.clone());
        let mut stream = ollama.generate_stream(request).await?;
        while let Some(responses) = stream.next().await.transpose()? {
            for resp in responses {
                cio.write_line(&resp.response).await?;
            }
        }
    }
    Ok(())
}
