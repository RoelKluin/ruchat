pub(crate) mod delete;
pub(crate) mod ls;
pub(crate) mod query;
pub(crate) mod similarity;

use crate::error::RuChatError;
use anyhow::Result;
use chroma::client::{ChromaAuthMethod, ChromaHttpClientOptions, ChromaRetryOptions};
use chroma::types::{
    Cmek, Metadata, MetadataValue, Schema, UpdateMetadata, UpdateMetadataValue, ValueTypes,
};
use chroma::ChromaHttpClient;
use clap::Parser;
use http::{HeaderName, HeaderValue};
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<dyn Error + Send + Sync + 'static>>
where
    T: std::str::FromStr,
    T::Err: Error + Send + Sync + 'static,
    U: std::str::FromStr,
    U::Err: Error + Send + Sync + 'static,
{
    match s.split_once('=') {
        Some((key, value)) => Ok((key.parse()?, value.parse()?)),
        None => Err(format!("invalid KEY=VALUE: no `=` found in `{}`", s).into()),
    }
}

#[derive(Parser, Debug, Clone, PartialEq)]
pub struct ChromaCollectionConfigArgs {
    #[arg(short = 'c', long, default_value = "default")]
    pub collection: String,

    #[arg(short = 'm', long, value_name = "KEY=VALUE", value_parser = parse_key_val::<String, String>)]
    pub metadata: HashMap<String, String>,

    #[arg(short = 's', long, value_name = "KEY=VALUE", value_parser = parse_key_val::<String, String>)]
    pub schema: HashMap<String, String>,
}

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
    args: &ChromaCollectionConfigArgs,
) -> Result<String> {
    let schema = match args.schema.is_empty() {
        true => None,
        false => {
            // FIXME: currently no defaults or keys support
            let defaults = ValueTypes {
                string: None,
                float_list: None,
                sparse_vector: None,
                int: None,
                float: None,
                boolean: None,
            };
            let keys: HashMap<String, ValueTypes> = HashMap::new();
            let cmek: Option<Cmek> = args
                .schema
                .get("cmek")
                .cloned()
                .map(|s| Cmek::Gcp(Arc::new(s)));
            let source_attached_function_id =
                args.schema.get("source_attached_function_id").cloned();
            Some(Schema {
                defaults,
                keys,
                cmek,
                source_attached_function_id,
            })
        }
    };
    let metadata: Option<Metadata> = match args.metadata.is_empty() {
        true => None,
        false => {
            let mut md = Metadata::new();
            for (k, v) in args.metadata.iter() {
                _ = md.insert(k.to_string(), MetadataValue::Str(v.to_string()));
            }
            Some(md)
        }
    };
    // Get or create a collection with the given name and no metadata.
    let collection = client
        .get_or_create_collection(collection, schema, metadata)
        .await?;

    // Get the UUID of the collection
    Ok(collection.id().to_string())
}
