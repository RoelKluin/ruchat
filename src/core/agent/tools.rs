use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;

/// Canonical tool identifiers, shared by the orchestrator's structured
/// tool-call parser and the native `ollama-rs` Coordinator tools registered
/// in `providers::llm::ollama::func`. `Retrieve`/`GitLog`/`GitBlame`/`GitDiff`
/// are stateless and share one implementation across both callers (see
/// `core::orchestrator::git` and `Query::query`); `Memorize`/`ApplyPatch`
/// share only this schema — their execution differs per caller because they
/// need `Agent`/`Context` state a bare Coordinator function doesn't have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolName {
    Memorize,
    ApplyPatch,
    Retrieve,
    GitLog,
    GitBlame,
    GitDiff,
    GitSearchHistory,
    ReadFile,
    ListDir,
    Ripgrep,
    ReadTags,
    CargoCheck,
    CargoDupes,
}

impl ToolName {
    fn schema_hint(self) -> &'static str {
        match self {
            ToolName::Memorize => r#"{"tool":"memorize","content":"<string>"}"#,
            ToolName::ApplyPatch => r#"{"tool":"apply_patch","diff":"<unified diff string>"}"#,
            ToolName::Retrieve => r#"{"tool":"retrieve","query":"<string>"}"#,
            ToolName::GitLog => {
                r#"{"tool":"git_log","path":"<string|omit>","max_count":<int|omit>}"#
            }
            ToolName::GitBlame => r#"{"tool":"git_blame","path":"<string>"}"#,
            ToolName::GitDiff => {
                r#"{"tool":"git_diff","path":"<string|omit>","staged":<bool|omit>}"#
            }
            ToolName::GitSearchHistory => {
                r#"{"tool":"git_search_history","pattern":"<string>","mode":"message"|"content","path":"<string|omit>","max_count":<int|omit>}"#
            }
            ToolName::ReadFile => {
                r#"{"tool":"read_file","path":"<string>","start":<int|omit>,"end":<int|omit>}"#
            }
            ToolName::ListDir => r#"{"tool":"list_dir","path":"<string>"}"#,
            ToolName::Ripgrep => {
                r#"{"tool":"ripgrep","pattern":"<string>","path":"<string|omit>","glob":"<string|omit>","max_count":<int|omit>}"#
            }
            ToolName::ReadTags => r#"{"tool":"read_tags","symbol":"<string|omit>"}"#,
            ToolName::CargoCheck => r#"{"tool":"cargo_check"}"#,
            ToolName::CargoDupes => r#"{"tool":"cargo_dupes"}"#,
        }
    }
    fn required_fields(self) -> &'static [&'static str] {
        match self {
            ToolName::Memorize => &["content"],
            ToolName::ApplyPatch => &["diff"],
            ToolName::Retrieve => &["query"],
            ToolName::GitLog => &[],
            ToolName::GitBlame => &["path"],
            ToolName::GitDiff => &[],
            ToolName::GitSearchHistory => &["pattern", "mode"],
            ToolName::ReadFile => &["path"],
            ToolName::ListDir => &["path"],
            ToolName::Ripgrep => &["pattern"],
            ToolName::ReadTags => &[],
            ToolName::CargoCheck => &[],
            ToolName::CargoDupes => &[],
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StructuredToolCall {
    pub(crate) tool: ToolName,
    pub(crate) args: Value,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ToolParseError {
    #[error("no fenced ```tool_call block found")]
    NotFound,
    #[error("invalid tool_call JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("unknown or missing 'tool' field")]
    UnknownTool,
    #[error("tool call missing required field '{0}'")]
    MissingField(&'static str),
}

fn render_catalog(prepend: &str, tools: &[ToolName]) -> String {
    let mut s = String::new();
    for t in tools {
        s.push_str(&format!("{prepend} {}\n", t.schema_hint()));
    }
    s
}

/// Renders the tool catalog injected into the Worker prompt — the schema
/// strings here are exactly what `parse_tool_call` validates against, so
/// prompt and parser can't drift independently.
pub(crate) fn prompt_tool_catalog(prepend: &str) -> String {
    render_catalog(
        prepend,
        &[
            ToolName::Memorize,
            ToolName::ApplyPatch,
            ToolName::Retrieve,
            ToolName::GitLog,
            ToolName::GitBlame,
            ToolName::GitDiff,
            ToolName::GitSearchHistory,
            ToolName::ReadFile,
            ToolName::ListDir,
            ToolName::Ripgrep,
            ToolName::ReadTags,
            ToolName::CargoCheck,
            ToolName::CargoDupes,
        ],
    )
}

/// Same as `prompt_tool_catalog` but restricted to the read-only lookup
/// tools the Scoper's own JSON schema actually declares as valid `"tool"`
/// values (see `agent_role/scoper.md`) — it must never be offered
/// `memorize`/`apply_patch`/`cargo_check`/`cargo_dupes`, which aren't in
/// that enum and aren't lookups.
pub(crate) fn prompt_scoper_tool_catalog(prepend: &str) -> String {
    render_catalog(
        prepend,
        &[
            ToolName::Retrieve,
            ToolName::GitLog,
            ToolName::GitBlame,
            ToolName::GitDiff,
            ToolName::GitSearchHistory,
            ToolName::ReadFile,
            ToolName::ListDir,
            ToolName::Ripgrep,
            ToolName::ReadTags,
        ],
    )
}

/// Validates an already-parsed tool-call JSON `Value` (no fence/regex
/// extraction) against `ToolName`'s schema. Shared by `parse_tool_call`
/// (which extracts the `Value` from a fenced block first) and any caller
/// that already has a structured `Value` in hand, e.g. the Scoper's
/// `information_needed` array.
pub(crate) fn structured_call_from_value(
    value: Value,
) -> std::result::Result<StructuredToolCall, ToolParseError> {
    let tool_str = value
        .get("tool")
        .and_then(|v| v.as_str())
        .ok_or(ToolParseError::UnknownTool)?;
    let tool: ToolName = serde_json::from_value(Value::String(tool_str.to_string()))
        .map_err(|_| ToolParseError::UnknownTool)?;

    for field in tool.required_fields() {
        if value.get(*field).is_none() {
            return Err(ToolParseError::MissingField(field));
        }
    }
    Ok(StructuredToolCall { tool, args: value })
}

/// Matches a fenced block tagged either ```tool_call``` (what the prompt
/// asks for) or ```json``` (what code-tuned models reliably substitute
/// instead, since "json" is the far more common fence label in their
/// training data). Requiring an explicit tag — rather than any bare fence —
/// keeps this from matching unrelated fenced content elsewhere in the
/// output; the `tool` field validated by `structured_call_from_value` is
/// the real signal either way.
pub(crate) fn parse_tool_call(
    output: &str,
) -> std::result::Result<StructuredToolCall, ToolParseError> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?is)```(?:tool_call|json)\s*\n(.*?)\n```").unwrap()
    });

    let caps = re.captures(output).ok_or(ToolParseError::NotFound)?;
    let json_str = caps.get(1).ok_or(ToolParseError::NotFound)?.as_str();
    let value: Value = serde_json::from_str(json_str)?;
    structured_call_from_value(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_retrieve_call() {
        let input =
            "some text\n```tool_call\n{\"tool\":\"retrieve\",\"query\":\"foo\"}\n```\nmore text";
        let call = parse_tool_call(input).unwrap();
        assert_eq!(call.tool, ToolName::Retrieve);
        assert_eq!(call.args["query"], "foo");
    }

    #[test]
    fn parses_json_fenced_call() {
        // Regression: qwen2.5-coder:14b (and others) reliably tag tool-call
        // blocks ```json instead of the requested ```tool_call, which used
        // to make every such call silently invisible to the orchestrator —
        // see the "Validator always rejects with non-answer" bug report.
        let input = "```json\n{\"tool\":\"ripgrep\",\"pattern\":\"//.*\",\"path\":\".\"}\n```";
        let call = parse_tool_call(input).unwrap();
        assert_eq!(call.tool, ToolName::Ripgrep);
        assert_eq!(call.args["pattern"], "//.*");
    }

    #[test]
    fn rejects_missing_required_field() {
        let input = "```tool_call\n{\"tool\":\"git_blame\"}\n```";
        assert!(matches!(
            parse_tool_call(input),
            Err(ToolParseError::MissingField("path"))
        ));
    }

    #[test]
    fn rejects_unknown_tool() {
        let input = "```tool_call\n{\"tool\":\"nonexistent\"}\n```";
        assert!(matches!(
            parse_tool_call(input),
            Err(ToolParseError::UnknownTool)
        ));
    }
}
