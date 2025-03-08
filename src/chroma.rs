use crate::args::QueryArgs;
use crate::error::RuChatError;
use anyhow::Result;
use chromadb::client::{ChromaAuthMethod, ChromaClient, ChromaClientOptions, ChromaTokenHeader};
use chromadb::collection::{ChromaCollection, GetOptions, GetResult};
use serde_json::json;

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

pub(crate) async fn query(args: &QueryArgs) -> Result<(), RuChatError> {
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
    println!("Get result: {:?}", get_result);
    Ok(())
}
