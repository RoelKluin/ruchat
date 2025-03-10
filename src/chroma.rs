use crate::chat_io::ChatIO;
use crate::error::RuChatError;
use crate::ollama::get_model_name;
use crate::ollama_ask::get_options;
use anyhow::Result;
use chromadb::client::{ChromaAuthMethod, ChromaClient, ChromaClientOptions, ChromaTokenHeader};
use chromadb::collection::{ChromaCollection, GetOptions, GetResult, QueryOptions, QueryResult};
use clap::Parser;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::Ollama;
use serde_json::json;
use tokio_stream::StreamExt;

/// access a running chroma server to store and retrieve data for embeddings
// You can use the following docker command to run a chroma database:
// docker pull chromadb/chroma
// # with auth using tokens and persistent storage:
// docker run -p 8000:8000 -e chroma_server_auth_credentials_provider="chromadb.auth.token.tokenconfigserverauthcredentialsprovider" -e chroma_server_auth_provider="chromadb.auth.token.tokenauthserverprovider" -e chroma_server_auth_token_transport_header="$(sed -n 1p ~/.chroma_creds.txt)" -e chroma_server_auth_credentials="$(sed -n 2p ~/.chroma_creds.txt)" -v ~/chroma_storage/:/chroma/chroma chromadb/chroma
pub async fn create_chroma_client(
    token: Option<&str>,
    server: &str,
    db: &str,
) -> Result<ChromaClient> {
    if let Some(token) = token {
        ChromaClient::new(ChromaClientOptions {
            url: Some(server.to_string()),
            database: db.to_string(),
            auth: ChromaAuthMethod::TokenAuth {
                token: token.to_string(),
                header: ChromaTokenHeader::Authorization,
            },
        })
        .await
    } else {
        // Defaults to http://localhost:8000
        ChromaClient::new(Default::default()).await
    }
}

/// Create a collection in the chroma database
pub async fn get_or_create_chroma_collection(
    client: &ChromaClient,
    collection: &str,
) -> Result<String> {
    // Get or create a collection with the given name and no metadata.
    let collection: ChromaCollection = client.get_or_create_collection(collection, None).await?;

    // Get the UUID of the collection
    Ok(collection.id().to_string())
}

#[derive(Parser, Debug, Clone)]
pub struct QueryArgs {
    #[clap(short, long, default_value = "qwen2.5-coder:14b")]
    pub(crate) model: String,

    #[clap(short, long)]
    pub(crate) config: Option<String>,

    #[clap(short, long)]
    pub(crate) query: String,

    #[clap(short, long)]
    pub(crate) prompt: String,

    #[clap(short, long, default_value = "1")]
    pub(crate) count: usize,

    /// Chroma database collection name
    #[clap(short, long, default_value = "default")]
    pub(crate) collection: String,

    /// Chroma database metadata, comma separated key:value pairs
    #[clap(short, long)]
    pub(crate) metadata: Option<String>,

    /// Chroma database server address and port
    #[clap(short = 'C', long, default_value = "http://localhost:8000")]
    pub(crate) chroma_server: String,

    /// Chroma database name
    #[clap(short = 'd', long, default_value = "default")]
    pub(crate) chroma_database: String,

    /// Chroma token for authentication
    #[clap(short = 't', long)]
    pub(crate) chroma_token: Option<String>,
}

pub(crate) async fn query(ollama: Ollama, args: &QueryArgs) -> Result<(), RuChatError> {
    let client = create_chroma_client(
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

    let mut cio = ChatIO::new();
    let model_name = get_model_name(&ollama, &args.model).await?;
    let request =
        GenerationRequest::new(model_name, prompt).options(get_options(&args.config).await?);
    let mut stream = ollama.generate_stream(request).await?;
    while let Some(res) = stream.next().await {
        let responses = res?;
        for resp in responses {
            cio.write_line(&resp.response).await?;
        }
    }
    Ok(())
}
#[derive(Parser, Debug, Clone)]
pub struct SimilarityArgs {
    #[clap(short, long)]
    pub(crate) query: String,

    #[clap(short, long, default_value = "1")]
    pub(crate) count: usize,

    /// Chroma database collection name
    #[clap(short, long, default_value = "default")]
    pub(crate) collection: String,

    /// Chroma database metadata, comma separated key:value pairs
    #[clap(short, long)]
    pub(crate) metadata: Option<String>,

    /// Chroma database server address and port
    #[clap(short = 'C', long, default_value = "http://localhost:8000")]
    pub(crate) chroma_server: String,

    /// Chroma database name
    #[clap(short = 'd', long, default_value = "default")]
    pub(crate) chroma_database: String,

    /// Chroma token for authentication
    #[clap(short = 't', long)]
    pub(crate) chroma_token: Option<String>,
}

pub(crate) async fn similarity_search(args: &SimilarityArgs) -> Result<(), RuChatError> {
    // Instantiate a ChromaClient to connect to the Chroma database
    let client = create_chroma_client(
        args.chroma_token.as_deref(),
        &args.chroma_server,
        &args.chroma_database,
    )
    .await?;

    // Instantiate a ChromaCollection to perform operations on a collection
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

    // FIXME: This is a placeholder for the actual embeddings
    // Instantiate QueryOptions to perform a similarity search on the collection
    // Alternatively, an embedding_function can also be provided with query_texts to perform the search
    let query = QueryOptions {
        query_texts: None,
        query_embeddings: get_result
            .embeddings
            .map(|embeddings| embeddings.into_iter().flatten().collect()),
        where_metadata: None,
        where_document: None,
        n_results: Some(5),
        include: None,
    };

    let query_result: QueryResult = collection.query(query, None).await?;
    println!("Query result: {:?}", query_result);

    Ok(())
}
