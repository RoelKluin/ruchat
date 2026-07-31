use crate::Result;
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
}

impl ToolName {
    fn schema_hint(self) -> &'static str {
        match self {
            ToolName::Memorize => r#"{"tool":"memorize","content":"<string>"}"#,
            ToolName::ApplyPatch => r#"{"tool":"apply_patch","diff":"<unified diff string>"}"#,
            ToolName::Retrieve => r#"{"tool":"retrieve","query":"<string>"}"#,
            ToolName::GitLog => r#"{"tool":"git_log","path":"<string|omit>","max_count":<int|omit>}"#,
            ToolName::GitBlame => r#"{"tool":"git_blame","path":"<string>"}"#,
            ToolName::GitDiff => r#"{"tool":"git_diff","path":"<string|omit>","staged":<bool|omit>}"#,
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

/// Extracts and validates a single structured tool call from model output.
/// Expects a fenced block: ```tool_call\n{ ... }\n```. Replaces the previous
/// regex-only `### TOOL CALL: NAME\n...\n### END TOOL CALL` marker format —
/// the JSON payload lets us validate required fields per tool up front
/// instead of the callee discovering a missing field deep in dispatch.
pub(crate) fn parse_tool_call(
    output: &str,
) -> std::result::Result<StructuredToolCall, ToolParseError> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?s)```tool_call\s*\n(.*?)\n```").unwrap());

    let caps = re.captures(output).ok_or(ToolParseError::NotFound)?;
    let json_str = caps.get(1).ok_or(ToolParseError::NotFound)?.as_str();
    let value: Value = serde_json::from_str(json_str)?;

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

/// Renders the tool catalog injected into the Worker prompt — the schema
/// strings here are exactly what `parse_tool_call` validates against, so
/// prompt and parser can't drift independently.
pub(crate) fn prompt_tool_catalog() -> String {
    let mut s = String::from(
        "AVAILABLE TOOLS — to call one, emit a fenced ```tool_call block \
         containing exactly one JSON object matching one of these shapes:\n",
    );
    for t in [
        ToolName::Memorize,
        ToolName::ApplyPatch,
        ToolName::Retrieve,
        ToolName::GitLog,
        ToolName::GitBlame,
        ToolName::GitDiff,
    ] {
        s.push_str(&format!("- {}\n", t.schema_hint()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_retrieve_call() {
        let input = "some text\n```tool_call\n{\"tool\":\"retrieve\",\"query\":\"foo\"}\n```\nmore text";
        let call = parse_tool_call(input).unwrap();
        assert_eq!(call.tool, ToolName::Retrieve);
        assert_eq!(call.args["query"], "foo");
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
        assert!(matches!(parse_tool_call(input), Err(ToolParseError::UnknownTool)));
    }
}
