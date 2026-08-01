use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::{retry_transient, Result, RuChatError};
use clap::Parser;
use log::info;
use serde_json::Value;

/// Command-line arguments for forking a Chroma collection.
///
/// Creates a shallow copy of an existing collection under a new name.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ForkArgs {
    /// The name of the new collection to be created.
    #[arg(short, long)]
    new_name: String,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    client: ChromaClientConfigArgs,
}

impl ForkArgs {
    pub(crate) async fn fork(&self, cfg: &Value) -> Result<()> {
        let client = self
            .client
            .create_client(cfg)
            .await
            .map_err(RuChatError::AnyhowError)?;

        // Get the source collection handle
        let collection = self.collection.get_collection(&client, "default").await?;

        // Perform the fork operation
        let forked_collection = retry_transient!(async {
            collection
                .fork(&self.new_name)
                .await
                .map_err(RuChatError::ChromaHttpClientError)
        })?;

        // Log the resulting collection handle
        info!("Fork result: {:?}", forked_collection);

        Ok(())
    }
}
