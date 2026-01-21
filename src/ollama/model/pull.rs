use crate::error::Result;
use crate::ollama::OllamaArgs;

/// Subcommand to pull a model from the main Ollama server.
///
/// This function connects to the Ollama server, retrieves the specified
/// model, and pulls it to the local environment.
///
/// # Parameters
///
/// - `args`: The command-line arguments containing the server and model information.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn pull(args: OllamaArgs) -> Result<()> {
    let ollama = args.init()?;
    let model = args.get_model(&ollama, "").await?;
    ollama.pull_model(model, false).await?;
    Ok(())
}
