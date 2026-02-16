mod client;
mod collection;
pub(crate) mod create;
pub(crate) mod delete;
pub(crate) mod get;
mod get_options;
pub(crate) mod ls;
mod metadata;
pub(crate) mod query;
pub(crate) mod similarity;

use crate::{Result, RuChatError};
use chroma::types::MetadataValue;
use serde_json;
use std::fs;
use std::path::Path;

pub(crate) use client::ChromaClientConfigArgs;
pub(crate) use collection::ChromaCollectionConfigArgs;
pub(crate) use metadata::MetadataArgs;

// Chroma metadata is serialized to JSON and stored.
// But filtering (where clause) is still very limited:
//     Only works reliably on top-level scalar fields

///
/// # Parameters
///
/// - `metadata`: An optional string containing metadata.
///
/// # Returns
///
/// A `Result` containing an optional map of metadata or a `RuChatError`.

pub(crate) fn parse_metadata(metadata: &Option<String>) -> Result<Option<MetadataValue>> {
    match metadata {
        Some(input) => {
            let json_content = if std::path::Path::new(input).exists() {
                fs::read_to_string(input)
                    .map_err(|e| RuChatError::MetadataFileReadError(input.clone(), e))?
            } else {
                input.clone()
            };

            let parsed: MetadataValue = serde_json::from_str(&json_content)
                .map_err(|e| RuChatError::MetadataParseError(input.clone(), e))?;

            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_metadata_valid() {
        let metadata_str = Some("key1:value1,key2:value2".to_string());
        let result = parse_metadata(&metadata_str);
        assert!(result.is_ok());
        let metadata = result.unwrap().unwrap();
        assert_eq!(metadata["key1"], "value1".into());
        assert_eq!(metadata["key2"], "value2".into());
    }

    #[test]
    fn test_get_metadata_invalid() {
        let metadata_str = Some("key1value1".to_string());
        let result = parse_metadata(&metadata_str);
        assert!(result.is_err());
    }
}
