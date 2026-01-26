use crate::chroma::get_metadata;
use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::error::RuChatError;
use crate::ollama::OllamaArgs;
use anyhow::Result;
use chromadb::collection::{ChromaCollection, GetOptions, GetResult, QueryOptions, QueryResult};
use clap::Parser;
use log::warn;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use serde_json::Value;

/// Chroma database similarity search command line arguments.
///
/// This struct defines the arguments required to perform a similarity
/// search in a Chroma database, including query parameters and database
/// connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub struct SimilarityArgs {
    /// Query string to search for similar embeddings.
    #[arg(short, long)]
    pub(crate) query: String,

    /// Number of embeddings to return.
    #[arg(short, long, default_value = "1")]
    pub(crate) count: usize,

    /// Number of similar embeddings to return.
    #[arg(short, long, default_value = "5")]
    pub(crate) similarity_count: usize,

    /// Chroma database metadata, comma separated key:value pairs.
    #[arg(short, long)]
    pub(crate) metadata: Option<String>,

    // FIXME: this is clashing with AskArgs ollama_args
    #[command(flatten)]
    ollama_args: OllamaArgs,

    #[command(flatten)]
    pub client_config: ChromaClientConfigArgs,

    #[command(flatten)]
    pub collection_config: ChromaCollectionConfigArgs,
}

impl SimilarityArgs {
    /// Subcommand to find similar embeddings in a Chroma database.
    ///
    /// This function connects to a Chroma database using the provided
    /// arguments, performs a similarity search, and returns the results.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn similarity_search(&self) -> Result<(), RuChatError> {
        let client = self.client_config.create_client().await?;

        // Instantiate a ChromaCollection to perform operations on a collection
        let collection = self
            .collection_config
            .get_or_create_collection(&client)
            .await?;

        let (ollama, models) = self.ollama_args.init("all-minilm:l6-v2").await?;
        let model = models.first().unwrap().as_str();
        if model != "all-minilm:l6-v2" && !model.contains("embed") {
            warn!("Model {model} might not be an embeddings model");
        }
        let request =
            GenerateEmbeddingsRequest::new(model.to_string(), vec![self.query.as_str()].into());
        let res = ollama.generate_embeddings(request).await?;

        let n_results = Some(self.similarity_count);
        let query_embeddings = Some(res.embeddings);
        let query_texts = None; //Some(vec![self.query.as_str()]);
        let where_metadata = get_metadata(&self.metadata)?.map(|m| Value::Object(m));
        let where_document = None;
        let include = Some(vec!["distances"]);
        let query_options = QueryOptions {
            query_embeddings,
            query_texts,
            n_results,
            where_metadata,
            where_document,
            include,
        };
        let embedding_function = None;

        let query_result = collection.query(query_options, embedding_function).await?;
        println!("Query result: {:?}", query_result);
        Ok(())
    }
}
