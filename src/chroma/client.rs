use anyhow::Result;
use chromadb::client::{ChromaAuthMethod, ChromaClientOptions, ChromaTokenHeader};
use chromadb::collection::ChromaCollection;
use chromadb::ChromaClient;
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
    #[arg(long, default_value = "default_tenant")]
    pub tenant_id: Option<String>,
    #[arg(short = 'd', long, default_value = "default")]
    pub chroma_database: Option<String>,
}
impl ChromaClientConfigArgs {
    /// Access a running Chroma server to store and retrieve data for embeddings.
    ///
    /// This function creates a client for interacting with a Chroma server. It
    /// supports authentication using tokens and can connect to a specified server
    /// and database.
    ///
    /// # Returns
    ///
    /// A `Result` containing the `ChromaClient` or an error.
    pub async fn create_client(&self) -> Result<ChromaClient> {
        if let Some(token) = self.chroma_token.as_ref() {
            let endpoint = self.chroma_server.parse()?;
            ChromaClient::new(ChromaClientOptions {
                url: Some(endpoint),
                database: self
                    .chroma_database
                    .clone()
                    .unwrap_or("default".to_string()),
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
}
