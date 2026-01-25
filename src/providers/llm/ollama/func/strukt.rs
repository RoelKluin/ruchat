use crate::Result;
use crate::io::Io;
use crate::ollama::OllamaArgs;
use ollama_rs::models::ModelOptions;
use ollama_rs::{
    coordinator::Coordinator,
    generation::{
        chat::ChatMessage,
        parameters::{FormatType, JsonSchema, JsonStructure},
    },
};
use serde::Deserialize;
use std::path::PathBuf;

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
pub(crate) async fn func_struct(args: OllamaArgs) -> Result<()> {
    // browserless requires an BROWSERLESS_TOKEN=... environment variable
    let history = vec![];
    let (ollama, model) = args.init("").await?;

    let format = FormatType::StructuredJson(Box::new(JsonStructure::new::<Weather>()));

    let mut coordinator = Coordinator::new(ollama, model, history)
        .add_tool(get_weather)
        .add_tool(get_available_space)
        .format(format)
        .options(ModelOptions::default().temperature(0.0));

    let mut cio = Io::new();
    cio.write_line("Ask about the weather somewhere or 'q' to quit:")
        .await?;
    loop {
        cio.write_line("\n> ").await?;
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
