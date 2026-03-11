use crate::{RuChatError, Result};
use crate::chroma::{
    ChromaClientConfigArgs, ChromaCollectionConfigArgs, ChromaResponse, IncludeArgs, OutputArgs,
    WhereArgs,
};
use crate::ollama::OllamaArgs;
use clap::Parser;
use log::{warn, info};
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use serde::Deserialize;
use ollama_rs::Ollama;
use chroma::ChromaHttpClient;

#[derive(Parser, Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct Query {
    /// The query string to search for in the database.
    #[arg(short, long)]
    query: String,

    /// The number of results to return.
    #[arg(short, long)]
    n_results: Option<u32>,

    /// Comma separated list of document IDs to restrict the search.
    #[arg(short, long)]
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
    pub(crate) async fn query(&self, client: &ChromaHttpClient, ollama: &Ollama, model: &str) -> Result<String> {
        let collection = self.collection.get_collection(client, "default").await?;

        if model != "all-minilm:l6-v2" && !model.contains("embed") {
            warn!("Model {model} might not be an embeddings model");
        }
        let request =
            GenerateEmbeddingsRequest::new(model.to_string(), vec![self.query.as_str()].into());
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
        serde_json::from_str(&value).map_err(RuChatError::SerdeError)
    }
}

impl QueryArgs {
    pub(crate) async fn query(&self) -> Result<()> {
        let client = self.client.create_client()?;

        let (ollama, models) = self.ollama.init("all-minilm:l6-v2").await?;
        let model = models
            .last()
            .ok_or(RuChatError::ModelNotFound("all-minilm:l6-v2".to_string()))?;
        info!("{}", self.query.query(&client, &ollama, model).await?);
        Ok(())
    }
}
