use crate::chroma::get_metadata;
use anyhow::Result;
use chromadb::collection::ChromaCollection;
use chromadb::ChromaClient;
use clap::Parser;

#[derive(Parser, Debug, Clone, PartialEq)]
pub struct ChromaCollectionConfigArgs {
    #[arg(short = 'c', long, default_value = "default")]
    pub collection: String,

    #[arg(short = 'm', long)]
    pub metadata: Option<String>,
}

impl ChromaCollectionConfigArgs {
    /// Create a collection in the chroma database
    pub async fn get_or_create_collection(
        &self,
        client: &ChromaClient,
    ) -> Result<ChromaCollection> {
        let metadata = get_metadata(&self.metadata)?;
        let collection: ChromaCollection = client
            .get_or_create_collection(self.collection.as_str(), metadata)
            .await?;
        Ok(collection)
    }
}
