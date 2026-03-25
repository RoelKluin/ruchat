use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::{Result, RuChatError};
use chroma::types::IndexStatusResponse;
use clap::Parser;
use serde::Serialize;

/// Formatting helper to group collection data for JSON output
#[derive(Serialize)]
struct CollectionInfo {
    name: String,
    id: String,
    database: String,
    tenant: String,
    version: i32,
    count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<chroma_types::Metadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema: Option<chroma_types::Schema>,
    #[serde(skip_serializing_if = "Option::is_none")]
    indexing_status: Option<IndexStatusResponse>,
    storage_prefix: String,
}

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ChromaLsArgs {
    /// Show detailed information including schema and indexing status.
    #[arg(short, long)]
    pub long: bool,

    /// Output in JSON format for scripting.
    #[arg(short, long)]
    pub json: bool,

    #[command(flatten)]
    pub collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    pub client: ChromaClientConfigArgs,
}

impl ChromaLsArgs {
    pub(crate) async fn ls(&self) -> Result<()> {
        let client = self
            .client
            .create_client()
            .await
            .map_err(RuChatError::AnyhowError)?;

        // Determine if we are listing all or one specific collection
        let collections = if self.collection.name().is_empty() {
            client
                .list_collections(100, None)
                .await
                .map_err(RuChatError::ChromaHttpClientError)?
        } else {
            vec![self.collection.get_collection(&client, "").await?]
        };

        let mut results = Vec::new();

        for col in collections {
            let count = col
                .count()
                .await
                .map_err(RuChatError::ChromaHttpClientError)?;

            let (schema, indexing_status) = if self.long {
                (
                    col.schema().clone(),
                    Some(
                        col.get_indexing_status()
                            .await
                            .map_err(RuChatError::ChromaHttpClientError)?,
                    ),
                )
            } else {
                (None, None)
            };

            results.push(CollectionInfo {
                name: col.name().to_string(),
                id: col.id().0.to_string(),
                database: col.database().to_string(),
                tenant: col.tenant().to_string(),
                version: col.version(),
                count,
                metadata: col.metadata().clone(),
                schema,
                indexing_status,
                storage_prefix: col.id().storage_prefix_for_log(),
            });
        }

        self.render(results)
    }

    fn render(&self, data: Vec<CollectionInfo>) -> Result<()> {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&data)
                    .map_err(|e| RuChatError::InternalError(e.to_string()))?
            );
        } else if self.long {
            for item in data {
                println!("--- Collection: {} ---", item.name);
                println!("ID:       {}", item.id);
                println!("Tenant:   {} | DB: {}", item.tenant, item.database);
                println!("Version:  {} | Count: {}", item.version, item.count);
                println!("Prefix:   {}", item.storage_prefix);
                if let Some(meta) = item.metadata {
                    println!("Metadata: {:?}", meta);
                }
                if let Some(status) = item.indexing_status {
                    println!(
                        "Indexing: {:.1}% ({} / {})",
                        status.op_indexing_progress * 100.0,
                        status.num_indexed_ops,
                        status.total_ops
                    );
                }
                if let Some(schema) = item.schema {
                    println!("Schema:   {:?}", schema);
                }
                println!();
            }
        } else {
            // Compact Table View
            println!(
                "{:<20} {:<38} {:<10} {:<10}",
                "NAME", "ID", "COUNT", "VERSION"
            );
            for item in data {
                println!(
                    "{:<20} {:<38} {:<10} {:<10}",
                    item.name, item.id, item.count, item.version
                );
            }
        }
        Ok(())
    }
}
