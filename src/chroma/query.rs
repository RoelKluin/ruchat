use crate::chroma::create_client;
use crate::error::RuChatError;
use crate::io::Io;
use crate::ollama::model::get_name;
use crate::options::get_options;
use anyhow::Result;
use chromadb::collection::{ChromaCollection, GetOptions, GetResult};
use clap::Parser;
use ollama_rs::Ollama;
use ollama_rs::generation::completion::request::GenerationRequest;
use serde_json::json;
use tokio_stream::StreamExt;

/// Command-line arguments for querying a Chroma database.
///
/// This struct defines the arguments required to perform a query
/// in a Chroma database, including model details, query parameters,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub struct QueryArgs {
    /// The model to use for the query.
    #[clap(short, long, default_value = "qwen2.5-coder:14b")]
    pub(crate) model: String,

    /// Optional configuration file for model options, or a string
    /// representing the options in JSON format.
    #[clap(short, long)]
    pub(crate) options: Option<String>,

    /// The query string to search for in the database.
    #[clap(short, long)]
    pub(crate) query: String,

    /// The prompt to use for generating a response.
    #[clap(short, long)]
    pub(crate) prompt: String,

    /// The number of results to return.
    #[clap(short, long, default_value = "1")]
    pub(crate) count: usize,

    /// Chroma database collection name.
    #[clap(short, long, default_value = "default")]
    pub(crate) collection: String,

    /// Chroma database metadata, comma separated key:value pairs.
    #[clap(short, long)]
    pub(crate) metadata: Option<String>,

    /// Chroma database server address and port.
    #[clap(short = 'C', long, default_value = "http://localhost:8000")]
    pub(crate) chroma_server: String,

    /// Chroma database name.
    #[clap(short = 'd', long, default_value = "default")]
    pub(crate) chroma_database: String,

    /// Chroma token for authentication.
    #[clap(short = 't', long)]
    pub(crate) chroma_token: Option<String>,
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
pub(crate) async fn query(ollama: Ollama, args: &QueryArgs) -> Result<(), RuChatError> {
    let client = create_client(
        args.chroma_token.as_deref(),
        &args.chroma_server,
        &args.chroma_database,
    )
    .await?;
    let collection: ChromaCollection = client
        .get_or_create_collection(&args.collection, None)
        .await?;

    let metadata = args.metadata.as_deref().map(|md| md.into());

    // Create a filter object to filter by document content.
    let where_document = json!({
        "$contains": args.query.as_str()
    });

    // Get embeddings from a collection with filters and limit set to 1.
    // An empty IDs vec will return all embeddings.
    let get_query = GetOptions {
        ids: vec![],
        where_metadata: metadata,
        limit: Some(args.count),
        offset: None,
        where_document: Some(where_document),
        include: Some(vec!["documents".into(), "embeddings".into()]),
    };
    let get_result: GetResult = collection.get(get_query).await?;
    let res: Vec<_> = get_result
        .embeddings
        .map(|embeddings| embeddings.into_iter().flatten().collect())
        .unwrap_or_default();
    eprintln!("Get result: {:?}", res);
    let prompt = format!(
        "Using this data: {:?}, respond to this prompt: {}",
        res, args.prompt
    );

    let mut cio = Io::new();
    let model_name = get_name(&ollama, &args.model).await?;
    let request =
        GenerationRequest::new(model_name, prompt).options(get_options(args.options.as_deref()).await?);
    let mut stream = ollama.generate_stream(request).await?;
    while let Some(res) = stream.next().await {
        let responses = res?;
        for resp in responses {
            cio.write_line(&resp.response).await?;
        }
    }
    Ok(())
}
