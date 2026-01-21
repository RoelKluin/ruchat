use crate::error::RuChatError;
use crate::ollama::ServerArgs;

/// Pretty print the size of a model.
///
/// This function formats the size of a model in a human-readable
/// format, using units such as K (kilobytes), M (megabytes), and G (gigabytes).
///
/// # Parameters
///
/// - `size`: The size of the model in bytes.
///
/// # Returns
///
/// A `String` representing the formatted size.
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

/// Subcommand to list all models.
///
/// This function connects to the Ollama server, retrieves the list
/// of local models, and prints their names and sizes in a formatted
/// table.
///
/// # Parameters
///
/// - `args`: The command-line arguments containing the server information.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn list(args: ServerArgs) -> Result<(), RuChatError> {
    let ollama = args.init()?;
    let models: Vec<_> = ollama.list_local_models().await?;
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
