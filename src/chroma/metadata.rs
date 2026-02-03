use crate::chroma::ChromaClientConfigArgs;
use crate::chroma::ChromaCollectionConfigArgs;
use crate::error::Result;
use clap::Parser;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct MetadataArgs {
    /// Chroma client configuration
    #[clap(flatten)]
    chroma_client_config: ChromaClientConfigArgs,
    /// Chroma collection configuration
    #[clap(flatten)]
    chroma_collection_config: ChromaCollectionConfigArgs,
}

impl MetadataArgs {
    pub(crate) async fn get_metadata(&self) -> Result<()> {
        let client = self.chroma_client_config.create_client().await?;
        let collection = self
            .chroma_collection_config
            .get_collection(&client)
            .await?;

        if let Some(map) = collection.metadata() {
            println!("{}", serde_json::to_string_pretty(&map)?);
        } else {
            println!("No metadata found for the collection.");
        }
        Ok(())
    }
}
