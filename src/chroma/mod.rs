mod client;
mod collection;
pub(crate) mod delete;
pub(crate) mod ls;
pub(crate) mod query;
pub(crate) mod similarity;

use crate::error::RuChatError;
use anyhow::Result;
use serde_json::map::Map;
use serde_json::value::Value;

pub use client::ChromaClientConfigArgs;
pub use collection::ChromaCollectionConfigArgs;

/// Parses metadata from a string of comma-separated key:value pairs.
///
/// # Parameters
///
/// - `arg_metadata`: An optional string containing metadata.
///
/// # Returns
///
/// A `Result` containing an optional map of metadata or a `RuChatError`.
pub(crate) fn get_metadata(
    arg_metadata: &Option<String>,
) -> Result<Option<Map<String, Value>>, RuChatError> {
    if arg_metadata.is_none() {
        return Ok(None);
    }
    let mut metadata = Map::new();
    if let Some(md) = arg_metadata {
        for s in md.split(',') {
            match s.split_once(':') {
                Some((k, v)) => _ = metadata.insert(k.to_string(), Value::String(v.to_string())),
                None => return Err(RuChatError::InvalidMetadata(s.to_string())),
            }
        }
    }
    Ok(Some(metadata))
}
