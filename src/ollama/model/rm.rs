use crate::error::Result;
use crate::ollama::OllamaArgs;

/// Subcommand to remove a model from the local Ollama instance.
///
/// This function connects to the local Ollama instance, retrieves the specified
/// model, and removes it from the local environment.
///
/// # Parameters
///
/// - `args`: The command-line arguments containing the server and model information.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn remove(args: OllamaArgs) -> Result<()> {
    let ollama = args.init()?;
    let model = args.get_model(&ollama, "").await?;
    ollama.delete_model(model).await?;
    Ok(())
}
