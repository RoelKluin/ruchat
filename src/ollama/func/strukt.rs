use crate::error::RuChatError;
use crate::io::Io;
use crate::ollama::model::get_name;
use clap::Parser;
use ollama_rs::models::ModelOptions;
use ollama_rs::{
    Ollama,
    coordinator::Coordinator,
    generation::{
        chat::ChatMessage,
        parameters::{FormatType, JsonSchema, JsonStructure},
    },
};
use serde::Deserialize;
use std::path::PathBuf;

/// Command-line arguments for querying a model using structured functions.
///
/// This struct defines the arguments required to query a model
/// using structured functions, including model details.
#[derive(Parser, Debug, Clone, PartialEq)]
pub struct FuncStructArgs {
    /// The model to use for the structured function query.
    #[clap(short, long, default_value = "qwen2.5-coder:14b")]
    pub(crate) model: String,
}

/// Get the weather for a given city.
///
/// This function retrieves the weather information for a specified city
/// using an external weather service.
///
/// # Parameters
///
/// - `city`: The city to get the weather for.
///
/// # Returns
///
/// A `Result` containing the weather information as a `String` or an error.
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

/// Get the available space in bytes for a given path.
///
/// This function retrieves the available disk space for a specified path.
///
/// # Parameters
///
/// - `path`: The path to check available space for.
///
/// # Returns
///
/// A `Result` containing the available space as a `String` or an error.
#[ollama_rs::function]
async fn get_available_space(
    path: PathBuf,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    Ok(fs2::available_space(path).map_or_else(
        // Note: this will let LLM handle the error. Return `Err` if you want to bubble it up.
        |err| format!("failed to get available space: {err}"),
        |space| space.to_string(),
    ))
}

/// Subcommand to run a structured function using a model.
///
/// This function connects to a model using the provided arguments,
/// sets up a coordinator with structured functions, and allows the user
/// to enter prompts to query the model.
///
/// # Parameters
///
/// - `ollama`: The Ollama client for generating responses.
/// - `args`: The command-line arguments for the structured function query.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn func_struct(ollama: Ollama, args: &FuncStructArgs) -> Result<(), RuChatError> {
    // browserless requires an BROWSERLESS_TOKEN=... environment variable
    let history = vec![];
    let model_name = get_name(&ollama, &args.model).await?;

    let format = FormatType::StructuredJson(JsonStructure::new::<Weather>());

    let mut coordinator = Coordinator::new(ollama, model_name.to_string(), history)
        .add_tool(get_weather)
        .add_tool(get_available_space)
        .format(format)
        .options(ModelOptions::default().temperature(0.0));

    let mut cio = Io::new();
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
