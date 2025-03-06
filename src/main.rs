use anyhow::{anyhow, Context, Error as AnyError, Result};
use chromadb::{
    client::{ChromaAuthMethod, ChromaClient, ChromaClientOptions, ChromaTokenHeader},
    collection::{ChromaCollection, CollectionEntries, GetResult},
};
use clap::Parser;
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
use tokio::io::{self, AsyncWriteExt};
use tokio_stream::StreamExt;

// https://ollama.com/blog/embedding-models

#[derive(Parser)]
struct Args {
    #[clap(short, long)]
    prompt: String,

    #[clap(short, long, default_value = "qwen2.5-coder:32b")]
    model: String,

    /// Request a certain output format, the default leaves the text as is
    #[clap(short, long, default_value_t = String::from("text"))]
    output_format: String,

    /// Text files to use as input, seperated by commas
    #[clap(short, long)]
    text_files: Option<String>,

    /// History file to use as input - invokes chat mode. #TODO
    #[clap(short, long)]
    history_file: Option<String>,

    /// Chroma database server address and port
    #[clap(short, long, default_value = "http://localhost:8000")]
    chroma_server: String,

    /// Chroma database name
    #[clap(short, long, default_value = "default")]
    chroma_database: String,

    /// Chroma token for authentication
    #[clap(short, long)]
    chroma_token: Option<String>,

    #[clap(short, long, default_value = "http://0.0.0.0:11434")]
    server: String,

    /// Path to a JSON file to amend default generation options, listed in
    /// https://docs.rs/ollama-rs/latest/ollama_rs/generation/options/struct.GenerationOptions.html
    #[clap(short, long)]
    config: Option<String>,
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

/// access a running chroma server to store and retrieve data for embeddings
// You can use the following docker command to run a chroma database:
// docker pull chromadb/chroma
// # with auth using tokens and persistent storage:
// docker run -p 8000:8000 -e chroma_server_auth_credentials_provider="chromadb.auth.token.tokenconfigserverauthcredentialsprovider" -e chroma_server_auth_provider="chromadb.auth.token.tokenauthserverprovider" -e chroma_server_auth_token_transport_header="$(sed -n 1p ~/.chroma_creds.txt)" -e chroma_server_auth_credentials="$(sed -n 2p ~/.chroma_creds.txt)" -v ~/chroma_storage/:/chroma/chroma chromadb/chroma
async fn create_chroma_client(args: &Args) -> Result<ChromaClient> {
    if let Some(token) = &args.chroma_token {
        ChromaClient::new(ChromaClientOptions {
            url: Some(args.chroma_server.clone()),
            database: args.chroma_database.clone(),
            auth: ChromaAuthMethod::TokenAuth {
                token: token.clone(),
                header: ChromaTokenHeader::Authorization,
            },
        })
        .await
    } else {
        // Defaults to http://localhost:8000
        ChromaClient::new(Default::default()).await
    }
}

async fn read_config_file(config_path: &str) -> Result<serde_json::Value> {
    let config_content = std::fs::read_to_string(config_path)?;
    serde_json::from_str(&config_content)
        .with_context(|| format!("Failed to parse config file at {}", config_path))
}

async fn get_generation_request<'a>(
    ollama: &'a Ollama,
    args: &'a Args,
) -> Result<GenerationRequest<'a>> {
    let prompt = generate_prompt(args)?;
    let options = if let Some(config_path) = &args.config {
        let mut defaults =
            serde_json::to_value(GenerationOptions::default()).with_context(|| {
                format!("Failed to serialize default generation options for config file at {config_path}")
            })?;
        // only options already present in GenerationOptions::default are allowed
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

async fn handle_request(args: Args) -> Result<()> {
    let server = &args.server;
    let ollama: Ollama = server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| anyhow!("Invalid server address: {server}"))?;

    let request = get_generation_request(&ollama, &args).await?;
    let mut stream = ollama.generate_stream(request).await?;

    let mut stdout = io::stdout();
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

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    handle_request(Args::parse()).await
}
