use crate::error::RuChatError;
use serde_json::Value;
use ollama_rs::models::ModelOptions;

async fn read_options_file(options_path: &str) -> Result<Value, RuChatError> {
    let options = std::fs::read_to_string(options_path)?;
    let options = serde_json::from_str(&options)?;
    Ok(options)
}

/// Get model options for prompt handling from a JSON file
pub(crate) async fn get_options(config: &Option<String>) -> Result<ModelOptions, RuChatError> {
    if let Some(options_path) = config {
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
