use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::{RuChatError, Result};
use clap::Parser;
use log::info;
use chroma::types::SearchPayload;
use chroma::types::{RankExpr, QueryVector, Key};
use chroma_types::plan::ReadLevel;

/// Command-line arguments for searching a Chroma collection.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct SearchArgs {
    /// A JSON string or path to a JSON file representing the SearchPayload.
    #[arg(short, long)]
    payload: Option<String>,

    /// Simple query vector (comma-separated floats) for a basic KNN search.
    #[arg(short, long, value_delimiter = ',')]
    query: Option<Vec<f32>>,

    /// The number of results to return.
    #[arg(short, long)]
    limit: Option<u32>,

    /// Consistency level: 'index-and-wal' (full consistency) or 'index-only' (higher throughput).
    /// Defaults to 'index-and-wal'.
    #[arg(long, default_value = "index-and-wal")]
    read_level: Option<String>,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    client: ChromaClientConfigArgs,
}

impl SearchArgs {
    pub(crate) async fn search(&self) -> Result<()> {
        let client = self.client.create_client().map_err(RuChatError::ChromaError)?;
        let collection = self.collection.get_collection(&client, "default").await?;

        // 1. Resolve the SearchPayload (Basic KNN or JSON-based)
        let search_payload = if let Some(ref p) = self.payload {
            self.parse_payload(p)?
        } else if let Some(ref q) = self.query {
            SearchPayload::default()
                .rank(RankExpr::Knn {
                    query: QueryVector::Dense(q.clone()),
                    key: Key::Embedding,
                    limit: self.limit.unwrap_or(10),
                    default: None,
                    return_rank: true,
                })
                .limit(self.limit, 0)
        } else {
            return Err(RuChatError::InternalError("Provide --payload or --query".into()));
        };

        // 2. Map the CLI string to the ReadLevel enum
        if let Some(read_level) = self.read_level.as_ref() {
            let read_level = match read_level.to_lowercase().as_str() {
                "index-only" | "indexonly" => ReadLevel::IndexOnly,
                _ => ReadLevel::IndexAndWal, // Default to full consistency
            };

            // 3. Execute with options
            let res = collection
                .search_with_options(vec![search_payload], read_level)
                .await
                .map_err(RuChatError::ChromaHttpClientError)?;

            // 4. Output results
            info!("Search #(ReadLevel: {:?}) results: {:?}", read_level, res);
        } else {
            let res = collection
                .search(vec![search_payload])
                .await
                .map_err(RuChatError::ChromaHttpClientError)?;
            info!("Search results: {:?}", res);
        }

        Ok(())
    }

    fn parse_payload(&self, input: &str) -> Result<SearchPayload> {
        let json_str = if std::path::Path::new(input).exists() {
            std::fs::read_to_string(input).map_err(|e| RuChatError::InternalError(e.to_string()))?
        } else {
            input.to_string()
        };
        serde_json::from_str(&json_str)
            .map_err(|e| RuChatError::InternalError(format!("Payload error: {}", e)))
    }
}
