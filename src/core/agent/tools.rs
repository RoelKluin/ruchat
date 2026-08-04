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
    CargoClippy,
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
                r#"{"tool":"ripgrep","pattern":"<string>","path":"<string|omit>","glob":"<string|omit>","max_count":<int|omit>,"context":<int|omit, lines of context around each match, e.g. to see a whole function around a matched line — max 50>}"#
            }
            ToolName::ReadTags => r#"{"tool":"read_tags","symbol":"<string|omit>"}"#,
            ToolName::CargoCheck => r#"{"tool":"cargo_check"}"#,
            ToolName::CargoClippy => r#"{"tool":"cargo_clippy"}"#,
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
            ToolName::CargoClippy => &[],
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
    #[error("missing 'tool' field")]
    MissingTool,
    #[error("unknown tool '{0}'")]
    UnknownTool(String),
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
            ToolName::CargoClippy,
            ToolName::CargoDupes,
        ],
    )
}

/// Same as `prompt_tool_catalog` but restricted to the read-only lookup
/// tools the Scoper's own JSON schema actually declares as valid `"tool"`
/// values (see `agent_role/scoper.md`) — it must never be offered
/// `memorize`/`apply_patch`/`cargo_check`/`cargo_clippy`/`cargo_dupes`, which
/// aren't in that enum and aren't lookups.
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
    mut value: Value,
) -> std::result::Result<StructuredToolCall, ToolParseError> {
    let tool_str = value
        .get("tool")
        .and_then(|v| v.as_str())
        .ok_or(ToolParseError::MissingTool)?;
    let tool: ToolName = serde_json::from_value(Value::String(tool_str.to_string()))
        .map_err(|_| ToolParseError::UnknownTool(tool_str.to_string()))?;

    // Coder-tuned models sometimes nest apply_patch's diff under a "patch" object
    // (`{"patch": {"path": ..., "diff": ...}}`) instead of the documented flat shape
    // (`{"diff": ...}`) — a live-verified real failure (see TODO.md's pinned reliability item):
    // the Worker copied this exact wrong shape from a hallucinated Architect example for 5
    // rounds straight, and every one of them was rejected as having no recognized tool_call at
    // all, since `diff` is only ever checked at the top level. `path` is ignored either way —
    // apply_patch resolves its target from the diff's own `--- a/<file>` header, not this field.
    if tool == ToolName::ApplyPatch
        && value.get("diff").is_none()
        && let Some(nested_diff) = value.get("patch").and_then(|p| p.get("diff")).cloned()
    {
        value["diff"] = nested_diff;
    }

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
    let re =
        RE.get_or_init(|| regex::Regex::new(r"(?is)```(?:tool_call|json)\s*\n(.*?)\n```").unwrap());

    if let Some(caps) = re.captures(output) {
        let json_str = caps.get(1).ok_or(ToolParseError::NotFound)?.as_str();
        let value: Value = serde_json::from_str(json_str)?;
        return structured_call_from_value(value);
    }

    parse_bare_diff_as_apply_patch(output)
}

