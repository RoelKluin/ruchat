use crate::args::{Args, Commands};
use crate::chroma::query;
use crate::error::RuChatError;
use crate::ollama_ask::{ask, AskArgs};
use crate::ollama_chat::chat;
use crate::ollama_embed::embed;
use crate::ollama_func::func;
use crate::ollama_func_struct::func_struct;
use clap::Parser;
use ollama_rs::Ollama;

pub async fn get_model_name(ollama: &Ollama, name: &str) -> Result<String, RuChatError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ':' || c == '-' || c == '.')
    {
        return Err(RuChatError::InvalidModelName(name.to_string()));
    }
    let model_list = ollama
        .list_local_models()
        .await
        .map_err(|_| RuChatError::ModelNotFound(name.to_string()))?;
    let model = model_list.iter().find(|m| {
        if name.contains(":") {
            m.name == name
        } else {
            m.name.starts_with(name)
        }
    });

    match model {
        Some(model) => Ok(model.name.clone()),
        None => {
            ollama.pull_model(name.to_string(), false).await?;
            Box::pin(get_model_name(ollama, name)).await
        }
    }
}

fn format_size(size: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let size_f64 = size as f64;

    if size_f64 >= GIB {
        format!("{:.1}G", size_f64 / GIB)
    } else if size_f64 >= MIB {
        format!("{:.1}M", size_f64 / MIB)
    } else if size_f64 >= KIB {
        format!("{:.1}K", size_f64 / KIB)
    } else {
        format!("{}", size)
    }
}

pub fn get_ollama(args: &Args) -> Result<Ollama, RuChatError> {
    let server = &args.server;
    server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| RuChatError::ArgServerError(server.to_string()))
}

async fn list_models(args: &Args) -> Result<(), RuChatError> {
    let ollama = get_ollama(args)?;
    let models = ollama.list_local_models().await?;
    let max_length = models.iter().map(|m| m.name.len()).max().unwrap_or(0);
    println!("Model name{}Size", " ".repeat(max_length - 8));
    for model in models {
        let size = format_size(model.size);
        let padding_length = max_length - model.name.len() + 6 - size.len();
        let padding = " ".repeat(padding_length);
        println!("{}{}{}", model.name, padding, size);
    }
    Ok(())
}

#[derive(Parser, Debug, Clone)]
pub struct PullArgs {
    #[clap(short, long)]
    pub(crate) model: String,
}

async fn pull_model(args: &Args, pull_args: &PullArgs) -> Result<(), RuChatError> {
    let ollama = get_ollama(args)?;
    let model_name = get_model_name(&ollama, &pull_args.model).await?;
    ollama.pull_model(model_name, false).await?;
    Ok(())
}

pub async fn handle_request(args: &Args) -> Result<(), RuChatError> {
    let default = Commands::Ask(AskArgs::default());
    match args.command.as_ref().unwrap_or(&default) {
        Commands::Ask(ask_args) => ask(get_ollama(args)?, ask_args).await?,
        Commands::Chat(chat_args) => chat(get_ollama(args)?, chat_args).await?,
        Commands::Embed(embed_args) => embed(get_ollama(args)?, embed_args).await?,
        Commands::Func(func_args) => func(get_ollama(args)?, func_args).await?,
        Commands::FuncStruct(func_args) => func_struct(get_ollama(args)?, func_args).await?,
        Commands::List => list_models(args).await?,
        Commands::Pull(pull_args) => pull_model(args, pull_args).await?,
        Commands::Query(query_args) => query(query_args).await?,
    }
    Ok(())
}
