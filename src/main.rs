use anyhow::Result;
use clap::{Parser, ValueEnum};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use futures_util::stream::StreamExt;
use chrono::{prelude::*, TimeZone};

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

    #[clap(short, long, default_value = "llama3.2")]
    model: String,

    #[clap(short, long, default_value = "text")]
    output_format: String,
}

#[derive(Serialize, Deserialize)]
struct OllamaPart {
    response: String,
    model: String,
    created_at: DateTime<FixedOffset>,
    done: bool,
    /*context: Option<Vec<u32>>,
    done_reason: Option<String>,
    total_duration: Option<u64>,
    load_duration: Option<u64>,
    prompt_eval_count: Option<u32>,
    prompt_eval_duration: Option<u64>,
    eval_count: Option<u32>,
    eval_duration: Option<u64>,*/
}

async fn get_model(name: &str) -> Result<()> {
    // Download the required model here
    println!("Downloading model {name}...");
    Ok(())
}

async fn send_request(args: Args) -> Result<String> {
    let client = Client::new();
    let url = "http://0.0.0.0:11434/api/generate";
    let json_body = serde_json::json!({
        "model": args.model,
        "role": args.role,
        "prompt": args.prompt
    });

    let mut response = client.post(url)
        .json(&json_body)
        .send()
        .await?
        .bytes_stream();

    let mut result = String::new();
    while let Some(chunk) = response.next().await {
        let chunk = chunk?;
        let part: OllamaPart = serde_json::from_slice(&chunk)?;
        result.push_str(&part.response);
    }
    Ok(result)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    get_model(&args.model).await?;


    let response = send_request(args).await?;
    println!("Response: {}", response);

    Ok(())
}
