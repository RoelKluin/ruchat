use crate::agent::manager::Manager; // generic approach is better, but explicit is easier here
use crate::cli::config::ConfigArgs;
use crate::utils::error::Result;
use serde_json::Value;
use std::path::Path;
use tokio::fs;

pub(crate) async fn load_merged_config(config_args: &ConfigArgs) -> Result<Value> {
    // Config file (with `--profile` selection) is the sole source of subcommand
    // defaults today. Per-flag CLI overrides are NOT merged generically at this
    // layer — each subcommand already applies its own flags over `cfg` downstream,
    // e.g. `ChromaClientConfigArgs::create_client`/`update_from_json`,
    // `OllamaArgs::init`. A generic merge here would require every `*Args` struct
    // to serialize itself to `Value`, which doesn't exist yet; tracked as a
    // follow-up rather than implemented speculatively.
    config_args.load().await
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
