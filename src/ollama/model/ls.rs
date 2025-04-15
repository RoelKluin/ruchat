use crate::ollama::init;
use crate::error::RuChatError;
use crate::args::Args;

/// pretty print the size of a model
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

/// subcommand to list all models
pub(crate) async fn list(args: &Args) -> Result<(), RuChatError> {
    let ollama = init(args)?;
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
