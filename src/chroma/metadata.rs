use crate::chroma::ChromaClientConfigArgs;
use crate::chroma::ChromaCollectionConfigArgs;
use crate::error::Result;
use clap::Parser;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct MetadataArgs {
    /// Chroma client configuration
    #[clap(flatten)]
    client: ChromaClientConfigArgs,
    /// Chroma collection configuration
    #[clap(flatten)]
    collection: ChromaCollectionConfigArgs,
}

impl MetadataArgs {
    pub(crate) async fn get_metadata(&self) -> Result<()> {
        let client = self.client.create_client().await?;
        let collection = self.collection.get_collection(&client, "default").await?;

        if let Some(map) = collection.metadata() {
            println!("{}", serde_json::to_string_pretty(&map)?);
        } else {
            println!("No metadata found for the collection.");
        }
        Ok(())
    }
}
