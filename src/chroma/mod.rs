mod client;
mod collection;
pub(crate) mod delete;
pub(crate) mod ls;
pub(crate) mod query;
pub(crate) mod similarity;

use crate::error::RuChatError;
use anyhow::Result;
use chroma::types::{Metadata, MetadataValue, UpdateMetadata, UpdateMetadataValue};

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
