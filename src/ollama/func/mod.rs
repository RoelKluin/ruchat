pub(crate) mod strukt;
use crate::io::Io;
use crate::error::RuChatError;
use crate::ollama::model::get_name;
use clap::Parser;
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
    Ollama,
};

/// query model using a function
#[derive(Parser, Debug, Clone)]
pub struct FuncArgs {
    #[clap(short, long, default_value = "qwen2.5-coder:14b")]
    pub(crate) model: String,
}

/// subcommand to run a function using a model
pub(crate) async fn func(ollama: Ollama, args: &FuncArgs) -> Result<(), RuChatError> {
    let history = vec![];
    let model_name = get_name(&ollama, &args.model).await?;
    let mut coordinator = Coordinator::new(ollama, model_name.to_string(), history)
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
        let input = cio.read_line(true).await?;
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
