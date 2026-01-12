use crate::error::RuChatError;
use crate::io::Io;
use crate::ollama::model::get_name;
use crate::options::get_options;
use clap::Parser;
use ollama_rs::{generation::completion::request::GenerationRequest, Ollama};
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

    /// Path to a JSON file to amend default generation options, or a string
    /// representing the options in JSON format.
    #[arg(short, long)]
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
        _ => DEFAULT_MODEL.to_string(), // will error out later if no model provided
    };

    while !done {
        cio.write_line("***\n").await?;
        let mut prompt = String::new();
        while let Ok(line) = cio.read_line().await {
            match line.as_str() {
                "---" | "***" | "___" => break,
                _ => {
                    if line == "!done" {
                        done = true;
                        break;
                    }
                    if let Some(new_model) = line.strip_prefix("!model: ") {
                        cio.write_line(&format!("[Switching model to {}]\n", new_model.trim()))
                            .await?;
                        model_name = get_name(&ollama, new_model.trim()).await?;
                    } else if let Some(new_options) = line.strip_prefix("!options: ") {
                        options = get_options(Some(new_options.trim())).await?;
                        cio.write_line("[Updated generation options]\n").await?;
                    } else if !line.starts_with("!!") {
                        // or comment
                        prompt.push_str(&line);
                    }
                }
            }
        }
        if model_name.is_empty() {
            return Err(RuChatError::NoModelSpecified);
        }
        if prompt.is_empty() {
            //done = true;
        } else {
            cio.write_line(&format!("> {prompt}\n---\n")).await?;
            let request =
                GenerationRequest::new(model_name.clone(), prompt).options(options.clone());
            let mut stream = ollama.generate_stream(request).await?;
            while let Some(res) = stream.next().await {
                let responses = res?;
                for resp in responses {
                    cio.write_line(&resp.response).await?;
                }
            }
            cio.write_line("\n").await?;
        }
    }
    Ok(())
}
