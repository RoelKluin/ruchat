use crate::args::{Args, Commands};
use crate::error::RuChatError;
use crate::ollama_chat::chat;
use crate::ollama_query::query;
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

pub async fn handle_request(args: Args) -> Result<(), RuChatError> {
    let server = &args.server;
    let ollama: Ollama = server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| RuChatError::ArgServerError(server.to_string()))?;

    match args.command {
        Some(Commands::Query(ref query_args)) => query(ollama, &args, Some(query_args)).await?,
        Some(Commands::Chat(ref chat_args)) => chat(ollama, &args, chat_args).await?,
        Some(Commands::List) => {
            let models = ollama.list_local_models().await?;
            let max_length = models.iter().map(|m| m.name.len()).max().unwrap_or(0);
            println!("Model name{}Size", " ".repeat(max_length - 8));
            for model in models {
                let size = format_size(model.size);
                let padding_length = max_length - model.name.len() + 6 - size.len();
                let padding = " ".repeat(padding_length);
                println!("{}{}{}", model.name, padding, size);
            }
        }
        None => query(ollama, &args, None).await?,
    }
    Ok(())
}
