use crate::chroma::r#where::{
    filter_get_response, where_needs_client_side_eval, with_metadata_included,
};
use crate::chroma::{ChromaClientConfigArgs, WhereArgs};
use crate::{Result, RuChatError, retry_transient};
use clap::Parser;
use log::info;
use serde_json::Value;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ChromaDeleteArgs {
    /// The name of the collection to delete from (or delete entirely — see --force).
    #[arg(short, long)]
    collection: String,

    /// With --ids or --where: bypasses confirmation prompts before deleting those specific
    /// records. Alone (no --ids, no --where): deletes the ENTIRE collection, not just its
    /// contents — irreversible.
    #[arg(short, long)]
    force: bool,

    /// Comma separated list of document IDs to delete.
    #[arg(short, long)]
    ids: Option<String>,

    /// Optional limit on the number of records to delete.
    #[arg(short, long)]
    limit: Option<u32>,

    #[command(flatten)]
    client_config: ChromaClientConfigArgs,

    #[command(flatten)]
    r#where: WhereArgs,
}

impl ChromaDeleteArgs {
    pub(crate) async fn delete(&self, cfg: &Value) -> Result<()> {
        let client = self
            .client_config
            .create_client(cfg)
            .await
            .map_err(RuChatError::AnyhowError)?;

        // Parse optional target filters
        let ids = super::parse_ids(&self.ids);

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

            // See `where.rs::where_needs_client_side_eval`'s doc comment: a `CONTAINS` on a
            // scalar metadata field (e.g. `file CONTAINS 'x'`) can't be evaluated by Chroma's
            // delete endpoint any more than by get/query — handing it the doomed filter would
            // silently delete nothing at all. Unlike get/query, delete has no "filter the
            // response" step to fall back on, so the fix here is to resolve which IDs actually
            // match client-side first (a `get`, scoped to `ids` if given, with metadata forced
            // into the response so there's something to evaluate), then delete that explicit
            // ID list instead of passing `where` straight through.
            if where_clause
                .as_ref()
                .is_some_and(where_needs_client_side_eval)
            {
                let w = where_clause.as_ref().unwrap().clone();
                let include = Some(with_metadata_included(None));
                let mut get_result = retry_transient!(async {
                    collection_handle
                        .get(ids.clone(), None, None, None, include.clone())
                        .await
                        .map_err(RuChatError::ChromaHttpClientError)
                })?;

                filter_get_response(&mut get_result, &w, 0, self.limit.map(|l| l as usize));

                if get_result.ids.is_empty() {
                    info!("No records matched --where; nothing deleted.");
                    return Ok(());
                }

                let matched = get_result.ids.len();
                retry_transient!(async {
                    collection_handle
                        .delete(Some(get_result.ids.clone()), None, None)
                        .await
                        .map_err(RuChatError::ChromaHttpClientError)
                })?;

                info!("Deleted {matched} record(s) matching --where");
            } else {
                let limit = match where_clause {
                    None => None,
                    _ => self.limit,
                };

                retry_transient!(async {
                    collection_handle
                        .delete(ids.clone(), where_clause.clone(), limit)
                        .await
                        .map_err(RuChatError::ChromaHttpClientError)
                })?;

                info!("Delete with ids and where");
            }
        } else if self.force {
            // Original behavior: Delete the entire collection via the client
            retry_transient!(async {
                client
                    .delete_collection(&self.collection)
                    .await
                    .map_err(RuChatError::ChromaHttpClientError)
            })?;

            info!("Deleted entire collection: {}", self.collection);
        } else {
            info!(
                "Use --force to delete the entire collection, or provide --ids or --where to delete specific records"
            );
        }
        Ok(())
    }
}
