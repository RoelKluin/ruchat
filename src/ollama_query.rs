use crate::args::{Args, QueryArgs};
use crate::chat_io::ChatIO;
use crate::config::read_config_file;
use crate::error::RuChatError;
use crate::ollama::get_model_name;
use ollama_rs::{
    generation::{completion::request::GenerationRequest, options::GenerationOptions},
    Ollama,
};
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
                prompt.push_str("Considering the input:\n");
            }

            let content = if file == "-" {
                prompt.push_str("stdin:");
                let stdin = std::io::stdin();
                let mut handle = stdin.lock();
                let mut content = String::new();
                handle.read_to_string(&mut content)?;
                content
            } else {
                prompt.push_str("file: ");
                prompt.push_str(file);
                fs::read_to_string(file)?
            };
            prompt.push_str("\n```\n");
            prompt.push_str(&content);
            prompt.push_str("\n```\n");
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

pub async fn get_generation_request<'a>(
    ollama: &Ollama,
    args: &'a Args,
    query_args: &QueryArgs,
) -> Result<GenerationRequest<'a>, RuChatError> {
    let options = if let Some(config_path) = &args.config {
        let mut defaults = serde_json::to_value(GenerationOptions::default())?;

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
        serde_json::from_value(defaults)?
    } else {
        GenerationOptions::default()
    };
    let model_name = get_model_name(ollama, &args.model).await?;
    let prompt = generate_prompt(query_args)?;
    Ok(GenerationRequest::new(model_name, prompt).options(options))
}

pub async fn query(ollama: Ollama, args: &Args, query_args: &QueryArgs) -> Result<(), RuChatError> {
    let request = get_generation_request(&ollama, args, query_args).await?;
    let mut stream = ollama.generate_stream(request).await?;
    let mut cio = ChatIO::new();
    while let Some(res) = stream.next().await {
        let responses = res?;
        for resp in responses {
            cio.write_line(&resp.response).await?;
        }
    }
    Ok(())
}
