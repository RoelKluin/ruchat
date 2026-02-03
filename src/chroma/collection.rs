use anyhow::Result;
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
        let collection: ChromaCollection = client
            .get_or_create_collection(self.collection.as_str(), metadata)
            .await?;
        Ok(collection)
    }
    pub(crate) async fn get_collection(&self, client: &ChromaClient) -> Result<ChromaCollection> {
        let collection: ChromaCollection = client.get_collection(self.collection.as_str()).await?;
        Ok(collection)
    }
    pub(crate) fn name(&self) -> &str {
        self.collection.as_str()
    }
}
