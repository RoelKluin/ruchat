use crate::chroma::ChromaClientConfigArgs;
use crate::{Result, RuChatError, retry_transient};
use chroma::types::{Metadata, MetadataValue};
use clap::Parser;
use serde::Deserialize;
use serde_json::Value;

/// One collection's schema entry from a `db_config.json`-shaped file. Only the fields
/// `chroma-init` actually needs to create/ensure a collection exists — `example_queries`,
/// `notes_on_metadata`, and `metadata_keys` are prompt-facing documentation for the Librarian
/// (see `Context::read_config_file`) and aren't needed here, so they're not modeled.
#[derive(Deserialize)]
struct CollectionSpec {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    embedding_model: Option<String>,
}

#[derive(Deserialize)]
struct DbConfig {
    collections: Vec<CollectionSpec>,
}

/// Builds the collection-level metadata `chroma-init` attaches to each collection it ensures —
/// `description`/`embedding_model` round-tripped from the config file so they're visible via
/// `chroma-ls --long` directly from Chroma, not only from reading `db_config.json` by hand.
/// `None` when a spec carries neither (an empty `Metadata` map is meaningless to send).
fn collection_metadata(spec: &CollectionSpec) -> Option<Metadata> {
    let mut metadata = Metadata::new();
    if let Some(desc) = &spec.description {
        metadata.insert("description".to_string(), MetadataValue::Str(desc.clone()));
    }
    if let Some(model) = &spec.embedding_model {
        metadata.insert(
            "embedding_model".to_string(),
            MetadataValue::Str(model.clone()),
        );
    }
    (!metadata.is_empty()).then_some(metadata)
}

/// Command-line arguments for `ruchat chroma-init`: reads a `db_config.json`-shaped file and
/// ensures every collection it documents actually exists in Chroma, instead of manually running
/// `chroma-create` once per collection. Idempotent — `get_or_create_collection` is a no-op for a
/// collection that already exists, so this is safe to re-run any time the config file changes
/// (e.g. a new collection was added) without disturbing existing ones or their data.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ChromaInitArgs {
    /// Path to the collection-schema config file (same shape as db_config.json).
    #[arg(short = 'f', long, default_value = "db_config.json")]
    file: String,

    #[command(flatten)]
    client: ChromaClientConfigArgs,
}

impl ChromaInitArgs {
    pub(crate) async fn init(&self, cfg: &Value) -> Result<()> {
        let client = self
            .client
            .create_client(cfg)
            .await
            .map_err(RuChatError::AnyhowError)?;

        // Mirrors `ChromaCreateArgs::create`'s same "ensure the database exists first" step —
        // a fresh Chroma instance may not have the configured database yet.
        let db_name = self
            .client
            .chroma_database
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let databases = client.list_databases().await?;
        if !databases.iter().any(|db| db.name == db_name) {
            client.create_database(db_name).await?;
        }

        let raw = tokio::fs::read_to_string(&self.file).await?;
        let db_config: DbConfig = serde_json::from_str(&raw)?;

        for spec in &db_config.collections {
            let metadata = collection_metadata(spec);
            retry_transient!(async {
                client
                    .get_or_create_collection(&spec.name, None, metadata.clone())
                    .await
                    .map_err(RuChatError::ChromaHttpClientError)
            })?;
            println!("ensured collection: {}", spec.name);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_config_parses_the_shipped_db_config_json_shape() {
        // Regression-relevant: this is the exact shape `db_config.json` ships in this repo,
        // including fields `chroma-init` doesn't use (example_queries, notes_on_metadata,
        // metadata_keys) — must not be rejected by strict deserialization.
        let raw = r#"{
            "collections": [
                {
                    "name": "repo_src-all-minilm_l6-v2",
                    "description": "Source code files.",
                    "embedding_model": "all-minilm-l6-v2",
                    "metadata_keys": ["file", "language"],
                    "notes_on_metadata": "some notes",
                    "example_queries": [{"query": "q", "where": "file CONTAINS 'x'"}]
                }
            ],
            "allowed_include_fields": ["distance", "document"],
            "default_n_results": 6
        }"#;
        let parsed: DbConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.collections.len(), 1);
        assert_eq!(parsed.collections[0].name, "repo_src-all-minilm_l6-v2");
        assert_eq!(
            parsed.collections[0].embedding_model.as_deref(),
            Some("all-minilm-l6-v2")
        );
    }

    #[test]
    fn db_config_accepts_a_collection_with_only_a_name() {
        let raw = r#"{"collections": [{"name": "bare"}]}"#;
        let parsed: DbConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.collections[0].name, "bare");
        assert!(parsed.collections[0].description.is_none());
        assert!(parsed.collections[0].embedding_model.is_none());
    }

    #[test]
    fn collection_metadata_includes_description_and_embedding_model_when_present() {
        let spec = CollectionSpec {
            name: "x".to_string(),
            description: Some("desc".to_string()),
            embedding_model: Some("all-minilm-l6-v2".to_string()),
        };
        let metadata = collection_metadata(&spec).expect("expected Some metadata");
        assert_eq!(
            metadata.get("description"),
            Some(&MetadataValue::Str("desc".to_string()))
        );
        assert_eq!(
            metadata.get("embedding_model"),
            Some(&MetadataValue::Str("all-minilm-l6-v2".to_string()))
        );
    }

    #[test]
    fn collection_metadata_is_none_when_the_spec_has_neither_field() {
        let spec = CollectionSpec {
            name: "x".to_string(),
            description: None,
            embedding_model: None,
        };
        assert!(collection_metadata(&spec).is_none());
    }
}
