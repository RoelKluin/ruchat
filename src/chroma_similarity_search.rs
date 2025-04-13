use crate::error::RuChatError;
use anyhow::Result;
use chromadb::client::{ChromaAuthMethod, ChromaClient, ChromaClientOptions, ChromaTokenHeader};
use chromadb::collection::{ChromaCollection, GetOptions, GetResult, QueryOptions, QueryResult};
use clap::Parser;
use serde_json::json;
use crate::chroma::create_chroma_client;

#[derive(Parser, Debug, Clone)]
pub struct SimilarityArgs {
    #[clap(short, long)]
    pub(crate) query: String,

    #[clap(short, long, default_value = "1")]
    pub(crate) count: usize,

    #[clap(short, long, default_value = "5")]
    pub(crate) similarity_count: usize,

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
        n_results: Some(args.similarity_count),
        include: None,
    };

    let query_result: QueryResult = collection.query(query, None).await?;
    println!("Query result: {:?}", query_result);

    Ok(())
}
