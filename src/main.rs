use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::generation::options::GenerationOptions;
use ollama_rs::Ollama;
use tokio::io::{self, AsyncWriteExt};
use tokio_stream::StreamExt;

use anyhow::{anyhow, Error as AnyError, Result};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, ValueEnum, Serialize, Deserialize)] // ArgEnum here
#[clap(rename_all = "lower")]
enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Parser)]
struct Args {
    #[clap(short, long, value_enum, default_value = "user")]
    role: Role,

    #[clap(short, long)]
    prompt: String,

    #[clap(short, long, default_value = "qwen2.5-coder:latest")]
    model: String,

    #[clap(short, long, default_value = "text")]
    output_format: String,

    /// Text files to use as input, seperated by commas
    #[clap(short, long)]
    text_files: Option<String>,

    #[clap(short, long, default_value = "http://localhost:11434")]
    server: String,

    #[clap(short, long)]
    config: Option<String>,
}

async fn get_model(ollama: &Ollama, name: &str) -> Result<String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ':' || c == '-' || c == '.')
    {
        return Err(anyhow::anyhow!("Invalid model name: {}.", name));
    }
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
                .map_err(|e| anyhow!("Failed to pull model: {}", e))?;
            Box::pin(get_model(ollama, name)).await
        }
    }
}

fn generate_prompt(args: &Args) -> Result<String> {
    let mut prompt = String::new();
    if let Some(text_files) = &args.text_files {
        text_files.split(',').try_for_each(|file| {
            if prompt.is_empty() {
                prompt.push_str("Considering the input:\n");
            }

            if file != "-" {
                prompt.push_str("file: ");
                prompt.push_str(file);
            } else {
                prompt.push_str("stdin:");
            }
            prompt.push_str("\n```\n");
            prompt.push_str(&std::fs::read_to_string(file)?);
            prompt.push_str("\n```\n");
            Ok::<(), AnyError>(())
        })?;
    }
    prompt.push_str(&args.prompt);
    Ok(prompt)
}

fn merge(a: &mut Value, b: Value) {
    if let Value::Object(a) = a {
        if let Value::Object(b) = b {
            for (k, v) in b {
                if v.is_null() {
                    a.remove(&k);
                } else {
                    merge(a.entry(k).or_insert(Value::Null), v);
                }
            }
            return;
        }
    }
    *a = b;
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let ollama = match args.server.rsplit_once(':') {
        Some((host, port)) => {
            if port.parse::<u16>().is_err() {
                return Err(anyhow::anyhow!("Invalid port number: {}", port));
            }
            Ollama::new(host.to_string(), port.parse()?)
        }
        None => return Err(anyhow::anyhow!("Invalid server address: {}", args.server)),
    };
    let completed_model_name = get_model(&ollama, &args.model).await?;

    let prompt = generate_prompt(&args)?;

    let mut options = GenerationOptions::default();

    if let Some(config) = &args.config {
        let update: Value = std::fs::read_to_string(config)
            .map(|s| serde_json::from_str(&s))
            .map_err(|e| anyhow::anyhow!("Failed to read config file: {}", e))??;

        let mut defaults = serde_json::to_value(&options)?;
        merge(&mut defaults, update);
        options = serde_json::from_value(defaults)?;
    }

    let mut stream = ollama
        .generate_stream(GenerationRequest::new(completed_model_name, prompt).options(options))
        .await?;

    let mut stdout = io::stdout();
    while let Some(res) = stream.next().await {
        let responses = res?;
        for resp in responses {
            stdout.write_all(resp.response.as_bytes()).await?;
            stdout.flush().await?;
        }
    }
    return Ok(());
}
