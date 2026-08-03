use crate::chroma::r#where::{
    filter_get_response, where_needs_client_side_eval, with_metadata_included,
};
use crate::chroma::{
    query::{filter_query_response, CLIENT_FILTER_MAX_FETCH, CLIENT_FILTER_OVERFETCH_FACTOR},
    rerank::{rerank_query_results, RerankWeights},
    ChromaClientConfigArgs, ChromaCollectionConfigArgs, ChromaResponse, IncludeArgs, OutputArgs,
    WhereArgs,
};
use crate::ollama::OllamaArgs;
use crate::{retry_transient, Result, RuChatError};
use chroma::types::SearchPayload;
use chroma::types::{Key, QueryVector, RankExpr};
use chroma::ChromaCollection;
use clap::{Parser, ValueEnum};
use log::warn;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use serde_json::Value;

#[derive(ValueEnum, Clone, Debug, PartialEq)]
enum RetrieveMode {
    Get,
    Query,
    Search,
}

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct RetrieveArgs {
    /// Retrieval mode: get (direct lookup), query (similarity via text), search (advanced/payload-based).
    #[arg(long, value_enum, default_value = "query")] // Default to query for common use
    mode: RetrieveMode,

    // Shared flags
    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,
    #[command(flatten)]
    client: ChromaClientConfigArgs,
    #[command(flatten)]
    output: OutputArgs,
    #[command(flatten)]
    include: IncludeArgs, // Shared for get/query
    #[command(flatten)]
    r#where: WhereArgs, // Shared for get/query/search (as filter)

    // Get-specific (mutually exclusive with query/search args?)
    // Long-only, deliberately: an explicit `-i` here collides with `IncludeArgs`'s own `-i`
    // (`--include`), both flattened into this same command — see `query.rs`'s identical fix.
    //
    // `requires = "mode=get"` used to be here as an attempt at "only meaningful in --mode get",
    // but that's not valid clap syntax (`requires` takes another arg's *name*, not a
    // `field=value` condition) — it made every invocation of this command panic at startup in
    // debug builds (clap's own argument-validity debug_assert), found while smoke-testing an
    // unrelated change. `determine_mode` already does the actual mode inference/validation at
    // runtime, so this was never load-bearing — removed rather than reimplemented via clap's
    // value-conditional `requires_if`, since nothing here actually needs mode-conditional CLI
    // enforcement on top of what `determine_mode` already provides.
    #[arg(long)]
    ids: Option<String>,
    // Long-only, deliberately: collides with `OllamaArgs`'s `ModelArgs::options` (`-o`), also
    // flattened into this command — see `query.rs`'s identical-shape fix above.
    #[arg(long)]
    offset: Option<u32>,

    // Query-specific. `requires = "mode=query"`/`"mode=search"` below were removed from this
    // and the two Search-specific fields for the same reason as `ids` above: not valid clap
    // syntax (every invocation of this command panicked at startup in debug builds), and
    // `determine_mode` already does the real mode inference/validation at runtime regardless.
    #[arg(short = 'q', long)]
    query_text: Option<String>, // Text to embed

    #[arg(short = 'n', long)]
    n_results: Option<u32>,

    #[command(flatten)]
    ollama: OllamaArgs, // For embedding query_text

    /// Restrict to these IDs (usable in query/search too)
    #[arg(long, conflicts_with = "ids")] // separate from get's --ids
    restrict_ids: Option<String>,

    // Search-specific
    #[arg(short = 'p', long)]
    payload: Option<String>, // JSON or file

    #[arg(short = 'v', long, value_delimiter = ',')]
    query_vector: Option<Vec<f32>>, // Simple dense vector

    #[arg(long, default_value = "index-and-wal")]
    read_level: Option<String>, // Consistency

    // Common limit (overrides mode-specific if needed)
    #[arg(short = 'l', long)]
    limit: Option<u32>,
}

impl RetrieveArgs {
    pub(crate) async fn retrieve(&self, cfg: &Value) -> Result<()> {
        let client = self
            .client
            .create_client(cfg)
            .await
            .map_err(RuChatError::AnyhowError)?;
        let collection =
            retry_transient!(async { self.collection.get_collection(&client, "default").await })?;

        let mode = self.determine_mode()?;

        // Optional: warn when inference differs from explicit --mode
        if self.mode != RetrieveMode::Query && mode != self.mode {
            warn!(
                "Inferred mode {:?} differs from explicit --mode {:?}",
                mode, self.mode
            );
        }

        match mode {
            RetrieveMode::Get => self.execute_get(&collection).await,
            RetrieveMode::Query => self.execute_query(&collection, cfg).await,
            RetrieveMode::Search => self.execute_search(&collection).await,
        }
    }

