use crate::args::{Args, QueryArgs};
use crate::chat_io::ChatIO;
use crate::config::read_config_file;
use crate::error::RuChatError;
use crate::ollama::get_model_name;
use ollama_rs::{generation::completion::request::GenerationRequest, models::ModelOptions, Ollama};
use serde_json::Value;
use std::iter::Iterator;
use std::{fs, io::Read};
use tokio_stream::StreamExt;

// TODO: allow more prompt configurations
fn generate_prompt(query_args: &QueryArgs) -> Result<String, RuChatError> {
    let mut prompt = String::new();
    if let Some(text_files) = &query_args.text_files {
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
    prompt.push_str(&query_args.prompt);
    if query_args.output_format != "text" {
        prompt.push_str("\nPlease generate your response in valid ");
        prompt.push_str(&query_args.output_format);
        prompt.push_str(" output format.\n");
    }
    Ok(prompt)
}

async fn get_options(args: &Args) -> Result<ModelOptions, RuChatError> {
    if let Some(config_path) = &args.config {
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

pub(crate) async fn query(
    ollama: Ollama,
    args: &Args,
    query_args: Option<&QueryArgs>,
) -> Result<(), RuChatError> {
    let mut cio = ChatIO::new();
    let prompt = if let Some(query_args) = query_args {
        generate_prompt(query_args)?
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
    let model_name = get_model_name(&ollama, &args.model).await?;
    let request = GenerationRequest::new(model_name, prompt).options(get_options(args).await?);
    let mut stream = ollama.generate_stream(request).await?;
    while let Some(res) = stream.next().await {
        let responses = res?;
        for resp in responses {
            cio.write_line(&resp.response).await?;
        }
    }
    Ok(())
}
