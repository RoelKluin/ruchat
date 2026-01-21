pub(crate) mod strukt;
use crate::error::Result;
use crate::io::Io;
use crate::ollama::OllamaArgs;
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
pub(crate) async fn func(args: OllamaArgs) -> Result<()> {
    let history = vec![];
    let ollama = args.init()?;
    let model = args.get_model(&ollama, "").await?;
    let mut coordinator = Coordinator::new(ollama, model, history)
        .options(ModelOptions::default().num_ctx(16384))
        .add_tool(Calculator {})
        .add_tool(DDGSearcher::new())
        .add_tool(Scraper {})
        .add_tool(StockScraper::new())
        .add_tool(Browserless {});
    // browserless requires an BROWSERLESS_TOKEN=... environment variable

    let mut cio = Io::new();
    cio.write_line("Enter prompt or 'q' to quit:").await?;
    loop {
        cio.write("\n> ").await?;
        let input = cio.read_line().await?;
        if input.eq_ignore_ascii_case("q") {
            break;
        }

        let response = coordinator
            .chat(vec![ChatMessage::user(input)])
            .await
            .unwrap();
        cio.write_line(&response.message.content).await?;
    }
    Ok(())
}
