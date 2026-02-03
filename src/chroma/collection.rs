use crate::error::{Result, RuChatError};
use chromadb::collection::ChromaCollection;
use chromadb::ChromaClient;
use clap::Parser;
use serde_json::{Map, Value};

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ChromaCollectionConfigArgs {
    #[arg(short = 'c', long, default_value = "")]
    collection: String,
}

impl ChromaCollectionConfigArgs {
    /// Create a collection in the chroma database
    pub(crate) async fn get_or_create_collection(
        &self,
        client: &ChromaClient,
        metadata: Option<Map<String, Value>>,
    ) -> Result<ChromaCollection> {
        if self.collection.is_empty() {
            return Err(RuChatError::NoCollectionSpecified);
        }
        let collection: ChromaCollection = client
            .get_or_create_collection(self.collection.as_str(), metadata)
            .await?;
        Ok(collection)
    }
    pub(crate) async fn get_collection(
        &self,
        client: &ChromaClient,
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
        let collection: ChromaCollection = client.get_collection(collection_name).await?;
        Ok(collection)
    }
    pub(crate) fn set_default_name(&mut self, name: &str) {
        if self.collection.is_empty() {
            self.collection = name.to_string();
        }
    }
    pub(crate) fn name(&self) -> &str {
        self.collection.as_str()
    }
}