    fn determine_mode(&self) -> Result<RetrieveMode> {
        let has_payload = self.payload.is_some();
        let has_vec = self.query_vector.is_some();
        let has_text = self.query_text.is_some();
        let has_ids = self.ids.is_some();

        let clues = [
            (has_payload || has_vec, RetrieveMode::Search),
            (has_text, RetrieveMode::Query),
            (has_ids, RetrieveMode::Get),
        ];

        let matching_modes: Vec<_> = clues
            .iter()
            .filter(|(cond, _)| *cond)
            .map(|(_, m)| m)
            .collect();

        match matching_modes.as_slice() {
            [] => {
                // No strong clues → default to most common UX
                Ok(RetrieveMode::Query)
            }
            [RetrieveMode::Search] => Ok(RetrieveMode::Search),
            [RetrieveMode::Query] => Ok(RetrieveMode::Query),
            [RetrieveMode::Get] => Ok(RetrieveMode::Get),
            _multiple => Err(RuChatError::InternalError(format!(
                "Conflicting mode clues provided. Use --mode to disambiguate.\nDetected: {:?}",
                _multiple
            ))),
        }
    }

    async fn execute_get(&self, collection: &ChromaCollection) -> Result<()> {
        let ids_vec: Option<Vec<String>> = self.ids.as_ref().map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        });

        let where_cond = self.r#where.parse()?;
        let mut include_list = self.include.parse()?;

        // See `get.rs::GetArgs::get`'s identical fix — same underlying bug, same reasoning.
        let needs_client_filter = where_cond.as_ref().is_some_and(where_needs_client_side_eval);
        let (query_where, fetch_limit, fetch_offset) = if needs_client_filter {
            include_list = Some(with_metadata_included(include_list));
            (None, None, None)
        } else {
            (where_cond.clone(), self.limit, self.offset)
        };

        let mut result = retry_transient!(async {
            collection
                .get(
                    ids_vec.clone(),
                    query_where.clone(),
                    fetch_limit,
                    fetch_offset,
                    include_list.clone(),
                )
                .await
                .map_err(RuChatError::ChromaHttpClientError)
        })?;

        if let Some(w) = where_cond.as_ref().filter(|_| needs_client_filter) {
            filter_get_response(
                &mut result,
                w,
                self.offset.unwrap_or(0) as usize,
                self.limit.map(|l| l as usize),
            );
        }

        let _ = ChromaResponse::Get(&mut result).render(&self.output);
        Ok(())
    }

    async fn execute_query(&self, collection: &ChromaCollection, cfg: &Value) -> Result<()> {
        let query_text = self.query_text.as_ref().ok_or_else(|| {
            RuChatError::InternalError("No --query-text provided in query mode".into())
        })?;

        let (ollama, models) = self.ollama.init("all-minilm:l6-v2", cfg).await?;
        let model = models
            .last()
            .ok_or(RuChatError::ModelNotFound("all-minilm:l6-v2".into()))?;

        if !model.contains("embed") {
            warn!("Model {model} might not be an embeddings model");
        }

        let request =
            GenerateEmbeddingsRequest::new(model.clone(), vec![query_text.as_str()].into());
        let res = ollama.generate_embeddings(request).await?;
        let embeddings = res.embeddings; // assuming Vec<Vec<f32>>

        let where_cond = self.r#where.parse()?;
        let restrict_ids: Option<Vec<String>> = self.restrict_ids.as_ref().map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        });

        let mut include = self.include.parse()?;

        // See `query.rs::Query::query`'s identical fix — same underlying bug, same reasoning.
        let needs_client_filter = where_cond.as_ref().is_some_and(where_needs_client_side_eval);
        let requested_n = self.n_results.or(self.limit).unwrap_or(10);
        let (query_where, fetch_n) = if needs_client_filter {
            include = Some(with_metadata_included(include));
            (
                None,
                Some(
                    (requested_n.saturating_mul(CLIENT_FILTER_OVERFETCH_FACTOR))
                        .min(CLIENT_FILTER_MAX_FETCH),
                ),
            )
        } else {
            (where_cond.clone(), self.n_results.or(self.limit))
        };

        let mut result = retry_transient!(async {
            collection
                .query(
                    embeddings.clone(),
                    fetch_n,
                    query_where.clone(),
                    restrict_ids.clone(),
                    include.clone(),
                )
                .await
                .map_err(RuChatError::ChromaHttpClientError)
        })?;

        if let Some(w) = where_cond.as_ref().filter(|_| needs_client_filter) {
            filter_query_response(&mut result, w, requested_n as usize);
        }

        let query_texts = vec![query_text.clone()];
        rerank_query_results(&query_texts, &mut result, &RerankWeights::default());
        let _ = ChromaResponse::Query(&mut result).render(&self.output);
        Ok(())
    }

    async fn execute_search(&self, collection: &ChromaCollection) -> Result<()> {
        let search_payload = if let Some(ref p) = self.payload {
            super::parse_search_payload_arg(p)?
        } else if let Some(ref v) = self.query_vector {
            SearchPayload::default()
                .rank(RankExpr::Knn {
                    query: QueryVector::Dense(v.clone()),
                    key: Key::Embedding,
                    limit: self.limit.unwrap_or(10),
                    default: None,
                    return_rank: true,
                })
                .limit(self.limit, 0)
        } else {
            return Err(RuChatError::InternalError(
                "search mode requires --payload or --query-vector".into(),
            ));
        };

        let read_level = super::resolve_read_level(self.read_level.as_deref());

        let mut result = retry_transient!(async {
            collection
                .search_with_options(vec![search_payload.clone()], read_level)
                .await
                .map_err(RuChatError::ChromaHttpClientError)
        })?;

        let _ = ChromaResponse::Search(&mut result).render(&self.output);
        Ok(())
    }
}
