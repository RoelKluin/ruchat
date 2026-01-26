use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::embed::EmbedArgs;
use crate::error::RuChatError;
use crate::io::Io;
use crate::ollama::OllamaArgs;
use anyhow::Result;
use chromadb::collection::GetOptions;
use clap::Parser;
use serde_json::json;
use tokio_stream::StreamExt;

/// Command-line arguments for querying a Chroma database.
///
/// This struct defines the arguments required to perform a query
/// in a Chroma database, including model details, query parameters,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub struct QueryArgs {
    /// The query string to search for in the database.
    #[arg(short, long)]
    query: String,

    /// The prompt to use for generating a response.
    #[arg(short, long)]
    prompt: String,

    /// The number of results to return.
    #[arg(short, long, default_value_t = 1)]
    count: usize,

    /// Chroma database metadata, comma separated key:value pairs.
    #[arg(short, long)]
    metadata: Option<String>,

    #[command(flatten)]
    collection_config: ChromaCollectionConfigArgs,

    #[command(flatten)]
    client_config: ChromaClientConfigArgs,

    #[command(flatten)]
    ollama_args: OllamaArgs,
}

impl QueryArgs {
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

        let client = self.client_config.create_client().await?;
        let collection = self
            .collection_config
            .get_or_create_collection(&client)
            .await?;
        let metadata = self.metadata.as_deref().map(|md| md.into());

        // Create a filter object to filter by document content.
        let where_document = json!({
            "$contains": self.query.as_str()
        });
        eprintln!("Where document filter: {:?}", where_document);

        // Get embeddings from a collection with filters and limit set to 1.
        // An empty IDs vec will return all embeddings.
        let get_query = GetOptions {
            ids: vec![],
            where_metadata: metadata,
            limit: Some(self.count),
            offset: None,
            where_document: Some(where_document),
            include: Some(vec!["documents".into(), "embeddings".into()]),
        };

        let get_result = collection.get(get_query).await?;
        let res: Vec<_> = get_result
            .embeddings
            .map(|embeddings| embeddings.into_iter().flatten().collect())
            .unwrap_or_default();
        eprintln!("Get result: {:?}", res);
        let prompt = format!(
            "Using this data: {:?}, respond to this prompt: {}",
            res, self.prompt
        );
        eprintln!("Final prompt: {}", prompt);

        let mut cio = Io::new();
        let (ollama, models) = self.ollama_args.init("").await?;
        let model = models.first().unwrap().to_string();
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
