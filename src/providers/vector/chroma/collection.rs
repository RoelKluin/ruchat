use crate::{Result, RuChatError};
use chroma::types::{Metadata, Schema};
use chroma::{ChromaCollection, ChromaHttpClient};
use clap::Parser;
use serde::Deserialize;
use serde_json::Value;

#[derive(Parser, Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct ChromaCollectionConfigArgs {
    #[arg(short = 'c', long, default_value = "", help_heading = "Collection")]
    collection: String,
}

impl ChromaCollectionConfigArgs {
    /// Create a collection in the chroma database
    pub(crate) async fn get_or_create_collection(
        &self,
        client: &ChromaHttpClient,
        schema: Option<Schema>,
        metadata: Option<Metadata>,
    ) -> Result<ChromaCollection> {
        if self.collection.is_empty() {
            return Err(RuChatError::NoCollectionSpecified);
        }
        let collection: ChromaCollection = client
            .get_or_create_collection(self.collection.as_str(), schema, metadata)
            .await?;
        Ok(collection)
    }
    pub(crate) async fn get_collection(
        &self,
        client: &ChromaHttpClient,
        default: &str,
    ) -> Result<ChromaCollection> {
        let collection_name = if self.collection.is_empty() {
            if default.is_empty() {
                return Err(RuChatError::NoCollectionSpecified);
            }
            default
        } else {
            self.collection.as_str()
        };
        let collection = client.get_collection(collection_name).await?;
        Ok(collection)
    }
    pub(crate) fn name(&self) -> &str {
        self.collection.as_str()
    }
    pub(crate) fn update_from_json(&mut self, json: &Value) -> Result<()> {
        if let Some(collection) = json.get("collection") {
            if collection.is_string() {
                self.collection = collection.as_str().unwrap().to_string();
                Ok(())
            } else {
                Err(RuChatError::Is(format!(
                    "Expected 'collection' to be a string in JSON, got {:?}",
                    collection
                )))
            }
        } else {
            Err(RuChatError::Is(
                "Missing 'collection' field in JSON for ChromaCollectionConfigArgs".into(),
            ))
        }
    }
}

impl Default for ChromaCollectionConfigArgs {
    fn default() -> Self {
        Self {
            collection: "default".to_string(),
        }
    }
}
