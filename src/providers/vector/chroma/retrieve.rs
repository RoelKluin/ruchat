use crate::chroma::{
    ChromaClientConfigArgs, ChromaCollectionConfigArgs, ChromaResponse, IncludeArgs, OutputArgs,
    WhereArgs,
};
use crate::ollama::OllamaArgs;
use crate::{Result, RuChatError};
use chroma::ChromaCollection;
use chroma::types::SearchPayload;
use chroma::types::{Key, QueryVector, RankExpr};
use chroma_types::plan::ReadLevel;
use clap::{Parser, ValueEnum};
use log::warn;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;

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
    #[arg(short = 'i', long, requires = "mode=get")] // Enforce via Clap or custom validation
    ids: Option<String>,
    #[arg(short = 'o', long)]
    offset: Option<u32>,

    // Query-specific
    #[arg(short = 'q', long, requires = "mode=query")]
    query_text: Option<String>, // Text to embed

    #[arg(short = 'n', long)]
    n_results: Option<u32>,

    #[command(flatten)]
    ollama: OllamaArgs, // For embedding query_text

    /// Restrict to these IDs (usable in query/search too)
    #[arg(long, conflicts_with = "ids")] // separate from get's --ids
    restrict_ids: Option<String>,

    // Search-specific
    #[arg(short = 'p', long, requires = "mode=search")]
    payload: Option<String>, // JSON or file

    #[arg(short = 'v', long, value_delimiter = ',', requires = "mode=search")]
    query_vector: Option<Vec<f32>>, // Simple dense vector

    #[arg(long, default_value = "index-and-wal")]
    read_level: Option<String>, // Consistency

    // Common limit (overrides mode-specific if needed)
    #[arg(short = 'l', long)]
    limit: Option<u32>,
}

impl RetrieveArgs {
    pub(crate) async fn retrieve(&self) -> Result<()> {
        let client = self
            .client
            .create_client()
            .map_err(RuChatError::AnyhowError)?;
        let collection = self.collection.get_collection(&client, "default").await?;

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
            RetrieveMode::Query => self.execute_query(&collection).await,
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
        let include_list = self.include.parse()?;

        let mut result = collection
            .get(ids_vec, where_cond, self.limit, self.offset, include_list)
            .await
            .map_err(RuChatError::ChromaHttpClientError)?;

        let _ = ChromaResponse::Get(&mut result).render(&self.output);
        Ok(())
    }

    async fn execute_query(&self, collection: &ChromaCollection) -> Result<()> {
        let query_text = self.query_text.as_ref().ok_or_else(|| {
            RuChatError::InternalError("No --query-text provided in query mode".into())
        })?;

        let (ollama, models) = self.ollama.init("all-minilm:l6-v2").await?;
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

        let include = self.include.parse()?;

        let mut result = collection
            .query(
                embeddings,
                self.n_results.or(self.limit),
                where_cond,
                restrict_ids,
                include,
            )
            .await
            .map_err(RuChatError::ChromaHttpClientError)?;

        let _ = ChromaResponse::Query(&mut result).render(&self.output);
        Ok(())
    }

    async fn execute_search(&self, collection: &ChromaCollection) -> Result<()> {
        let search_payload = if let Some(ref p) = self.payload {
            self.parse_payload(p)?
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

        let read_level = self
            .read_level
            .as_ref()
            .map(|s| match s.to_lowercase().as_str() {
                "index-only" | "indexonly" => ReadLevel::IndexOnly,
                _ => ReadLevel::IndexAndWal,
            })
            .unwrap_or(ReadLevel::IndexAndWal);

        let mut result = collection
            .search_with_options(vec![search_payload], read_level)
            .await
            .map_err(RuChatError::ChromaHttpClientError)?;

        let _ = ChromaResponse::Search(&mut result).render(&self.output);
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
