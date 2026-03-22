use crate::chroma::{
    ChromaClientConfigArgs, ChromaCollectionConfigArgs, ChromaResponse, IncludeArgs, OutputArgs,
    WhereArgs,
};
use crate::ollama::OllamaArgs;
use crate::{Result, RuChatError};
use chroma::ChromaHttpClient;
use clap::Parser;
use log::warn;
use ollama_rs::Ollama;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use serde::Deserialize;
use serde_json::Value;

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
        client: &ChromaHttpClient,
        ollama: &Ollama,
        model: &str,
    ) -> Result<String> {
        let collection = self.collection.get_collection(client, "default").await?;

        if model != "all-minilm:l6-v2" && !model.contains("embed") {
            warn!("Model {model} might not be an embeddings model");
        }
        let request = GenerateEmbeddingsRequest::new(model.to_string(), self.query.clone().into());
        let res = ollama.generate_embeddings(request).await?;

        let query_embeddings = res.embeddings;

        let r#where = self.r#where.parse()?;

        let ids = self
            .ids
            .as_ref()
            .map(|s| s.split(',').map(|id| id.trim().to_string()).collect());

        let include = self.include.parse()?;

        let mut query_result = collection
            .query(query_embeddings, self.n_results, r#where, ids, include)
            .await?;
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
    pub(crate) async fn query(&self) -> Result<()> {
        let client = self.client.create_client()?;

        let (ollama, models) = self.ollama.init("all-minilm:l6-v2").await?;
        let model = models
            .last()
            .ok_or(RuChatError::ModelNotFound("all-minilm:l6-v2".to_string()))?;
        let res = self.query.query(&client, &ollama, model).await?;
        eprintln!("got: {}", res);
        Ok(())
    }
}
