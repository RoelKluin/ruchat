use ollama_rs::Ollama;
use ollama_rs::generation::options::GenerationOptions;
use ollama_rs::generation::completion::request::GenerationRequest;
use tokio::io::{self, AsyncWriteExt};
use tokio_stream::StreamExt;

use anyhow::{Result, Context};
use clap::{Parser, ValueEnum};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, FixedOffset};
use indicatif::{ProgressBar, ProgressStyle};
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
    text_files: String,

    #[clap(short, long, default_value = "http://localhost:11434")]
    server: String,
}

#[derive(Serialize, Deserialize)]
struct ReqResponsePart {
    response: String,
    model: String,
    created_at: DateTime<FixedOffset>,
    done: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct ModelDetails {
    parent_model: String,
    format: String,
    family: String,
    // families: Vec<String>,
    parameter_size: String,
    quantization_level: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Model {
    name: String,
    model: String,
    modified_at: DateTime<FixedOffset>,
    size: u64,
    digest: String,
    details: ModelDetails,
}

#[derive(Serialize, Deserialize, Debug)]
struct ModelList {
    models: Vec<Model>,
}

#[derive(Serialize, Deserialize, Debug)]
struct PullResponsePart {
    status: String,
    digest: Option<String>,
    total: Option<u64>,
    completed: Option<u64>,
}

async fn get_model(client: &Client, args: &Args, pb: &ProgressBar) -> Result<()> {
    let url = format!("{}/api/tags", args.server);
    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Failed to get model list: {}", response.text().await?));
    }

    let model_list: ModelList = serde_json::from_str(&response.text().await?)?;

    let name = &args.model;
    let model = if name.contains(":") {
        model_list.models.iter().find(|m| &m.name == name)
    } else {
        model_list.models.iter().find(|m| m.name.starts_with(name))
    };

    if let Some(model) = model {
        let model_name = &model.name;
        eprintln!("Model {name} resolved to existing model {model_name}.");
    } else {
        println!("Downloading model {name}...");
        let url = format!("{}/api/pull", args.server);
        let json_body = serde_json::json!({
            "model": args.model
        });

        // Print the request body for debugging purposes
        eprintln!("Request body: {:?}", json_body);

        let mut response = client.post(&url)
            .json(&json_body)
            .send()
            .await?
            .bytes_stream();

        pb.set_message("Downloading model...");
        let mut last_digest: Option<String> = None;

        while let Some(chunk) = response.next().await {
            let chunk = chunk?;
            if let Ok(part) = serde_json::from_slice::<PullResponsePart>(&chunk) {
                match part.status.as_str() {
                    "success" => {
                        pb.set_message("Downloaded model manifest.");
                        break;
                    },
                    x if x.starts_with("pulling ") => {
                        if last_digest.as_ref() != part.digest.as_ref() {
                            pb.reset();
                            pb.set_message("Downloading model:");
                            last_digest = part.digest.clone();
                        }
                        if part.completed.is_some() {
                            pb.inc(8);
                        } else {
                            pb.inc(20);
                        }
                    },
                    msg => {
                        eprintln!("response: {msg}");
                        pb.set_message(msg.to_string());
                    }
                }
            } else {
                pb.set_message(std::str::from_utf8(&chunk).unwrap_or_default().to_string());
            }
        }
    }

    Ok(())
}

fn generate_prompt(args: &Args) -> Result<String> {
    let mut prompt = String::new();
    args.text_files.split(',').into_iter().map(|file| {
        if prompt.is_empty() {
            prompt.push_str("Considering the input:\n");
        }

        if file != "-" {
            prompt.push_str("file: ");
            prompt.push_str(&file);
        } else {
            prompt.push_str("stdin:");
        }
        prompt.push_str("\n```\n");
        prompt.push_str(&std::fs::read_to_string(file).context("Failed to read file")?);
        prompt.push_str("\n```\n");
        Ok(())
    }).collect::<Result<()>>()?;
    prompt.push_str(&args.prompt);
    Ok(prompt)
}


async fn send_request(client: Client, args: Args) -> Result<()> {
    let json_body = serde_json::json!({
        "model": args.model,
        "role": args.role,
        "prompt": generate_prompt(&args)?
    });

    let mut response = client.post(args.server + "/api/generate")
        .json(&json_body)
        .send()
        .await?
        .bytes_stream();

    while let Some(chunk) = response.next().await {
        let chunk = chunk?;
        let part: ReqResponsePart = serde_json::from_slice(&chunk)
            .or_else(|e| {
                let json: Value = serde_json::from_slice(&chunk)?;
                if json["done"] == true {
                    Ok(ReqResponsePart {
                        response: String::new(),
                        model: json["model"].as_str().unwrap().to_string(),
                        created_at: DateTime::parse_from_rfc3339(json["created_at"].as_str().unwrap())?,
                        done: json["done"].as_bool().unwrap(),
                    })
                } else {
                    Err(anyhow::anyhow!("Failed to parse response from {}: {}", std::str::from_utf8(&chunk).unwrap_or_default(), e))
                }
            })?;
        print!("{}", part.response);
    }
    println!("\n");
    Ok(())
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
        },
        None => return Err(anyhow::anyhow!("Invalid server address: {}", args.server)),
    };

    let model = args.model.clone();
    let prompt = generate_prompt(&args)?;

    let options = GenerationOptions::default()
        .temperature(0.2)
        .repeat_penalty(1.5)
        .top_k(25)
        .top_p(0.25);

    let mut stream = ollama.generate_stream(GenerationRequest::new(model, prompt)).await?;

    let mut stdout = io::stdout();
    while let Some(res) = stream.next().await {
        for resp in res? {
            resp.response.as_bytes().iter().for_each(|b| {
                async {
                    match stdout.write_all(&[*b]).await {
                        Ok(_) => (),
                        Err(e) => eprintln!("Failed to write to stdout: {}", e),
                    }
                    match stdout.flush().await {
                        Ok(_) => (),
                        Err(e) => eprintln!("Failed to flush stdout: {}", e),
                    }
                };
                ()
            });
            //stdout.write_all(resp.response.as_bytes()).await?;
            //stdout.flush().await?;
        }
    }
    return Ok(());
    

    /*let client = Client::new();
    let pb = ProgressBar::new(100);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")?
        .progress_chars("#>-"));

    get_model(&client, &args, &pb).await?;

    send_request(client, args).await*/
}
