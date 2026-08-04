pub(crate) mod collection;

use crate::{Result, RuChatError};
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;

pub(crate) use collection::SqliteVecClient;

/// `--vector-provider sqlite-vec`'s connection config — mirrors
/// `ChromaClientConfigArgs`'s role (resolve a concrete client from CLI/config),
/// but a SQLite-vec "client" is just an open file, not a network endpoint.
#[derive(Parser, Debug, Clone, PartialEq, Deserialize, Default)]
pub(crate) struct SqliteVecClientConfigArgs {
    /// Path to the SQLite database file backing this vector store — created if it
    /// doesn't exist yet. Required when `--vector-provider sqlite-vec` is selected.
    #[arg(long, help_heading = "SQLite-vec")]
    sqlite_vec_path: Option<String>,
}

impl SqliteVecClientConfigArgs {
    pub(crate) fn update_from_json(&mut self, json: &serde_json::Value) -> Result<()> {
        if let Some(path) = json.get("sqlite_vec_path").and_then(|v| v.as_str()) {
            self.sqlite_vec_path = Some(path.to_string());
        }
        Ok(())
    }

    /// Opens (creating if absent) the SQLite file at `sqlite_vec_path`, registering the
    /// sqlite-vec extension on the connection. Blocking file/extension-load work is moved
    /// onto a blocking thread — this is called from async call sites (`EmbedArgs`,
    /// `Orchestrator::new`) that must not stall the executor.
    pub(crate) async fn create_client(&self) -> Result<SqliteVecClient> {
        let path = self.sqlite_vec_path.clone().ok_or_else(|| {
            RuChatError::Is(
                "--sqlite-vec-path is required when --vector-provider is sqlite-vec".into(),
            )
        })?;
        let path = PathBuf::from(path);
        tokio::task::spawn_blocking(move || SqliteVecClient::open(&path))
            .await
            .map_err(|e| RuChatError::InternalError(e.to_string()))?
    }
}
