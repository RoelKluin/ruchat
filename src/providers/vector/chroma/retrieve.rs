use crate::chroma::{
    ChromaClientConfigArgs, ChromaCollectionConfigArgs, ChromaResponse, IncludeArgs, OutputArgs,
    WhereArgs,
};
use crate::ollama::OllamaArgs;
use crate::{Result, RuChatError};
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
            .map_err(RuChatError::ChromaError)?;
        let collection = self.collection.get_collection(&client, "default").await?;

        // Optional: Infer mode if not explicit, e.g., if ids provided -> get; if query_text -> query; etc.
        // But for clarity, rely on --mode and validate args.
        self.validate_args()?; // Custom fn to check compat (e.g., no payload in get mode)

        match self.mode {
            RetrieveMode::Get => {
                let ids_vec: Option<Vec<String>> = self
                    .ids
                    .as_ref()
                    .map(|s| s.split(',').map(str::trim).map(str::to_string).collect());
                let where_cond = self.r#where.parse()?;
                let include_list = self.include.parse()?;
                let mut result = collection
                    .get(ids_vec, where_cond, self.limit, self.offset, include_list)
                    .await?;
                ChromaResponse::Get(&mut result).render(&self.output)
            }
            RetrieveMode::Query => {
                if self.query_text.is_none() {
                    return Err(RuChatError::InternalError(
                        "Provide --query-text for query mode".into(),
                    ));
                }
                let (ollama, models) = self.ollama.init("all-minilm:l6-v2").await?;
                let model = models
                    .last()
                    .ok_or_else(|| RuChatError::ModelNotFound("all-minilm:l6-v2".to_string()))?
                    .to_string();
                if model != "all-minilm:l6-v2" && !model.contains("embed") {
                    warn!("Model {model} might not be an embeddings model");
                }
                let request = GenerateEmbeddingsRequest::new(
                    model.to_string(),
                    vec![self.query_text.as_ref().unwrap().as_str()].into(),
                );
                let res = ollama.generate_embeddings(request).await?;
                let query_embeddings = res.embeddings;

                let where_cond = self.r#where.parse()?;
                let ids_vec: Option<Vec<String>> = self
                    .ids
                    .as_ref()
                    .map(|s| s.split(',').map(str::trim).map(str::to_string).collect());
                let include_list = self.include.parse()?;
                let mut result = collection
                    .query(
                        query_embeddings,
                        self.n_results.or(self.limit),
                        where_cond,
                        ids_vec,
                        include_list,
                    )
                    .await?;
                ChromaResponse::Query(&mut result).render(&self.output)
            }
            RetrieveMode::Search => {
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
                        "Provide --payload or --query-vector for search mode".into(),
                    ));
                };
                // Apply where if needed (assuming SearchPayload can include it)
                let mut result = if let Some(read_level) = self.read_level.as_ref() {
                    let read_level = match read_level.to_lowercase().as_str() {
                        "index-only" | "indexonly" => ReadLevel::IndexOnly,
                        _ => ReadLevel::IndexAndWal, // Default to full consistency
                    };

                    // 3. Execute with options
                    collection
                        .search_with_options(vec![search_payload], read_level)
                        .await
                        .map_err(RuChatError::ChromaHttpClientError)?
                } else {
                    collection
                        .search(vec![search_payload])
                        .await
                        .map_err(RuChatError::ChromaHttpClientError)?
                };
                ChromaResponse::Search(&mut result).render(&self.output)
            }
        }
    }

    fn validate_args(&self) -> Result<()> {
        match self.mode {
            RetrieveMode::Get => {
                if self.query_text.is_some() {
                    return Err(RuChatError::InternalError(
                        "Cannot use --query-text in get mode".into(),
                    ));
                }
                if self.payload.is_some() || self.query_vector.is_some() {
                    return Err(RuChatError::InternalError(
                        "Cannot use --payload or --query-vector in get mode".into(),
                    ));
                }
            }
            RetrieveMode::Query => {
                if self.ids.is_some() {
                    return Err(RuChatError::InternalError(
                        "Cannot use --ids in query mode".into(),
                    ));
                }
                if self.payload.is_some() || self.query_vector.is_some() {
                    return Err(RuChatError::InternalError(
                        "Cannot use --payload or --query-vector in query mode".into(),
                    ));
                }
                if self.query_text.is_none() {
                    return Err(RuChatError::InternalError(
                        "Provide --query-text for query mode".into(),
                    ));
                }
            }
            RetrieveMode::Search => {
                if self.ids.is_some() {
                    return Err(RuChatError::InternalError(
                        "Cannot use --ids in search mode".into(),
                    ));
                }
                if self.query_text.is_some() {
                    return Err(RuChatError::InternalError("Cannot use --query-text in search mode; use --payload or --query-vector instead".into()));
                }
                if self.query_vector.is_some() && self.payload.is_some() {
                    return Err(RuChatError::InternalError(
                        "Provide either --payload or --query-vector for search mode, not both"
                            .into(),
                    ));
                }
                if self.query_vector.is_none() && self.payload.is_none() {
                    return Err(RuChatError::InternalError(
                        "Provide --payload or --query-vector for search mode".into(),
                    ));
                }
            }
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
