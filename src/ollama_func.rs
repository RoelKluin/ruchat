use crate::args::Args;
use crate::chat_io::ChatIO;
use crate::error::RuChatError;
use crate::ollama::get_model_name;
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
    tool_group, Ollama,
};

pub(crate) async fn func(ollama: Ollama, args: &Args) -> Result<(), RuChatError> {
    // browserless requires an BROWSERLESS_TOKEN=... environment variable
    let tools = tool_group![
        Calculator {},
        DDGSearcher::new(),
        Scraper {},
        StockScraper::new(),
        Browserless {},
    ];
    let history = vec![];
    let model_name = get_model_name(&ollama, &args.model).await?;
    let mut coordinator =
        Coordinator::new_with_tools(ollama, model_name.to_string(), history, tools)
            .options(ModelOptions::default().num_ctx(16384));

    let mut cio = ChatIO::new();
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
