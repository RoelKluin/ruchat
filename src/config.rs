use crate::error::RuChatError;
use serde_json::Value;

pub async fn read_config_file(config_path: &str) -> Result<Value, RuChatError> {
    let content = std::fs::read_to_string(config_path)?;
    let content = serde_json::from_str(&content)?;
    Ok(content)
}
