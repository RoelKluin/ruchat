use crate::chroma::parse_metadata;
use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::RuChatError;
use crate::ollama::OllamaArgs;
use anyhow::Result;
use chroma::types::{
    BooleanOperator, CompositeExpression, DocumentExpression, DocumentOperator, IncludeList,
    MetadataComparison, MetadataExpression, MetadataValue, PrimitiveOperator, Where,
};
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
pub(crate) struct SimilarityArgs {
    /// Query string to search for similar embeddings.
    #[arg(short, long)]
    query: String,

    /// Number of embeddings to return.
    #[arg(short, long, default_value = "1")]
    count: u32,

    /// Number of similar embeddings to return.
    #[arg(short, long, default_value_t = 5)]
    similarity_count: u32,

    /// Chroma database metadata, comma separated key:value pairs.
    #[arg(short, long)]
    metadata: Option<String>,

    #[command(flatten)]
    ollama_args: OllamaArgs,

    #[command(flatten)]
    client: ChromaClientConfigArgs,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,
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
        let client = self.client.create_client().await?;

        // Instantiate a ChromaCollection to perform operations on a collection
        let collection = self.collection.get_collection(&client, "default").await?;

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
        let where_metadata = parse_metadata(&self.metadata)?.map(Value::Object);
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
