use crate::error::RuChatError;
use crate::io::Io;
use crate::ollama::model::get_name;
use crate::options::get_options;
use clap::Parser;
use ollama_rs::{Ollama, generation::completion::request::GenerationRequest};
use tokio_stream::StreamExt;

/// Command-line arguments for piping a question to a model.
///
/// This struct defines the arguments required to pipe a question
/// to a model, including model details and configuration options.
#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub struct PipeArgs {
    /// Initial model to (down)load and use.
    #[clap(short, long, default_value = "qwen2.5-coder:14b")]
    pub(crate) model: Option<String>,

    /// Path to a JSON file to amend default generation options, or a string
    /// representing the options in JSON format.
    #[clap(short, long)]
    pub(crate) options: Option<String>,

    /// Specify the model using a positional argument.
    pub(crate) positional_model: Option<String>,
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
pub(crate) async fn pipe(ollama: Ollama, args: &PipeArgs) -> Result<(), RuChatError> {
    let mut cio = Io::new();
    let mut done = false;
    let mut options = get_options(args.options.as_deref()).await?;
    let mut model_name = match args.model.as_deref().or(args.positional_model.as_deref()) {
        Some(model) if !model.is_empty() => get_name(&ollama, model).await?,
        _ => {
            return Err(RuChatError::ModelError(
                "Model name is required".to_string(),
            ));
        }
    };

    while !done {
        let mut prompt = String::new();
        while let Ok(line) = cio.read_line(false).await {
            match line.as_str() {
                "" => {
                    done = true;
                    break;
                }
                "---" | "***" | "___" => break,
                _ if line.starts_with("!instruction") => {
                    // Parse instruction
                    let instruction = line.trim_start_matches("!instruction").trim();
                    if instruction.starts_with("model:") {
                        // Change model
                        let new_model = instruction.trim_start_matches("model:").trim();
                        model_name = get_name(&ollama, new_model).await?;
                    } else if instruction.starts_with("options:") {
                        // Change options
                        let new_options = instruction.trim_start_matches("options:").trim();
                        options = get_options(Some(new_options)).await?;
                    }
                }
                _ => prompt.push_str(&line),
            }
        }

        prompt.push_str("\nRespond in CommonMark format.\n");
        let request = GenerationRequest::new(model_name.clone(), prompt).options(options.clone());
        let mut stream = ollama.generate_stream(request).await?;
        while let Some(res) = stream.next().await {
            let responses = res?;
            for resp in responses {
                cio.write_line(&resp.response).await?;
            }
        }
    }
    Ok(())
}
