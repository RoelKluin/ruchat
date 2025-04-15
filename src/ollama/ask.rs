use crate::io::Io;
use crate::config::read_config_file;
use crate::error::RuChatError;
use crate::ollama::model::get_name;
use clap::Parser;
use ollama_rs::{generation::completion::request::GenerationRequest, models::ModelOptions, Ollama};
use serde_json::Value;
use std::iter::Iterator;
use std::{fs, io::Read};
use tokio_stream::StreamExt;

/// Ask a question to a model and get a response
#[derive(Parser, Debug, Clone, Default)]
pub struct AskArgs {
    /// model to (down)load and use
    #[clap(short, long, default_value = "qwen2.5-coder:14b")]
    pub(crate) model: String,

    /// prompt to use, if not provided, stdin will be used
    #[clap(short, long)]
    pub(crate) prompt: Option<String>,

    /// Request a certain output format, the default leaves the text as is
    #[clap(short, long, default_value_t = String::from("text"))]
    pub(crate) output_format: String,

    /// Text files to use as input, seperated by commas
    #[clap(short = 'i', long)]
    pub(crate) text_files: Option<String>,

    /// Path to a JSON file to amend default generation options, listed in
    /// https://docs.rs/ollama-rs/0.3.0/src/ollama_rs/models.rs.html#61-94
    #[clap(short, long)]
    pub(crate) config: Option<String>,

    /// Specify the prompt using a positional argument
    pub(crate) positional_prompt: Option<String>,
}

// TODO: allow more prompt configurations
fn generate_prompt(args: &AskArgs) -> Result<String, RuChatError> {
    let mut prompt = String::new();
    if let Some(text_files) = &args.text_files {
        text_files.split(',').try_for_each(|file| {
            if prompt.is_empty() {
                prompt.push_str("Concerning:\n");
            }

            let content = if file == "-" {
                prompt.push_str("stdin:\n");
                let stdin = std::io::stdin();
                let mut handle = stdin.lock();
                let mut content = String::new();
                handle.read_to_string(&mut content)?;
                content
            } else {
                prompt.push_str("file: ");
                prompt.push_str(file);
                prompt.push('\n');
                fs::read_to_string(file)?
            };
            if content.starts_with("```") {
                prompt.push_str(&content);
            } else {
                prompt.push_str("```\n");
                prompt.push_str(&content);
                prompt.push_str("\n```");
            }
            Ok::<(), RuChatError>(())
        })?;
    }
    let question = args
        .prompt
        .as_deref()
        .or(args.positional_prompt.as_deref())
        .unwrap_or("What do you make of this?");
    prompt.push_str(question);
    Ok(prompt)
}

/// Get model options for prompt handling from a JSON file
pub(crate) async fn get_options(config: &Option<String>) -> Result<ModelOptions, RuChatError> {
    if let Some(config_path) = config {
        let mut defaults = serde_json::to_value(ModelOptions::default())?;

        if let Value::Object(ref mut defaults) = defaults {
            let updates = read_config_file(config_path).await?;
            if let Value::Object(config_updates) = updates {
                for (k, v) in config_updates.into_iter() {
                    if defaults.contains_key(&k) && !v.is_null() {
                        defaults[&k] = v.clone();
                    }
                }
            }
        }
        serde_json::from_value(defaults).map_err(RuChatError::SerdeError)
    } else {
        Ok(ModelOptions::default())
    }
}

/// The ask command handles prompted questions with context using a model
pub(crate) async fn ask(ollama: Ollama, args: &AskArgs) -> Result<(), RuChatError> {
    let mut cio = Io::new();
    let mut prompt = if args.prompt.is_some() || args.positional_prompt.is_some() || args.text_files.is_some() {
        generate_prompt(args)?
    } else {
        let mut input = String::new();
        while let Ok(line) = cio.read_line(false).await {
            if line.is_empty() {
                break;
            }
            input += line.as_str();
        }
        input
    };
    if args.output_format != "text" {
        prompt.push_str("\nPlease generate your response in valid ");
        prompt.push_str(&args.output_format);
        prompt.push_str(" output format.\n");
    }
    let model_name = get_name(&ollama, &args.model).await?;
    let request =
        GenerationRequest::new(model_name, prompt).options(get_options(&args.config).await?);
    let mut stream = ollama.generate_stream(request).await?;
    while let Some(res) = stream.next().await {
        let responses = res?;
        for resp in responses {
            cio.write_line(&resp.response).await?;
        }
    }
    Ok(())
}
