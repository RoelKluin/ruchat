use crate::{Result, RuChatError};
use ollama_rs::models::ModelOptions;
use serde_json::Value;
use std::collections::HashMap;

/// Reads a JSON file containing model options.
///
/// This function reads the specified JSON file and parses it into a `Value`.
///
/// # Parameters
///
/// - `options`: The path to the JSON file containing model options, or a string
///   representing the options in JSON format.
///
/// # Returns
///
/// A `Result` containing the parsed `Value` or a `RuChatError`.
async fn read_options_file(options: &str) -> Result<Value> {
    match std::fs::read_to_string(options) {
        Ok(options) => serde_json::from_str(&options),
        Err(_) => serde_json::from_str(options),
    }
    .map_err(|e| {
        tracing::error!(error = ?e, "failed to read or parse options: {options}");
        e
    })
    .map_err(RuChatError::SerdeError)
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
pub(crate) async fn get_options(options: &str) -> Result<(ModelOptions, HashMap<String, Value>)> {
    let (defaults, remain) = merge_options_json(options).await?;
    serde_json::from_value(defaults)
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to deserialize JSON into ModelOptions");
            e
        })
        .map_err(RuChatError::SerdeError)
        .map(|opts| (opts, remain))
}

/// Every JSON field name `ollama_rs::models::ModelOptions` actually accepts, confirmed against
/// the crate's builder methods (ollama-rs 0.3.6) rather than derived from
/// `ModelOptions::default()`'s serialized shape: every field is `Option<_>` with
/// `skip_serializing_if = "Option::is_none"`, so a default instance always serializes to `{}`
/// and can't be used to discover the valid key set — see `merge_options_json` below, which used
/// to gate on that empty shape and silently drop every config-supplied option as a result.
const MODEL_OPTION_KEYS: &[&str] = &[
    "num_ctx",
    "temperature",
    "top_k",
    "top_p",
    "repeat_penalty",
    "stop",
    "num_predict",
    "seed",
    "mirostat",
    "mirostat_eta",
    "mirostat_tau",
    "tfs_z",
    "repeat_last_n",
];

/// Merges `options` (a JSON file path or literal JSON string) onto
/// `ModelOptions::default()`'s JSON shape, without deserializing back to
/// `ModelOptions` yet. Shared by `get_options` above and
/// `ModelArgs::build_generation_request`, the latter needing to merge in
/// CLI flag overrides on top before doing the (single) final deserialize —
/// pulled out so that caller doesn't need its own separate
/// `ModelOptions` -> JSON -> `ModelOptions` round trip to get there.
pub(crate) async fn merge_options_json(options: &str) -> Result<(Value, HashMap<String, Value>)> {
    let mut remain = HashMap::new();
    let mut defaults = serde_json::to_value(ModelOptions::default())?;

    if let Value::Object(ref mut defaults) = defaults {
        let updates = read_options_file(options).await?;
        if let Value::Object(config_updates) = updates {
            for (k, v) in config_updates.into_iter() {
                if MODEL_OPTION_KEYS.contains(&k.as_str()) && !v.is_null() {
                    defaults.insert(k, v);
                } else {
                    remain.insert(k, v);
                }
            }
        }
    }
    Ok((defaults, remain))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_read_options_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_options.json");
        fs::write(&path, r#"{"option1": "value1"}"#).unwrap();
        let value = read_options_file(path.to_str().unwrap()).await.unwrap();
        assert_eq!(value["option1"], "value1");
    }

    #[tokio::test]
    async fn test_get_options_with_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_options.json");
        fs::write(&path, r#"{"option1": "value1"}"#).unwrap();
        assert!(get_options(path.to_str().unwrap()).await.is_ok());
    }

    #[tokio::test]
    async fn merge_options_json_applies_known_keys_and_keeps_unknown_ones_in_remain() {
        let (defaults, remain) = merge_options_json(r#"{"seed": 7, "role": "Worker"}"#)
            .await
            .unwrap();
        assert_eq!(defaults["seed"], 7);
        assert_eq!(remain.get("role"), Some(&Value::String("Worker".into())));
        assert!(!remain.contains_key("seed"));
    }

    #[tokio::test]
    async fn test_get_options_without_file() {
        assert!(get_options("{}").await.is_ok());
    }
}
