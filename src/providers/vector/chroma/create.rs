use crate::Result;
use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs, MetadataArgs};
use clap::Parser;
use serde_json::Value;

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
    pub(crate) async fn create(&self, cfg: &Value) -> Result<()> {
        let client = self.client.create_client(cfg).await?;
        // or there should be a subcommand to create a database if it doesn't exist
        let databases = client.list_databases().await?;
        let mut exists = false;
        let db_name = self
            .client
            .chroma_database
            .clone()
            .unwrap_or("default".to_string());
        for db in databases {
            if db.name == db_name {
                exists = true;
                break;
            }
        }
        if !exists {
            client.create_database(db_name).await?;
        }
        let name = self.collection.name();
        let schema = self
            .schema
            .as_ref()
            .map(|s| serde_json::from_str(s))
            .transpose()?;
        let metadata = self.metadata.parse()?;

        client.create_collection(name, schema, metadata).await?;
        Ok(())
    }
}
