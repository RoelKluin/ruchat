use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs, MetadataArgs};
use crate::Result;
use clap::Parser;

/// Command-line arguments for creating data in a Chroma database.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ChromaCreateArgs {
    /// Chroma schema, a JSON string defining the schema for the collection.
    #[arg(short, long)]
    schema: Option<String>,

    #[command(flatten)]
    client: ChromaClientConfigArgs,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    metadata: MetadataArgs,
}

impl ChromaCreateArgs {
    /// Creates data into a Chroma database.
    ///
    /// This function connects to a Chroma database using the provided
    /// arguments, parses the metadata, and creates a collection with the specified name and
    /// metadata.
    pub(crate) async fn create(&self) -> Result<()> {
        let client = self.client.create_client()?;
        let name = self.collection.name();
        let schema = self.schema.as_ref().map(|s| serde_json::from_str(s)).transpose()?;
        let metadata = self.metadata.parse()?;

        client.create_collection(name, schema, metadata).await?;
        Ok(())
    }
}
