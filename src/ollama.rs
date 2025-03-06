use super::args::Args;
use super::config::read_config_file;
use crate::args::QueryArgs;
use log::{error, info};
use ollama_rs::{
    error::OllamaError,
    generation::{
        completion::request::GenerationRequest, embeddings::request::GenerateEmbeddingsRequest,
        options::GenerationOptions,
    },
    headers::HeaderMap,
    Ollama,
};
use serde_json::Value;
use std::fmt::{self, Display, Formatter};
use std::{fs, io::Read};
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;

#[derive(Debug)]
pub enum Error {
    InvalidModelName(String),
    ModelNotFound(String),
    FileReadError(std::io::Error),
    ConfigSerializationError(serde_json::Error),
    ConfigDeserializationError(serde_json::Error),
    ModelPullError(String),
    OllamaServerError(String),
    ReadError(String, std::io::Error),
    StreamWriteError(std::io::Error),
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::StreamWriteError(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        if err.is_data() || err.is_syntax() {
            Error::ConfigDeserializationError(err)
        } else {
            Error::ConfigSerializationError(err)
        }
    }
}

impl From<OllamaError> for Error {
    fn from(err: OllamaError) -> Self {
        match err {
            _ => todo!(),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Error::InvalidModelName(name) => write!(f, "Invalid model name: {}", name),
            Error::ModelNotFound(name) => write!(f, "Model not found: {}", name),
            Error::FileReadError(e) => write!(f, "Failed to read file: {}", e),
            Error::ConfigSerializationError(e) => write!(f, "Failed to serialize config: {}", e),
            Error::ConfigDeserializationError(e) => {
                write!(f, "Failed to deserialize config: {}", e)
            }
            Error::ModelPullError(name) => write!(f, "Failed to pull model: {}", name),
            Error::OllamaServerError(server) => write!(f, "Invalid Ollama server: {}", server),
            Error::ReadError(file, e) => write!(f, "Failed to read {}: {}", file, e),
            Error::StreamWriteError(e) => write!(f, "Failed to write to stream: {}", e),
        }
    }
}

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
    ollama: &'a Ollama,
    args: &'a Args,
    query_args: &'a QueryArgs,
) -> Result<GenerationRequest<'a>, Error> {
    let prompt = generate_prompt(query_args)?;
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
    let model_name = get_model_name(ollama, &args.model).await?;
    Ok(GenerationRequest::new(model_name, prompt).options(options))
}

pub async fn handle_request(args: Args, query_args: &QueryArgs) -> Result<(), Error> {
    let server = &args.server;
    let ollama: Ollama = server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| Error::OllamaServerError(server.to_string()))?;

    let request = get_generation_request(&ollama, &args, query_args).await?;
    let mut stream = ollama
        .generate_stream(request)
        .await
        .map_err(|_| Error::StreamWriteError(std::io::ErrorKind::Other.into()))?;

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
