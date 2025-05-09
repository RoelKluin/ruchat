use crate::io::Io;
use crate::config::get_options;
use crate::error::RuChatError;
use crate::ollama::model::get_name;
use clap::Parser;
use ollama_rs::{generation::completion::request::GenerationRequest, Ollama};
use tokio_stream::StreamExt;

/// Pipe a question to a model and get a response
#[derive(Parser, Debug, Clone, Default)]
pub struct PipeArgs {
    /// initial model to (down)load and use
    #[clap(short, long, default_value = "qwen2.5-coder:14b")]
    pub(crate) model: Option<String>,

    /// Path to a JSON file to amend default generation options, listed in
    /// https://docs.rs/ollama-rs/0.3.0/src/ollama_rs/models.rs.html#61-94
    #[clap(short, long)]
    pub(crate) config: Option<String>,

    /// Specify the model using a positional argument
    pub(crate) positional_model: Option<String>,
}

/// The pipe command handles prompted questions with context using a model
pub(crate) async fn pipe(ollama: Ollama, args: &PipeArgs) -> Result<(), RuChatError> {
    let mut cio = Io::new();
    let mut done = false;
    let mut options = get_options(&args.config).await?;
    let mut model_name = match args.model.as_deref().or(args.positional_model.as_deref()) {
        Some(model) if !model.is_empty() => {
            get_name(&ollama, model).await?
        }
        _ => return Err(RuChatError::ModelError("Model name is required".to_string())),
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
                _ => prompt.push_str(&line),
            }
        }

        prompt.push_str("\nRespond in CommonMark format.\n");
        let request =
            GenerationRequest::new(model_name.clone(), prompt).options(options.clone());
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
