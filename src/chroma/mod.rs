pub(crate) mod delete;
pub(crate) mod ls;
pub(crate) mod query;
pub(crate) mod similarity;

use anyhow::Result;
use chroma::client::{ChromaAuthMethod, ChromaHttpClientOptions, ChromaRetryOptions};
use chroma::ChromaHttpClient;
use clap::Parser;
use http::{HeaderName, HeaderValue};
use std::time::Duration;

#[derive(Parser, Debug, Clone, PartialEq)]
pub struct ChromaClientConfigArgs {
    #[arg(short = 'C', long, default_value = "http://localhost:8000")]
    pub chroma_server: String,
    #[arg(short = 't', long)]
    pub chroma_token: Option<String>,
    #[arg(long, default_value_t = 3)]
    pub max_retries: usize,
    #[arg(long, default_value_t = 100)]
    pub min_delay: u64,
    #[arg(long, default_value_t = 10)]
    pub max_delay: u64,
    #[arg(long, default_value_t = true)]
    pub jitter: bool,
    #[arg(long, default_value = "")]
    pub tenant_id: Option<String>,
    #[arg(short = 'd', long, default_value = "default")]
    pub chroma_database: Option<String>,
}

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
pub fn create_client(config: &ChromaClientConfigArgs) -> Result<ChromaHttpClient> {
    if let Some(token) = config.chroma_token.as_ref() {
        Ok(ChromaHttpClient::new(ChromaHttpClientOptions {
            endpoint: config.chroma_server.parse()?,
            auth_method: ChromaAuthMethod::HeaderAuth {
                header: HeaderName::from_static("X-Chroma-Token"),
                value: HeaderValue::from_str(token.as_str())?,
            },
            retry_options: ChromaRetryOptions {
                max_retries: config.max_retries,
                min_delay: Duration::from_millis(config.min_delay),
                max_delay: Duration::from_secs(config.max_delay),
                jitter: config.jitter,
            },
            tenant_id: config.tenant_id.clone(),
            database_name: config.chroma_database.clone(),
        }))
    } else {
        // Defaults to http://localhost:8000
        Ok(ChromaHttpClient::new(Default::default()))
    }
}

/// Create a collection in the chroma database
pub async fn get_or_create_chroma_collection(
    client: &ChromaHttpClient,
    collection: &str,
) -> Result<String> {
    // Get or create a collection with the given name and no metadata.
    let collection = client
        .get_or_create_collection(collection, None, None)
        .await?;

    // Get the UUID of the collection
    Ok(collection.id().to_string())
}
