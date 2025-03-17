use crate::chat_io::ChatIO;
use crate::error::RuChatError;
use crate::ollama::get_model_name;
use clap::Parser;
use ollama_rs::models::ModelOptions;
use ollama_rs::{
    coordinator::Coordinator,
    generation::{
        chat::ChatMessage,
        parameters::{FormatType, JsonSchema, JsonStructure},
    },
    tool_group, Ollama,
};
use serde::Deserialize;

#[derive(Parser, Debug, Clone)]
pub struct FuncStructArgs {
    #[clap(short, long, default_value = "qwen2.5-coder:32b")]
    pub(crate) model: String,
}

/// Get the weather for a given city.
///
/// * city - City to get the weather for.
#[ollama_rs::function]
async fn get_weather(city: String) -> Result<String, Box<dyn std::error::Error + Sync + Send>> {
    println!("Get weather function called for {city}");
    Ok(
        reqwest::get(format!("https://wttr.in/{city}?format=%C+%t+%w+%P"))
            .await?
            .text()
            .await?,
    )
}

pub(crate) async fn func_struct(ollama: Ollama, args: &FuncStructArgs) -> Result<(), RuChatError> {
    // browserless requires an BROWSERLESS_TOKEN=... environment variable
    let tools = tool_group![get_weather];
    let history = vec![];
    let model_name = get_model_name(&ollama, &args.model).await?;

    let format = FormatType::StructuredJson(JsonStructure::new::<Weather>());

    let mut coordinator =
        Coordinator::new_with_tools(ollama, model_name.to_string(), history, tools)
            .format(format)
            .options(ModelOptions::default().temperature(0.0));

    let mut cio = ChatIO::new();
    cio.write_line("Ask about the weather somewhere or 'q' to quit:")
        .await?;
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

#[allow(dead_code)]
#[derive(JsonSchema, Deserialize, Debug)]
struct Weather {
    city: String,
    temperature_units: String,
    temperature: f32,
    wind_units: String,
    wind: f32,
    pressure_units: String,
    pressure: f32,
}
