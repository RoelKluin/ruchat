use anyhow::Result;
use chromadb::ChromaClient;
use chromadb::client::{ChromaAuthMethod, ChromaClientOptions, ChromaTokenHeader};
use clap::Parser;

#[derive(Parser, Debug, Clone, PartialEq)]
pub struct ChromaClientConfigArgs {
    #[arg(short = 'C', long, default_value = "http://localhost:8000")]
    pub chroma_server: String,
    #[arg(short = 't', long)]
    pub chroma_token: Option<String>,
    #[arg(short = 'd', long, default_value = "default")]
    pub chroma_database: String,
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
                database: self.chroma_database.clone(),
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
