use super::args::Args;
use super::config::read_config_file;
use anyhow::{anyhow, Context, Error as AnyError, Result};
use log::{error, info};
use ollama_rs::{
    generation::{
        completion::request::GenerationRequest, embeddings::request::GenerateEmbeddingsRequest,
        options::GenerationOptions,
    },
    headers::HeaderMap,
    Ollama,
};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;

fn generate_prompt(args: &Args) -> Result<String> {
    let mut prompt = String::new();
    if let Some(text_files) = &args.text_files {
        text_files.split(',').try_for_each(|file| {
            if prompt.is_empty() {
                prompt.push_str("Considering the input:\n");
            }

            if file == "-" {
                prompt.push_str("stdin:");
            } else {
                prompt.push_str("file: ");
                prompt.push_str(file);
            }
            prompt.push_str("\n```\n");
            prompt.push_str(
                &std::fs::read_to_string(file)
                    .with_context(|| format!("Failed to read file: {file}"))?,
            );
            prompt.push_str("\n```\n");
            Ok::<(), AnyError>(())
        })?;
    }
    prompt.push_str(&args.prompt);
    if args.output_format != "text" {
        prompt.push_str("\nPlease generate your response in valid ");
        prompt.push_str(&args.output_format);
        prompt.push_str(" output format.\n");
    }
    Ok(prompt)
}

async fn get_model_name(ollama: &Ollama, name: &str) -> Result<String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ':' || c == '-' || c == '.')
    {
        return Err(anyhow::anyhow!("Invalid model name: {name}."));
    }
    info!("Model: {}", name);
    let model_list = ollama.list_local_models().await?;
    let model = if name.contains(":") {
        model_list.iter().find(|m| m.name == name)
    } else {
        model_list.iter().find(|m| m.name.starts_with(name))
    };

    match model {
        Some(model) => Ok(model.name.clone()),
        None => {
            ollama
                .pull_model(name.to_string(), false)
                .await
                .map_err(|e| anyhow!("Failed to pull model: {e}"))?;
            Box::pin(get_model_name(ollama, name)).await
        }
    }
}

pub async fn get_generation_request<'a>(
    ollama: &'a Ollama,
    args: &'a Args,
) -> Result<GenerationRequest<'a>, Box<dyn std::error::Error>> {
    let prompt = generate_prompt(args)?;
    let options = if let Some(config_path) = &args.config {
        let mut defaults =
            serde_json::to_value(GenerationOptions::default()).with_context(|| {
                format!("Failed to serialize default generation options for config file at {config_path}")
            })?;
        if let Value::Object(ref mut defaults) = defaults {
            if let Value::Object(config_updates) = read_config_file(config_path).await? {
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
    Ok(GenerationRequest::new(model_name, prompt).options(options))
}

pub async fn handle_request(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let server = &args.server;
    let ollama: Ollama = server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| anyhow!("Invalid server address: {server}"))?;

    let request = get_generation_request(&ollama, &args).await?;
    let mut stream = ollama.generate_stream(request).await?;

    let mut stdout = tokio::io::stdout();
    while let Some(res) = stream.next().await {
        let responses = res?;
        for resp in responses {
            match stdout.write_all(resp.response.as_bytes()).await {
                Ok(_) => {}
                Err(e) => {
                    error!("Failed to write response to stdout: {e}");
                    return Err(e.into());
                }
            }
            stdout.flush().await?;
        }
    }
    Ok(())
}
