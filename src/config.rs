use crate::ollama_error::Error;
use serde_json::Value;

pub async fn read_config_file(config_path: &str) -> Result<Value, Error> {
    let config_content = std::fs::read_to_string(config_path)?;
    serde_json::from_str(&config_content).map_err(Error::ConfigDeserializationError)
}
