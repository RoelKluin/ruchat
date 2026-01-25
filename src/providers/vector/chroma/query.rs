use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::RuChatError;
use crate::io::Io;
use crate::ollama::OllamaArgs;
use anyhow::Result;
use chroma::types::{
    BooleanOperator, CompositeExpression, DocumentExpression, DocumentOperator, IncludeList,
    MetadataComparison, MetadataExpression, MetadataValue, PrimitiveOperator, Where,
};
use clap::Parser;
use serde_json::json;
use tokio_stream::StreamExt;
use crate::embed::EmbedArgs;

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
    #[arg(short, long, default_value_t = 1)]
    count: u32,

    /// Chroma database metadata, comma separated key:value pairs.
    #[arg(short, long)]
    metadata: Option<String>,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    client: ChromaClientConfigArgs,

    #[command(flatten)]
    ollama: OllamaArgs,

    #[command(flatten)]
    embed_args: EmbedArgs,
}

impl QueryArgs {
    pub(crate) async fn query_chroma(&self) -> Result<Vec<Vec<f32>>, RuChatError> {
        let client = self.client_config.create_client()?;
        // Perform the query
        let collection = self
            .collection_config
            .get_or_create_collection(&client)
            .await?;

        let query_embeddings: Vec<Vec<f32>> = vec![];
        let n_results: Option<u32> = Some(self.count);
        let where_metadata: Option<Where> = self.get_where_metadata();
        let ids: Option<Vec<String>> = None;
        let include: Option<IncludeList> = Some(IncludeList::default_get());

        let result = collection
            .query(query_embeddings, n_results, where_metadata, ids, include)
            .await?;

        match result.embeddings {
            Some(embeddings) => Ok(embeddings
                .into_iter()
                .map(|e| e.into_iter().flatten().flatten().collect())
                .collect()),
            None => Ok(vec![]),
        }
    }

    /// Performs a query on a Chroma database and generates a response.
    ///
    /// This function connects to a Chroma database using the provided
    /// arguments, performs a query, and generates a response using the
    /// specified model.
    ///
    /// # Parameters
    ///
    /// - `ollama`: The Ollama client for generating responses.
    /// - `args`: The command-line arguments for the query.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn query(&self) -> Result<(), RuChatError> {
        // Get embeddings from a collection with filters and limit set to 1.
        // An empty IDs vec will return all embeddings.
        println!("Creating Chroma client...");

        let client = self.client.create_client()?;
        let collection = self.collection.get_collection(&client, "default").await?;
        let metadata = self.metadata.as_deref().map(|md| md.into());

        let ids: Option<Vec<String>> = None;
        let where_metadata = self.get_where_metadata();
        let limit = Some(self.count);
        let offset = None;
        let include = Some(IncludeList::default_get());
        let get_result = collection
            .get(ids, where_metadata, limit, offset, include)
            .await?;

        let res: Vec<_> = get_result
            .embeddings
            .map(|embeddings| embeddings.into_iter().flatten().collect())
            .unwrap_or_default();
        eprintln!("Get result: {:?}", res);
        let prompt = format!(
            "Using this data: {:?}, respond to this prompt: {}",
            res, self.prompt
        );

        let mut cio = Io::new();
        let (ollama, model) = self.ollama_args.init("").await?;
        let request = self
            .ollama_args
            .build_generation_request(model, prompt)
            .await?;
        let mut stream = ollama.generate_stream(request).await?;
        while let Some(res) = stream.next().await {
            let responses = res?;
            for resp in responses {
                cio.write_line(&resp.response).await?;
            }
        }
        Ok(())
    }
}
