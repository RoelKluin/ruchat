use crate::agent::llm_client::{LlmClient, VectorStore};
use crate::chroma::r#where::{
    metadata_matches, select_indices, where_needs_client_side_eval, with_metadata_included,
};
use crate::chroma::{
    rerank::{rerank_query_results, RerankWeights},
    ChromaClientConfigArgs, ChromaCollectionConfigArgs, ChromaResponse, IncludeArgs, OutputArgs,
    WhereArgs,
};
use crate::ollama::OllamaArgs;
use crate::{Result, RuChatError};
use chroma::types::QueryResponse;
use clap::Parser;
use log::warn;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use serde::Deserialize;
use serde_json::Value;

/// When a query's `--where` needs client-side evaluation (see `where_needs_client_side_eval`),
/// how many candidates to over-fetch per requested result: Chroma's own similarity ranking has
/// no idea which of the nearest neighbors will also pass our client-side filter, so we ask for
/// more than requested and truncate after filtering. A heuristic, not a guarantee — a filter
/// that only a small fraction of the collection satisfies can still under-return versus what a
/// real server-side filter would have found; there's no way around that without Chroma
/// supporting substring matching on scalar metadata natively. `pub(crate)` so `retrieve.rs`'s
/// `execute_query` (the same similarity-search shape as `Query::query` below) uses the same
/// heuristic instead of picking its own, different numbers.
pub(crate) const CLIENT_FILTER_OVERFETCH_FACTOR: u32 = 5;
pub(crate) const CLIENT_FILTER_MAX_FETCH: u32 = 200;

#[derive(Parser, Debug, Clone, PartialEq, Deserialize, Default)]
pub(crate) struct Query {
    /// The query strings to search for in the database.
    #[arg(short, long, value_delimiter = ',', help_heading = "Query Content")]
    query: Vec<String>,

    /// The number of results to return.
    #[arg(
        short,
        long,
        help = "Number of results to return (default: 10)",
        long_help = "Number of nearest neighbors to return.\n\
                     Higher values = slower but more complete answers.\n\
                     Typical range: 3–50",
        help_heading = "Query Content"
    )]
    n_results: Option<u32>,

    /// Comma separated list of document IDs to restrict the search.
    #[arg(short, long, value_delimiter = ',', help_heading = "Filtering")]
    ids: Option<String>,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    include: IncludeArgs,

    #[command(flatten)]
    r#where: WhereArgs,

    #[command(flatten)]
    output: OutputArgs,
}

impl Query {
    pub(crate) async fn query(
        &self,
        client: &dyn VectorStore,
        ollama: &dyn LlmClient,
        model: &str,
    ) -> Result<String> {
        if model != "all-minilm:l6-v2" && !model.contains("embed") {
            warn!("Model {model} might not be an embeddings model");
        }
        let request = GenerateEmbeddingsRequest::new(model.to_string(), self.query.clone().into());
        let res = ollama.generate_embeddings(request).await?;
        let query_embeddings = res.embeddings;

        let r#where = self.r#where.parse()?;
        let ids = super::parse_ids(&self.ids);
        let mut include = self.include.parse()?;

        // Collection-name resolution moved here from
        // `ChromaCollectionConfigArgs::get_collection` (which needed a
        // concrete `ChromaHttpClient`) — the retry-wrapped get+query round
        // trip itself now lives in `VectorStore::query_collection`.
        let collection_name = if self.collection.name().is_empty() {
            "default"
        } else {
            self.collection.name()
        };

        // See `where_needs_client_side_eval`'s doc comment: Chroma's metadata filter has no
        // scalar-substring operator, so a CONTAINS anywhere in `where` must be evaluated
        // ourselves rather than sent to Chroma (which would just silently filter everything
        // out against a plain string field). Don't send `where` at all in that case, over-fetch
        // candidates from the unfiltered similarity search instead, and make sure metadata is
        // actually part of the response — our filter can't evaluate what it doesn't have.
        let needs_client_filter = r#where.as_ref().is_some_and(where_needs_client_side_eval);
        let requested_n = self.n_results.unwrap_or(10);
        let (query_where, fetch_n) = if needs_client_filter {
            include = Some(with_metadata_included(include));
            (
                None,
                Some((requested_n.saturating_mul(CLIENT_FILTER_OVERFETCH_FACTOR)).min(CLIENT_FILTER_MAX_FETCH)),
            )
        } else {
            (r#where.clone(), self.n_results)
        };

        let mut query_result = client
            .query_collection(
                collection_name,
                query_embeddings,
                fetch_n,
                query_where,
                ids,
                include,
            )
            .await?;

        if let Some(w) = r#where.as_ref().filter(|_| needs_client_filter) {
            filter_query_response(&mut query_result, w, requested_n as usize);
        }

        rerank_query_results(&self.query, &mut query_result, &RerankWeights::default());
        ChromaResponse::Query(&mut query_result).as_string(&self.output)
    }
    pub(crate) fn update_from_json(&mut self, v: Value) -> Result<()> {
        if let Some(query) = v.get("query").and_then(|q| q.as_array()) {
            self.query = query
                .iter()
                .filter_map(|q| q.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(n_results) = v.get("n_results").and_then(|n| n.as_u64()) {
            self.n_results = Some(n_results as u32);
        }
        if let Some(ids) = v.get("ids").and_then(|i| i.as_str()) {
            self.ids = Some(ids.to_string());
        }
        if v.get("collection").is_some() {
            self.collection.update_from_json(&v)?;
        }
        if v.get("include").is_some() {
            self.include.update_from_json(&v)?;
        }
        if v.get("where").is_some() {
            self.r#where.update_from_json(&v)?;
        }
        if v.get("output").is_some() {
            self.output.update_from_json(&v)?;
        }
        Ok(())
    }
}

/// Filters an over-fetched `QueryResponse` down to only the results whose metadata satisfies
/// `w` (evaluated via `metadata_matches`, since Chroma couldn't apply this filter itself — see
/// `where_needs_client_side_eval`), then truncates to `keep_n` per query. `QueryResponse` batches
/// multiple query texts in one call (one outer `Vec` entry each), so every parallel field is
/// filtered by the same kept-index set per batch entry to stay aligned. `pub(crate)` so
/// `retrieve.rs`'s `execute_query` (structurally the same similarity-search shape as `Query::
/// query` below, just via a different embedding call) can reuse it instead of duplicating.
pub(crate) fn filter_query_response(r: &mut QueryResponse, w: &chroma::types::Where, keep_n: usize) {
    for i in 0..r.ids.len() {
        let keep_indices: Vec<usize> = (0..r.ids[i].len())
            .filter(|&j| {
                r.metadatas
                    .as_ref()
                    .and_then(|m| m.get(i))
                    .and_then(|row| row.get(j))
                    .and_then(|opt| opt.as_ref())
                    .is_some_and(|meta| metadata_matches(w, meta))
            })
            .take(keep_n)
            .collect();

        r.ids[i] = select_indices(&r.ids[i], &keep_indices);
        if let Some(outer) = r.embeddings.as_mut() {
            outer[i] = select_indices(&outer[i], &keep_indices);
        }
        if let Some(outer) = r.documents.as_mut() {
            outer[i] = select_indices(&outer[i], &keep_indices);
        }
        if let Some(outer) = r.uris.as_mut() {
            outer[i] = select_indices(&outer[i], &keep_indices);
        }
        if let Some(outer) = r.metadatas.as_mut() {
            outer[i] = select_indices(&outer[i], &keep_indices);
        }
        if let Some(outer) = r.distances.as_mut() {
            outer[i] = select_indices(&outer[i], &keep_indices);
        }
    }
}

/// Command-line arguments for querying a Chroma database.
///
/// This struct defines the arguments required to perform a query
/// in a Chroma database, including model details, query parameters,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct QueryArgs {
    #[command(flatten)]
    client: ChromaClientConfigArgs,

    #[command(flatten)]
    ollama: OllamaArgs,

    #[command(flatten)]
    query: Query,
}

impl TryFrom<String> for QueryArgs {
    type Error = RuChatError;

