use super::args::Args;
use super::config::read_config_file;
use crate::args::{ChatArgs, Commands, QueryArgs};
use crate::ollama_error::Error;
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
use std::{
    fs,
    io::{stdin, Read},
};
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;

// TODO: allow more prompt configurations
fn generate_prompt(query_args: &QueryArgs) -> Result<String, Error> {
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
            Ok::<(), Error>(())
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

async fn get_model_name(ollama: &Ollama, name: &str) -> Result<String, Error> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ':' || c == '-' || c == '.')
    {
        return Err(Error::InvalidModelName(name.to_string()));
    }
    info!("Model: {}", name);
    let model_list = ollama
        .list_local_models()
        .await
        .map_err(|_| Error::ModelNotFound(name.to_string()))?;
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
                .map_err(|_| Error::ModelPullError(name.to_string()))?;
            Box::pin(get_model_name(ollama, name)).await
        }
    }
}

pub async fn get_generation_request<'a>(
    args: &'a Args,
    model_name: &str,
    prompt: String,
) -> Result<GenerationRequest<'a>, Error> {
    let options = if let Some(config_path) = &args.config {
        let mut defaults = serde_json::to_value(GenerationOptions::default())
            .map_err(Error::ConfigDeserializationError)?;

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
        serde_json::from_value(defaults).map_err(Error::ConfigDeserializationError)?
    } else {
        GenerationOptions::default()
    };
    Ok(GenerationRequest::new(model_name.to_string(), prompt).options(options))
}

pub async fn handle_request(args: Args) -> Result<(), Error> {
    let server = &args.server;
    let ollama: Ollama = server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| Error::OllamaServerError(server.to_string()))?;

    let mut stdout = tokio::io::stdout();
    let stdin = stdin();
    let model_name = get_model_name(&ollama, &args.model).await?;
    let mut do_exit = false;
    while !do_exit {
        let prompt = match &args.command {
            Commands::Query(query_args) => {
                do_exit = true;
                generate_prompt(&query_args)?
            }
            Commands::Chat(_chat_args) => {
                stdout.write_all(b"\n> ").await?;
                stdout.flush().await?;

                let mut input = String::new();
                stdin.read_line(&mut input)?;

                let input = input.trim_end();
                do_exit = input.eq_ignore_ascii_case("exit");
                input.to_string()
            }
        };
        let request = get_generation_request(&args, &model_name, prompt).await?;

        let mut stream = ollama
            .generate_stream(request)
            .await
            .map_err(|_| Error::StreamWriteError(std::io::ErrorKind::Other.into()))?;

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
    }
    Ok(())
}
