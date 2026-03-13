use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::{Result, RuChatError};
use chroma_types::Metadata;
use clap::Parser;
use log::info;

/// Command-line arguments for modifying a Chroma collection.
///
/// Allows renaming a collection or updating its metadata.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ModifyArgs {
    /// The new name for the collection.
    #[arg(short, long)]
    new_name: Option<String>,

    /// The new metadata for the collection as a JSON string.
    /// Example: '{"version": "2.0", "description": "updated"}'
    #[arg(short, long)]
    metadata: Option<String>,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    client: ChromaClientConfigArgs,
}

impl ModifyArgs {
    pub(crate) async fn modify(&self) -> Result<()> {
        let client = self
            .client
            .create_client()
            .map_err(RuChatError::AnyhowError)?;

        // Note: modify() requires a mutable reference to the collection handle
        let mut collection = self.collection.get_collection(&client, "default").await?;

        // 1. Parse Metadata if provided
        let metadata_map: Option<Metadata> = if let Some(ref m) = self.metadata {
            let parsed: Metadata = serde_json::from_str(m)
                .map_err(|e| RuChatError::InternalError(format!("Invalid metadata JSON: {}", e)))?;
            Some(parsed)
        } else {
            None
        };

        // 2. Perform the modification
        // We pass the name and metadata. At least one should be Some for an effect.
        collection
            .modify(self.new_name.as_deref(), metadata_map)
            .await
            .map_err(RuChatError::ChromaHttpClientError)?;

        // 3. Log the response (the collection state itself after modification)
        info!("Modify result: {:?}", collection);

        Ok(())
    }
}
