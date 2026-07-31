mod strukt;
use crate::io::Io;
use crate::ollama::OllamaArgs;
use crate::Result;
use ollama_rs::models::ModelOptions;
use ollama_rs::{
    coordinator::Coordinator,
    generation::{
        chat::ChatMessage,
        tools::implementations::{
            Browserless,
            Calculator,
            DDGSearcher,
            Scraper,
            StockScraper,
            // SerperSearchToo // seems to have issue and SERPER_API_KEY=... is required
        },
    },
};
use serde_json::Value;
pub(crate) use strukt::func_struct;

/// Subcommand to run a function using a model.
///
/// This function connects to a model using the provided arguments,
/// sets up a coordinator with various tools, and allows the user
/// to enter prompts to query the model.
///
/// # Parameters
///
/// - `ollama`: The Ollama client for generating responses.
/// - `args`: The command-line arguments for the function query.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn func(args: OllamaArgs, cfg: &Value) -> Result<()> {
    let history = vec![];
    let (ollama, model) = args.init("", cfg).await?;
    let mut coordinator = Coordinator::new(ollama, model[0].clone(), history)
        .options(ModelOptions::default().num_ctx(16384))
        .add_tool(Calculator {})
        .add_tool(DDGSearcher::new())
        .add_tool(Scraper {})
        .add_tool(StockScraper::new())
        .add_tool(Browserless {})
        .add_tool(git_log)
        .add_tool(git_blame)
        .add_tool(git_diff);
    // browserless requires an BROWSERLESS_TOKEN=... environment variable

    let mut cio = Io::new();
    cio.write_line("Enter prompt or 'q' to quit:").await?;
    loop {
        cio.write("\n> ").await?;
        let input = cio.read_line().await?;
        if input.eq_ignore_ascii_case("q") {
            break;
        }
        match coordinator.chat(vec![ChatMessage::user(input)]).await {
            Ok(response) => cio.write_line(&response.message.content).await?,
            Err(e) => cio.write_error_line(&format!("Chat error: {e}")).await?,
        }
    }
    Ok(())
}

// Native-tool wrapper around the read-only git helpers shared with the
// orchestrator's structured tool dispatch (`core::orchestrator::handle_structured_tool`).
// This is the "single dispatch path" for these three tools - same
// implementation, different calling convention per caller.
/// Get the git log for a repository, optionally scoped to a path and capped at max_count (default
/// 20).
#[ollama_rs::function]
async fn git_log(
    path: Option<String>,
    max_count: Option<u32>,
) -> std::result::Result<String, Box<dyn std::error::Error + Send + Sync>> {
    crate::orchestrator::git::git_log(path.as_deref(), max_count)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
}

/// Get the git blame for a single file.
#[ollama_rs::function]
async fn git_blame(
    path: String,
) -> std::result::Result<String, Box<dyn std::error::Error + Send + Sync>> {
    crate::orchestrator::git::git_blame(&path)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
}

/// Get the git diff for a repository, optionally scoped to a path and/or staged.
#[ollama_rs::function]
async fn git_diff(
    path: Option<String>,
    staged: Option<bool>,
) -> std::result::Result<String, Box<dyn std::error::Error + Send + Sync>> {
    crate::orchestrator::git::git_diff(path.as_deref(), staged.unwrap_or(false))
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
}
