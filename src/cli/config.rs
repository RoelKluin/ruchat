// src/cli/config.rs
use crate::{Result, RuChatError};
use clap::Parser;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub(crate) struct ConfigArgs {
    /// Path to config file (JSON). Defaults to ~/.config/ruchat/config.json or ./ruchat.json
    #[arg(long, env = "RUCHAT_CONFIG", help_heading = "Configuration")]
    config: Option<PathBuf>,

    /// Profile name inside config (default: "default")
    #[arg(
        long,
        env = "RUCHAT_PROFILE",
        default_value = "default",
        help_heading = "Configuration"
    )]
    profile: String,
}

impl ConfigArgs {
    pub(crate) async fn load(&self) -> Result<Value> {
        let path = self.config_path()?;

        if !path.exists() {
            return Ok(Value::Object(Default::default()));
        }

        let content = fs::read_to_string(&path)
            .await
            .map_err(|e| RuChatError::InternalError(format!("Failed to read {path:?}: {e}")))?;

        let full: Value = serde_json::from_str(&content)
            .map_err(|e| RuChatError::InternalError(format!("Invalid JSON in {path:?}: {e}")))?;

        // Support both flat config and {"profiles": {"default": {...}}}
        if let Some(profiles) = full.get("profiles").and_then(|p| p.as_object()) {
            if let Some(profile) = profiles.get(&self.profile) {
                return Ok(profile.clone());
            }
        }

        Ok(full)
    }

    fn config_path(&self) -> Result<PathBuf> {
        if let Some(p) = &self.config {
            return Ok(p.clone());
        }

        // Minimal home detection without extra crate
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(home).join(".config/ruchat/config.json");
            if p.exists() {
                return Ok(p);
            }
        }

        // Fallback to cwd
        Ok(PathBuf::from("ruchat.json"))
    }
}
