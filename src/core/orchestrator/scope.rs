use crate::agent::json_extract::strip_json_fences;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct ScopeVerdict {
    pub(crate) verdict: String,
    #[serde(default)]
    pub(crate) clarified_goal: String,
    #[serde(default)]
    pub(crate) information_needed: Vec<Value>,
    #[serde(default)]
    pub(crate) notes: String,
}

/// Parses the Scoper's structured verdict, tolerating ```json fences the same
/// way the Validator/Librarian parsers do. Returns `None` on unparseable
/// output rather than erroring — the caller treats that as "fall through to
/// Plan with the goal as-is" rather than a hard failure.
pub(crate) fn parse_scope_verdict(output: &str) -> Option<ScopeVerdict> {
    serde_json::from_str(strip_json_fences(output)).ok()
}
