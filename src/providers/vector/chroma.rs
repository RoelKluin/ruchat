mod client;
mod collection;
pub(crate) mod create;
pub(crate) mod delete;
mod get_options;
pub(crate) mod ls;
mod metadata;
pub(crate) mod query;
pub(crate) mod similarity;

use crate::error::RuChatError;
use anyhow::{Context, Result};
use serde_json::{map::Map, Value};
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
pub(crate) fn parse_metadata(
    metadata: &Option<String>,
) -> Result<Option<Map<String, Value>>, RuChatError> {
    let input = match metadata.as_deref() {
        None | Some("") => return Ok(None),
        Some(s) => s.trim(),
    };

    // Helper to normalize Value → Option<Map<String, Value>>
    fn normalize(v: Value) -> Result<Option<Map<String, Value>>, RuChatError> {
        match v {
            Value::Object(map) => Ok(Some(map)),
            Value::Null => Ok(None),
            other => Err(RuChatError::InvalidMetadata(format!(
                "Metadata root must be JSON object {{ ... }} or null, got {other}"
            ))),
        }
    }

    // Case 1: inline JSON string
    if let Ok(v) = serde_json::from_str::<Value>(input) {
        return normalize(v);
    }

    // Case 2: file path pointing to JSON
    let path = Path::new(input);
    if path.exists() && path.is_file() {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Cannot read metadata file: {}", input))?;

        let v: Value =
            serde_json::from_str(&content).context("File exists but is not valid JSON")?;

        return normalize(v);
    }

    // Neither inline JSON nor valid JSON file
    Err(RuChatError::InvalidMetadata(
        "Value is neither valid inline JSON nor a path to an existing valid JSON file".into(),
    ))
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
