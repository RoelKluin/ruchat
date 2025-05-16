use crate::error::RuChatError;
use ollama_rs::models::ModelOptions;
use serde_json::Value;

/// Reads a JSON file containing model options.
///
/// This function reads the specified JSON file and parses it into a `Value`.
///
/// # Parameters
///
/// - `options`: The path to the JSON file containing model options, or a string
///  representing the options in JSON format.
///
/// # Returns
///
/// A `Result` containing the parsed `Value` or a `RuChatError`.
async fn read_options_file(options: &str) -> Result<Value, serde_json::Error> {
    match std::fs::read_to_string(options) {
        Ok(options) => serde_json::from_str(&options),
        Err(_) => serde_json::from_str(options),
    }
}

/// Get model options for prompt handling from a JSON file.
///
/// This function retrieves model options from a specified JSON configuration
/// file. If no configuration file is provided, it returns the default model
/// options.
///
/// # Parameters
///
/// - `options`: An optional path to the JSON configuration file.
///
/// # Returns
///
/// A `Result` containing the `ModelOptions` or a `RuChatError`.
pub(crate) async fn get_options(options: Option<&str>) -> Result<ModelOptions, RuChatError> {
    if let Some(options_path) = options {
        let mut defaults = serde_json::to_value(ModelOptions::default())?;

        if let Value::Object(ref mut defaults) = defaults {
            let updates = read_options_file(options_path).await?;
            if let Value::Object(config_updates) = updates {
                for (k, v) in config_updates.into_iter() {
                    if defaults.contains_key(&k) && !v.is_null() {
                        defaults[&k] = v.clone();
                    }
                }
            }
        }
        serde_json::from_value(defaults).map_err(RuChatError::SerdeError)
    } else {
        Ok(ModelOptions::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_read_options_file() {
        let path = "test_options.json";
        fs::write(path, r#"{\"option1\": \"value1\"}"#).unwrap();
        let value = read_options_file(path).await.unwrap();
        assert_eq!(value["option1"], "value1");
        fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn test_get_options_with_file() {
        let path = "test_options.json";
        fs::write(path, r#"{\"option1\": \"value1\"}"#).unwrap();
        assert!(get_options(Some(path)).await.is_ok());
        fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn test_get_options_without_file() {
        assert!(get_options(None).await.is_ok());
    }
}
