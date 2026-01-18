use crate::agent::manager::Manager; // generic approach is better, but explicit is easier here
use crate::error::RuChatError;
use serde_json::Value;
use std::path::Path;
use tokio::fs;

pub async fn read_config_file(config_path: &str) -> Result<Value, RuChatError> {
    let content = fs::read_to_string(config_path).await?;
    let content = serde_json::from_str(&content)?;
    Ok(content)
}

// New
pub async fn load_manager(path: &str) -> Result<Manager, RuChatError> {
    if !Path::new(path).exists() {
        return Ok(Manager::default());
    }
    let content = fs::read_to_string(path).await?;
    let manager: Manager = serde_json::from_str(&content)?;
    Ok(manager)
}

pub async fn save_manager(path: &str, manager: &Manager) -> Result<(), RuChatError> {
    let content = serde_json::to_string_pretty(manager)?;
    fs::write(path, content).await?;
    Ok(())
}
