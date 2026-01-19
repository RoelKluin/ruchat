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
use chroma::types::{Metadata, MetadataValue, UpdateMetadata, UpdateMetadataValue};
use crate::error::RuChatError;

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
    pub fn get_client_config(&self) -> ChromaClientConfigArgs {
        ChromaClientConfigArgs {
            chroma_server: self.chroma_server.clone(),
            chroma_token: self.chroma_token.clone(),
            max_retries: self.max_retries,
            min_delay: self.min_delay,
            max_delay: self.max_delay,
            jitter: self.jitter,
            tenant_id: self.tenant_id.clone(),
            chroma_database: self.chroma_database.clone(),
        }
    }
}

/// Access a running Chroma server to store and retrieve data for embeddings.
///
/// This function creates a client for interacting with a Chroma server. It
/// supports authentication using tokens and can connect to a specified server
/// and database.
///
/// # Parameters
///
/// - `config`: Configuration arguments for the Chroma client.
///
/// # Returns
///
/// A `Result` containing the `ChromaClient` or an error.
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

/// Parses metadata from a string of comma-separated key:value pairs.
///
/// # Parameters
///
/// - `arg_metadata`: An optional string containing metadata.
///
/// # Returns
///
/// A `Result` containing an optional map of metadata or a `RuChatError`.
fn get_metadata(arg_metadata: &Option<String>) -> Result<Option<Metadata>, RuChatError> {
    if arg_metadata.is_none() {
        return Ok(None);
    }
    let mut metadata = Metadata::new();
    if let Some(md) = arg_metadata {
        for s in md.split(',') {
            match s.split_once(':') {
                Some((k, v)) => {
                    _ = metadata.insert(k.to_string(), MetadataValue::Str(v.to_string()))
                }
                None => return Err(RuChatError::InvalidMetadata(s.to_string())),
            }
        }
    }
    Ok(Some(metadata))
}

/// Parses metadata from a string of comma-separated key:value pairs.
///
/// # Parameters
///
/// - `arg_metadata`: An optional string containing metadata.
///
/// # Returns
///
/// A `Result` containing an optional map of metadata or a `RuChatError`.
fn get_update_metadata(
    arg_metadata: &Option<String>,
) -> Result<Option<Vec<Option<UpdateMetadata>>>, RuChatError> {
    if arg_metadata.is_none() {
        return Ok(None);
    }
    let mut metadata = UpdateMetadata::new();
    if let Some(md) = arg_metadata {
        for s in md.split(',') {
            match s.split_once(':') {
                Some((k, v)) => {
                    _ = metadata.insert(k.to_string(), UpdateMetadataValue::Str(v.to_string()))
                }
                None => return Err(RuChatError::InvalidMetadata(s.to_string())),
            }
        }
    }
    Ok(Some(vec![Some(metadata)]))
}

/// Create a collection in the chroma database
pub async fn get_or_create_chroma_collection(
    client: &ChromaHttpClient,
    collection: &str,
    args: &ChromaClientConfigArgs,
) -> Result<String> {
    // FIXME: currently no schema or metadata support
    let collection_schema = None;
    let collection_metadata = None;
    let client = create_client(&args.get_client_config())?;
    // Get or create a collection with the given name and no metadata.
    let collection = client
        .get_or_create_collection(collection, collection_schema, collection_metadata)
        .await?;

    // Get the UUID of the collection
    Ok(collection.id().to_string())
}
