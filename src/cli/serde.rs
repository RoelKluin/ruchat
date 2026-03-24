use crate::agent::manager::Manager; // generic approach is better, but explicit is easier here
use crate::cli::config::ConfigArgs;
use crate::utils::error::Result;
use crate::RuChatError;
use serde_json::Value;
use std::path::Path;
use tokio::fs;

pub(crate) async fn load_merged_config(config_args: &ConfigArgs) -> Result<Value> {
    let mut base = config_args.load().await?;

    // Future: env var overrides, CLI flags will be merged on top in each subcommand
    Ok(base)
}

pub(crate) async fn read_config_file(config_path: &str) -> Result<Value> {
    let content = fs::read_to_string(config_path).await?;
    let content = serde_json::from_str(&content)?;
    Ok(content)
}

// New
pub(crate) async fn load_manager(path: &str) -> Result<Manager> {
    if !Path::new(path).exists() {
        return Ok(Manager::default());
    }
    let content = fs::read_to_string(path).await?;
    let manager: Manager = serde_json::from_str(&content)?;
    Ok(manager)
}

pub(crate) async fn save_manager(path: &str, manager: &Manager) -> Result<()> {
    let content = serde_json::to_string_pretty(manager)?;
    fs::write(path, content).await?;
    Ok(())
}
