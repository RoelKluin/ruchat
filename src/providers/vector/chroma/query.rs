use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs, parse_where};
use crate::ollama::OllamaArgs;
use crate::RuChatError;
use anyhow::Result;
use chroma::types::{
     IncludeList,
};
use clap::Parser;
use log::{info, warn};
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;

/// Command-line arguments for querying a Chroma database.
///
/// This struct defines the arguments required to perform a query
/// in a Chroma database, including model details, query parameters,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct QueryArgs {
    /// The query string to search for in the database.
    #[arg(short, long)]
    query: String,

    /// The prompt to use for generating a response.
    #[arg(short, long)]
    prompt: String,

    /// The number of results to return.
    #[arg(short, long)]
    n_results: Option<u32>,

    /// Comma separated list of document IDs to restrict the search.
    #[arg(short, long)]
    ids: Option<String>,

    /// JSON string for IncludeList.
    #[arg(short, long)]
    include: Option<String>,

    /// Chroma database metadata, comma separated key:value pairs.
    #[arg(short, long)]
    metadata: Option<String>,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    client: ChromaClientConfigArgs,

    #[command(flatten)]
    ollama: OllamaArgs,
}

impl QueryArgs {
    pub(crate) async fn query(&self) -> Result<(), RuChatError> {
        let client = self.client.create_client()?;
        let collection = self.collection.get_collection(&client, "default").await?;

        let (ollama, models) = self.ollama.init("all-minilm:l6-v2").await?;
        let model = models.last().unwrap().to_string();
        if model != "all-minilm:l6-v2" && !model.contains("embed") {
            warn!("Model {model} might not be an embeddings model");
        }
        let request = GenerateEmbeddingsRequest::new(model, vec![self.query.as_str()].into());
        let res = ollama.generate_embeddings(request).await?;

        let query_embeddings = res.embeddings;
        
        let where_metadata = self.metadata.as_ref()
            .map(|md| parse_where(md))
            .transpose()?;

        let ids = self.ids.as_ref()
            .map(|s| s.split(',').map(|id| id.trim().to_string()).collect());

        let include = self.include.as_ref()
            .map(|inc| serde_json::from_str::<IncludeList>(inc))
            .transpose()?;

        let query_result = collection.query(
            query_embeddings,
            self.n_results,
            where_metadata,
            ids,
            include,
        ).await?;

        info!("Query results: {}", serde_json::to_string_pretty(&query_result)?); 
        Ok(())
    }
}

