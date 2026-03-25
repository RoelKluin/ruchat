// src/cli/config.rs  (updated with merge helper)
use crate::{Result, RuChatError};
use clap::Parser;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

#[derive(Parser, Debug, Clone, Default, PartialEq, Deserialize)]
pub(crate) struct ConfigArgs {
    /// Path to config file (JSON). Defaults to ~/.config/ruchat/config.json or ./ruchat.json
    #[arg(long, env = "RUCHAT_CONFIG", help_heading = "Configuration")]
    config: Option<PathBuf>,

    /// Profile name inside config (default: "default")
    #[arg(
        long,
        env = "RUCHAT_PROFILE",
        default_value_t = String::from("default"),
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

        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(home).join(".config/ruchat/config.json");
            if p.exists() {
                return Ok(p);
            }
        }
        Ok(PathBuf::from("ruchat.json"))
    }

    // Merge config into target (CLI overrides win)
    pub(crate) fn merge_into(&self, config: Value, target: &mut Value) {
        if let Value::Object(c) = config {
            if let Value::Object(t) = target {
                for (k, v) in c {
                    if !v.is_null() {
                        t.insert(k, v);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_config_json_profile() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("ruchat.json");

        let content = r#"
        {
          "profiles": {
            "default": {
              "chroma": { "server": "http://localhost:8001", "token": "test-token" },
              "ollama": { "server": "http://localhost:11435" }
            },
            "prod": {
              "chroma": { "server": "https://chroma.example.com" }
            }
          }
        }"#;

        fs::write(&config_path, content).unwrap();

        let args = ConfigArgs {
            config: Some(config_path),
            profile: "default".into(),
        };

        let val = args.load().await.unwrap();

        assert_eq!(val["chroma"]["server"], "http://localhost:8001");
        assert_eq!(val["chroma"]["token"], "test-token");
        assert_eq!(val["ollama"]["server"], "http://localhost:11435");
    }
}
