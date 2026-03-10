use crate::{Result, RuChatError};
use chroma::types::{Metadata, UpdateMetadata};
use clap::Parser;
use serde::de::DeserializeOwned;
use std::fs;
use std::path::Path;
use serde::Deserialize;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct MetadataArgs {
    /// An JSON string or a file path to JSON metadata
    #[arg(short, long)]
    metadata: Option<String>,
}

impl MetadataArgs {
    /// Parses the metadata argument and returns an optional map of metadata.
    pub(crate) fn parse(&self) -> Result<Option<Metadata>> {
        self.metadata
            .as_ref()
            .map(|s| parse_metadata::<Metadata>(s))
            .transpose()
    }
}

#[derive(Parser, Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct UpdateMetadataArrayArgs {
    /// An JSON string or a file path to JSON metadata
    #[arg(short, long)]
    metadata: Option<String>,
}

impl UpdateMetadataArrayArgs {
    /// Parses the update metadata argument and returns a map of metadata.
    pub(crate) fn parse(&self) -> Result<Option<Vec<Option<UpdateMetadata>>>> {
        self.metadata
            .as_ref()
            .map(|s| parse_metadata::<Vec<Option<UpdateMetadata>>>(s))
            .transpose()
    }
}

impl Default for UpdateMetadataArrayArgs {
    fn default() -> Self {
        UpdateMetadataArrayArgs { metadata: None }
    }
}

///
/// # Parameters
///
/// - `metadata`: An optional string containing metadata.
///
/// # Returns
///
/// A `Result` containing an optional map of metadata or a `RuChatError`.
fn parse_metadata<T>(metadata: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    // Try to parse as JSON string first
    if let Ok(parsed) = serde_json::from_str::<T>(metadata) {
        return Ok(parsed);
    }

    // If that fails, try to treat it as a file path
    if Path::new(metadata).exists() {
        let file_contents = fs::read_to_string(metadata)
            .map_err(|e| RuChatError::MetadataFileReadError(metadata.to_string(), e))?;

        let parsed = serde_json::from_str::<T>(&file_contents)
            .map_err(|e| RuChatError::MetadataParseError(metadata.to_string(), e))?;

        Ok(parsed)
    } else {
        // If it's neither valid JSON nor a valid file path, return an error
        Err(RuChatError::MetadataFileOrParseError(metadata.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_metadata_valid() {
        let metadata_str = "key1:value1,key2:value2";
        let result = parse_metadata::<Metadata>(&metadata_str);
        assert!(result.is_ok());
        let metadata = result.unwrap();
        assert_eq!(metadata["key1"], "value1".into());
        assert_eq!(metadata["key2"], "value2".into());
    }

    #[test]
    fn test_get_metadata_invalid() {
        let metadata_str = "key1value1";
        let result = parse_metadata::<Metadata>(&metadata_str);
        assert!(result.is_err());
    }
}
