use crate::{retry_transient, Result, RuChatError};
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
        let name = self.collection.as_str();
        let collection: ChromaCollection = retry_transient!(async {
            client
                .get_or_create_collection(name, schema.clone(), metadata.clone())
                .await
                .map_err(RuChatError::from)
        })?;
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
        let collection = retry_transient!(async {
            client
                .get_collection(collection_name)
                .await
                .map_err(RuChatError::from) // same verification note as above
        })?;
        Ok(collection)
    }
    pub(crate) fn name(&self) -> &str {
        self.collection.as_str()
    }
    pub(crate) fn update_from_json(&mut self, json: &Value) -> Result<()> {
        if let Some(collection) = json.get("collection") {
            if let Some(s) = collection.as_str() {
                self.collection = s.to_string();
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
