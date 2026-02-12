use crate::chroma::parse_metadata;
use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::Result;
use clap::Parser;

/// Command-line arguments for creating data in a Chroma database.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ChromaCreateArgs {
    /// Chroma update metadata, comma separated key:value pairs.
    #[arg(short, long)]
    metadata: Option<String>,

    #[command(flatten)]
    client: ChromaClientConfigArgs,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,
}

impl ChromaCreateArgs {
    /// Creates data into a Chroma database.
    ///
    /// This function connects to a Chroma database using the provided
    /// arguments, parses the metadata, and creates a collection with the specified name and
    /// metadata.
    pub(crate) async fn create(&self) -> Result<()> {
        let client = self.client.create_client().await?;
        let name = self.collection.name();
        let metadata = parse_metadata(&self.metadata)?;

        client.create_collection(name, metadata, false).await?;
        Ok(())
    }
}
