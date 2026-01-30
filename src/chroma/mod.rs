mod client;
mod collection;
pub(crate) mod delete;
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

// Chroma does accept nested metadata in the client — it serializes to JSON and stores it.
//
// But filtering (where clause) is still very limited:
//
//     Only works reliably on top-level scalar fields ($eq, $ne, $gt, $in, $nin, etc.)
//     Nested access via dot notation ("user.name": {"$eq": "Alice"}) — sometimes supported,
//     but inconsistent across versions and backends (especially DuckDB vs ClickHouse vs local)
//     Arrays: $in / $nin on top-level arrays sometimes works, but deep/nested array filtering
//     is weak or broken in many versions
//     Deeply nested objects → often forces you to denormalize or flatten keys (user.address.city)
//
// So while you can store arbitrary nested JSON metadata, you should design it knowing that complex
// filtering may not be possible without flattening.
//
// If your use-case is only storage + retrieval by id,
// or you filter only on top-level keys → full nesting is fine.
//
// If you need rich where filters → prefer flat structure or key.subkey style strings.
//
// Let me know if you want to add basic validation (e.g. reject too-deep nesting,
// reject non-JSON-serializable types, size limits, etc.).

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

    // ────────────────────────────────────────────────
    // Case 1: inline JSON string
    // ────────────────────────────────────────────────
    if let Ok(v) = serde_json::from_str::<Value>(input) {
        return normalize(v);
    }

    // ────────────────────────────────────────────────
    // Case 2: file path pointing to JSON
    // ────────────────────────────────────────────────
    let path = Path::new(input);
    if path.exists() && path.is_file() {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Cannot read metadata file: {}", input))?;

        let v: Value =
            serde_json::from_str(&content).context("File exists but is not valid JSON")?;

        return normalize(v);
    }

    // ────────────────────────────────────────────────
    // Neither inline JSON nor valid JSON file
    // ────────────────────────────────────────────────
    Err(RuChatError::InvalidMetadata(
        "Value is neither valid inline JSON nor a path to an existing valid JSON file".into(),
    ))
}
