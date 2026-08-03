use crate::{retry_transient, Result, RuChatError};
use chroma::types::{Metadata, Schema};
use chroma::{ChromaCollection, ChromaHttpClient};
use clap::Parser;
use serde::Deserialize;

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
}

impl Default for ChromaCollectionConfigArgs {
    fn default() -> Self {
        Self {
            collection: "default".to_string(),
        }
    }
}
