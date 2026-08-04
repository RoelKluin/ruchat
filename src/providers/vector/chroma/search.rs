use crate::chroma::{
    ChromaClientConfigArgs, ChromaCollectionConfigArgs, ChromaResponse, OutputArgs,
};
use crate::{Result, RuChatError, retry_transient};
use chroma::types::SearchPayload;
use chroma::types::{Key, QueryVector, RankExpr};
use clap::Parser;
use serde_json::Value;

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

    #[command(flatten)]
    output: OutputArgs,
}

impl SearchArgs {
    pub(crate) async fn search(&self, cfg: &Value) -> Result<()> {
        let client = self
            .client
            .create_client(cfg)
            .await
            .map_err(RuChatError::AnyhowError)?;
        let collection = self.collection.get_collection(&client, "default").await?;

        // 1. Resolve the SearchPayload (Basic KNN or JSON-based)
        let search_payload = if let Some(ref p) = self.payload {
            super::parse_search_payload_arg(p)?
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
            return Err(RuChatError::InternalError(
                "Provide --payload or --query".into(),
            ));
        };

        // 2. Map the CLI string to the ReadLevel enum
        let mut search_result = if let Some(read_level) = self.read_level.as_deref() {
            let read_level = super::resolve_read_level(Some(read_level));

            // 3. Execute with options
            retry_transient!(async {
                collection
                    .search_with_options(vec![search_payload.clone()], read_level)
                    .await
                    .map_err(RuChatError::from)
            })?
        } else {
            retry_transient!(async {
                collection
                    .search(vec![search_payload.clone()])
                    .await
                    .map_err(RuChatError::from)
            })?
        };
        ChromaResponse::Search(&mut search_result).render(&self.output)
    }
}