/// Falls back to a bare ` ```diff `/` ```patch ` fenced block as an implicit
/// `apply_patch` call. Coder-tuned models (e.g. qwen2.5-coder) reliably
/// answer an edit request with a raw diff instead of the requested
/// `apply_patch` tool_call JSON — the same class of non-compliance the
/// ```json``` fence tolerance above exists for, just for the Worker's
/// terminal action instead of a lookup. Without this, such a diff was
/// silently discarded (`parse_tool_call` returning `NotFound`, treated by
/// `execute_and_verify` as `Validation::Skip`) and the round proceeded as
/// if the Worker had done nothing, burning the iteration budget with no
/// patch ever attempted and no diff-specific feedback ever fed back to it.
fn parse_bare_diff_as_apply_patch(
    output: &str,
) -> std::result::Result<StructuredToolCall, ToolParseError> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re =
        RE.get_or_init(|| regex::Regex::new(r"(?is)```(?:diff|patch)\s*\n(.*?)\n```").unwrap());
    let caps = re.captures(output).ok_or(ToolParseError::NotFound)?;
    let diff = caps.get(1).ok_or(ToolParseError::NotFound)?.as_str();
    if !diff.contains("--- ") || !diff.contains("+++ ") || !diff.contains("@@") {
        return Err(ToolParseError::NotFound);
    }
    Ok(StructuredToolCall {
        tool: ToolName::ApplyPatch,
        args: serde_json::json!({"tool": "apply_patch", "diff": diff}),
    })
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
    fn parses_cargo_clippy_call() {
        let input = "```tool_call\n{\"tool\":\"cargo_clippy\"}\n```";
        let call = parse_tool_call(input).unwrap();
        assert_eq!(call.tool, ToolName::CargoClippy);
    }

    #[test]
    fn cargo_clippy_is_worker_only_not_offered_to_scoper() {
        // Same posture as cargo_check/cargo_dupes: a diagnostic tool for the Worker mid-
        // Implement, not a repo-fact lookup the Scoper's own JSON schema declares.
        assert!(prompt_tool_catalog(">").contains(r#""tool":"cargo_clippy""#));
        assert!(!prompt_scoper_tool_catalog(">").contains(r#""tool":"cargo_clippy""#));
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
    fn parses_bare_diff_fence_as_apply_patch() {
        // Regression: qwen2.5-coder:14b reliably answers an edit request with
        // a raw ```diff fence instead of the requested apply_patch tool_call
        // JSON. Previously parse_tool_call returned NotFound for this, so
        // execute_and_verify treated it as Validation::Skip — no patch was
        // ever attempted and the run proceeded as if the Worker had done
        // nothing, silently burning the iteration budget.
        let input = "```diff\n--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n```";
        let call = parse_tool_call(input).unwrap();
        assert_eq!(call.tool, ToolName::ApplyPatch);
        assert!(call.args["diff"].as_str().unwrap().contains("+new"));
    }

    // Regression: a live-verified run (see TODO.md's pinned reliability item, ruchat_trace_492.md)
    // showed the Worker (copying a hallucinated Architect example) submitting apply_patch with
    // its diff nested under a "patch" object instead of the documented flat "diff" field — every
    // one of 5 rounds was rejected as having no recognized tool_call at all, since `diff` was
    // only ever checked at the top level.
    #[test]
    fn promotes_a_diff_nested_under_a_patch_object_to_the_top_level() {
        let input = "```tool_call\n{\"tool\":\"apply_patch\",\"patch\":{\"path\":\"src/foo.rs\",\
            \"diff\":\"--- a/src/foo.rs\\n+++ b/src/foo.rs\\n@@ -1,1 +1,1 @@\\n-old\\n+new\\n\"}}\n```";
        let call = parse_tool_call(input).unwrap();
        assert_eq!(call.tool, ToolName::ApplyPatch);
        assert!(call.args["diff"].as_str().unwrap().contains("+new"));
    }

    #[test]
    fn a_top_level_diff_is_not_overwritten_by_a_nested_patch_object() {
        let input = "```tool_call\n{\"tool\":\"apply_patch\",\"diff\":\"real\",\
            \"patch\":{\"diff\":\"decoy\"}}\n```";
        let call = parse_tool_call(input).unwrap();
        assert_eq!(call.args["diff"].as_str().unwrap(), "real");
    }

    #[test]
    fn ignores_fenced_block_that_is_not_a_real_diff() {
        let input = "```diff\nnot actually a unified diff\n```";
        assert!(matches!(
            parse_tool_call(input),
            Err(ToolParseError::NotFound)
        ));
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
        let err = parse_tool_call(input).unwrap_err();
        assert!(matches!(err, ToolParseError::UnknownTool(ref t) if t == "nonexistent"));
        assert_eq!(err.to_string(), "unknown tool 'nonexistent'");
    }

    #[test]
    fn rejects_missing_tool_field() {
        let input = "```tool_call\n{\"not_tool\":\"x\"}\n```";
        assert!(matches!(
            parse_tool_call(input),
            Err(ToolParseError::MissingTool)
        ));
    }
}
