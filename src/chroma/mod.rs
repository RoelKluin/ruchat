pub(crate) mod ls;
pub(crate) mod query;
pub(crate) mod similarity;

use anyhow::Result;
use chromadb::client::{ChromaAuthMethod, ChromaClient, ChromaClientOptions, ChromaTokenHeader};

/// Access a running Chroma server to store and retrieve data for embeddings.
///
/// This function creates a client for interacting with a Chroma server. It
/// supports authentication using tokens and can connect to a specified server
/// and database.
///
/// # Parameters
///
/// - `token`: An optional token for authentication.
/// - `server`: The URL of the Chroma server.
/// - `db`: The name of the database to connect to.
///
/// # Returns
///
/// A `Result` containing the `ChromaClient` or an error.
///
/// # Example
///
/// You can use the following Docker command to run a Chroma database:
///
/// ```bash
/// docker pull chromadb/chroma
/// # with auth using tokens and persistent storage:
/// docker run -p 8000:8000 -e chroma_server_auth_credentials_provider="chromadb.auth.token.tokenconfigserverauthcredentialsprovider" -e chroma_server_auth_provider="chromadb.auth.token.tokenauthserverprovider" -e chroma_server_auth_token_transport_header="$(sed -n 1p ~/.chroma_creds.txt)" -e chroma_server_auth_credentials="$(sed -n 2p ~/.chroma_creds.txt)" -v ~/chroma_storage/:/chroma/chroma chromadb/chroma
/// ```
pub async fn create_client(
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
