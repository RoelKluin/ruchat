use crate::chroma::{ChromaClientConfigArgs, WhereArgs};
use crate::{Result, RuChatError};
use clap::Parser;
use log::info;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ChromaDeleteArgs {
    /// The name of the collection.
    #[arg(short, long)]
    collection: String,

    #[arg(short, long)]
    force: bool,

    /// Comma separated list of document IDs to delete from the collection.
    #[arg(short, long)]
    ids: Option<String>,

    #[command(flatten)]
    client_config: ChromaClientConfigArgs,

    #[command(flatten)]
    r#where: WhereArgs,
}

impl ChromaDeleteArgs {
    pub(crate) async fn delete(&self) -> Result<()> {
        let client = self
            .client_config
            .create_client()
            .map_err(RuChatError::ChromaError)?;

        // Parse optional target filters
        let ids: Option<Vec<String>> = self
            .ids
            .as_ref()
            .map(|s| s.split(',').map(|id| id.trim().to_string()).collect());

        let where_clause = self.r#where.parse()?;

        // Logic: If IDs or a Where clause are provided, delete specific records.
        // Otherwise, delete the entire collection metadata and data.
        if ids.is_some() || where_clause.is_some() {
            // Get collection handle to perform record-level deletion
            // We use None for the embedding function as it's not needed for deletion
            let collection_handle = client
                .get_collection(&self.collection)
                .await
                .map_err(RuChatError::ChromaHttpClientError)?;

            collection_handle
                .delete(ids, where_clause)
                .await
                .map_err(RuChatError::ChromaHttpClientError)?;

            info!("Delete with ids and where");
        } else if self.force {
            // Original behavior: Delete the entire collection via the client
            client
                .delete_collection(&self.collection)
                .await
                .map_err(RuChatError::ChromaHttpClientError)?;

            info!("Deleted entire collection: {}", self.collection);
        } else {
            info!(
                "Use --force to delete the entire collection, or provide --ids or --where to delete specific records"
            );
        }
        Ok(())
    }
}
