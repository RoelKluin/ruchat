use anyhow::Result;
use chromadb::client::{ChromaAuthMethod, ChromaClient, ChromaClientOptions, ChromaTokenHeader};
use chromadb::collection::ChromaCollection;

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
