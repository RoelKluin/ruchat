mod client;
mod collection;
pub(crate) mod create;
pub(crate) mod delete;
pub(crate) mod get;
pub(crate) mod ls;
pub(crate) mod query;
pub(crate) mod parser;

use crate::{Result, RuChatError};
use serde_json;
use std::fs;
use std::path::Path;

pub(crate) use client::ChromaClientConfigArgs;
pub(crate) use collection::ChromaCollectionConfigArgs;
pub(crate) use parser::parse_where;

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


pub(crate) fn parse_metadata<T>(
    metadata: &Option<String>,
) -> Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    match metadata {
        None => Ok(None),
        Some(input) => {
            // Try to parse as JSON string first
            if let Ok(parsed) = serde_json::from_str::<T>(input) {
                return Ok(Some(parsed));
            }

            // If that fails, try to treat it as a file path
            if Path::new(input).exists() {
                let file_contents = fs::read_to_string(input)
                    .map_err(|e|  RuChatError::MetadataFileReadError(input.clone(), e))?;

                let parsed = serde_json::from_str::<T>(&file_contents)
                    .map_err(|e| RuChatError::MetadataParseError(input.clone(), e))?;

                return Ok(Some(parsed));
            }

            // If it's neither valid JSON nor a valid file path, return an error
            Err(RuChatError::MetadataFileOrParseError(input.clone()))
        }
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
