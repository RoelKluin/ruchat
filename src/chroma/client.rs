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
    pub fn create_client(&self) -> Result<ChromaHttpClient> {
        if let Some(token) = self.chroma_token.as_ref() {
            Ok(ChromaHttpClient::new(ChromaHttpClientOptions {
                endpoint: self.chroma_server.parse()?,
                auth_method: ChromaAuthMethod::HeaderAuth {
                    header: HeaderName::from_static("X-Chroma-Token"),
                    value: HeaderValue::from_str(token.as_str())?,
                },
                retry_options: ChromaRetryOptions {
                    max_retries: self.max_retries,
                    min_delay: Duration::from_millis(self.min_delay),
                    max_delay: Duration::from_secs(self.max_delay),
                    jitter: self.jitter,
                },
                tenant_id: self.tenant_id.clone(),
                database_name: self.chroma_database.clone(),
            }))
        } else {
            // Defaults to http://localhost:8000
            Ok(ChromaHttpClient::new(Default::default()))
        }
    }
}
