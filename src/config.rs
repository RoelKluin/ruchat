use anyhow::{anyhow, Result};
use serde_json::Value;

pub async fn read_config_file(config_path: &str) -> Result<Value> {
    let config_content = std::fs::read_to_string(config_path)?;
    serde_json::from_str(&config_content)
        .map_err(|e| anyhow!("Failed to parse config file at {config_path}: {e}"))
}