    fn try_from(value: String) -> Result<Self> {
        serde_json::from_str(&value)
            .map_err(|e| {
                tracing::error!(error = ?e, "failed to deserialize JSON into QueryArgs");
                e
            })
            .map_err(RuChatError::SerdeError)
    }
}

impl QueryArgs {
    pub(crate) async fn query(&self, cfg: &Value) -> Result<()> {
        let client = self.client.create_client(cfg).await?;

        let (ollama, models) = self.ollama.init("all-minilm:l6-v2", cfg).await?;
        let model = models
            .last()
            .ok_or(RuChatError::ModelNotFound("all-minilm:l6-v2".to_string()))?;
        let res = self.query.query(&client, &ollama, model).await?;
        println!("{res}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm_client::fake_vector_store::FakeVectorStore;
    use crate::agent::llm_client::FakeLlmClient;
    use chroma::types::Metadata;
    use chroma::types::MetadataValue;

    fn metadata_with_file(file: &str) -> Metadata {
        let mut m = Metadata::new();
        m.insert("file".to_string(), MetadataValue::Str(file.to_string()));
        m
    }

    fn fake_response(files: &[&str]) -> QueryResponse {
        QueryResponse {
            ids: vec![(0..files.len()).map(|i| format!("id{i}")).collect()],
            embeddings: None,
            documents: None,
            uris: None,
            metadatas: Some(vec![
                files.iter().map(|f| Some(metadata_with_file(f))).collect(),
            ]),
            distances: None,
            include: vec![],
        }
    }

    // Regression: a real run found `file CONTAINS 'cli'` (one of db_config.json's own
    // documented example queries) always returned zero results against the repo_src
    // collection's scalar `file` metadata field. `FakeVectorStore` always returns its whole
    // fixed response regardless of what `where`/`n_results` it's called with (mirroring
    // Chroma's own would-be-broken behavior would require a smarter fake; the point of this
    // test is what `Query::query` does with the response afterward), so the fix must be
    // visible in the rendered output: only rows whose `file` value actually contains "cli"
    // should survive.
    #[tokio::test]
    async fn query_filters_contains_on_a_scalar_field_client_side() {
        let response = fake_response(&["src/cli/args.rs", "src/tui/io.rs", "src/cli/prompt.rs"]);
        let store = FakeVectorStore { response };
        let ollama = FakeLlmClient::new(vec![]);

        let mut q = Query {
            query: vec!["anything".to_string()],
            ..Query::default()
        };
        q.r#where
            .update_from_json(&serde_json::json!({ "where": "file CONTAINS 'cli'" }))
            .unwrap();

        let rendered = q.query(&store, &ollama, "all-minilm:l6-v2").await.unwrap();

        assert!(rendered.contains("src/cli/args.rs"));
        assert!(rendered.contains("src/cli/prompt.rs"));
        assert!(!rendered.contains("src/tui/io.rs"));
    }
}
