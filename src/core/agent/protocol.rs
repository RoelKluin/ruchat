use super::types::{Context, TurnKind};
use crate::Result;
use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::process::Command;

pub(crate) enum Tool {
    Memorize { content: String },
    ApplyPatch { diff: String },
}

pub(crate) struct ToolCall {
    pub(crate) name: String,
    pub(crate) content: String,
}
impl ToolCall {
    pub(crate) fn parse(output: &str) -> Option<Self> {
        static REGEX: OnceLock<Regex> = OnceLock::new();
        // Simple string parsing to detect TOOL CALLS in the format: ### TOOL CALL: TOOL_NAME\nCONTENT\n### END TOOL CALL
        REGEX
            .get_or_init(|| {
                Regex::new(r"(?s)### TOOL CALL: (\w+)\n(.*?)\n### END TOOL CALL").unwrap()
            })
            .captures(output)
            .and_then(|caps| {
                Some(Self {
                    name: caps.get(1)?.as_str().to_string(),
                    content: caps.get(2)?.as_str().to_string(),
                })
            })
    }
    pub(crate) fn to_tool(&self) -> Option<Tool> {
        match self.name.as_str() {
            "MEMORIZE" => Some(Tool::Memorize {
                content: self.content.clone(),
            }),
            "APPLY_PATCH" => Some(Tool::ApplyPatch {
                diff: self.content.clone(),
            }),
            _ => None,
        }
    }
}

pub(crate) enum Validation {
    Success,
    Failure(String),
    Skip,
}

/// A single compiler diagnostic parsed from `cargo ... --message-format=json`.
#[derive(Debug, Clone)]
pub(crate) struct Diagnostic {
    pub(crate) level: String, // "error", "warning"
    pub(crate) message: String,
    pub(crate) file: Option<String>,
    pub(crate) line: Option<usize>,
    pub(crate) column: Option<usize>,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.file, self.line, self.column) {
            (Some(file), Some(line), Some(col)) => {
                write!(f, "{file}:{line}:{col}: {}: {}", self.level, self.message)
            }
            _ => write!(f, "{}: {}", self.level, self.message),
        }
    }
}

/// Parses `cargo ... --message-format=json` stdout (one JSON object per line)
/// into `error`/`warning` diagnostics. Lines that aren't JSON, or JSON messages
/// that aren't `reason: "compiler-message"`, are ignored — cargo's json output
/// also emits `build-finished`/`artifact` lines interleaved on the same stream.
fn parse_cargo_json_diagnostics(stdout: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let level = message
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("note")
            .to_string();
        // "note" often just restates an already-reported error/warning; skip to
        // keep the Worker/Validator prompt focused on actionable items.
        if level != "error" && level != "warning" {
            continue;
        }
        let text = message
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let spans = message.get("spans").and_then(|s| s.as_array());
        let primary = spans.and_then(|arr| {
            arr.iter()
                .find(|s| s.get("is_primary").and_then(|p| p.as_bool()) == Some(true))
                .or_else(|| arr.first())
        });

        out.push(Diagnostic {
            level,
            message: text,
            file: primary
                .and_then(|s| s.get("file_name"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            line: primary
                .and_then(|s| s.get("line_start"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize),
            column: primary
                .and_then(|s| s.get("column_start"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize),
        });
    }
    out
}

pub(crate) struct BuildReport {
    pub(crate) compiled: bool,
    pub(crate) tests_passed: bool,
    pub(crate) diagnostics: String,
    /// Structured form of the same diagnostics, reserved for callers that want
    /// file/line/col programmatically rather than the rendered string above.
    #[allow(dead_code)]
    pub(crate) parsed_diagnostics: Vec<Diagnostic>,
}

impl Validation {
    pub(crate) async fn apply_patch(diff_text: &str, ctx: &mut Context) -> Result<Self> {
        let patch = match diffy::Patch::from_str(diff_text) {
            Ok(p) => p,
            Err(e) => {
                let content = format!("Patch parse error: {e}");
                ctx.push_turn(TurnKind::Rejection, "Validator", content);
                return Ok(Validation::Failure(e.to_string()));
            }
        };
        // Resolve target file from the patch header rather than trusting free text elsewhere.
        let target = patch
            .original()
            .unwrap_or("unknown")
            .trim_start_matches("a/");
        let original = tokio::fs::read_to_string(target).await.unwrap_or_default();
        match diffy::apply(&original, &patch) {
            Ok(patched) => {
                tokio::fs::write(target, patched).await?;
                Ok(Validation::Success)
            }
            Err(e) => {
                let content = format!("Patch apply failed on {target}: {e}");
                ctx.push_turn(TurnKind::Rejection, "Validator", content);
                Ok(Validation::Failure(e.to_string()))
            }
        }
    }
    pub(crate) async fn run_cargo_check() -> Result<Self> {
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            Command::new("cargo").args(["check"]).output(),
        )
        .await;
        match output {
            Ok(Ok(output)) if output.status.success() => Ok(Validation::Success),
            Ok(Ok(output)) => {
                let err = String::from_utf8_lossy(&output.stderr).to_string();
                Ok(Validation::Failure(err))
            }
            Ok(Err(e)) => Ok(Validation::Failure(format!(
                "Failed to execute cargo check: {e}"
            ))),
            Err(_) => Ok(Validation::Failure(
                "Cargo check timed out after 30s".into(),
            )),
        }
    }

    pub(crate) async fn run_build_and_test() -> Result<BuildReport> {
        let check = tokio::time::timeout(
            Duration::from_secs(60),
            Command::new("cargo")
                .args(["check", "--message-format=json"])
                .output(),
        )
        .await;
        let (compiled, parsed_diagnostics, mut diagnostics) = match check {
            Ok(Ok(o)) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let parsed = parse_cargo_json_diagnostics(&stdout);
                let rendered = if parsed.is_empty() {
                    // cargo can fail before emitting any JSON (e.g. no Cargo.toml
                    // in cwd) — fall back to raw stderr so nothing is silently lost.
                    String::from_utf8_lossy(&o.stderr).into_owned()
                } else {
                    parsed
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                (o.status.success(), parsed, rendered)
            }
            Ok(Err(e)) => (false, Vec::new(), format!("cargo check failed to run: {e}")),
            Err(_) => (
                false,
                Vec::new(),
                "cargo check timed out after 60s".to_string(),
            ),
        };
        let mut tests_passed = false;
        if compiled {
            let test = tokio::time::timeout(
                Duration::from_secs(120),
                Command::new("cargo")
                    .args(["test", "--", "--nocapture"])
                    .output(),
            )
            .await;
            match test {
                Ok(Ok(o)) => {
                    tests_passed = o.status.success();
                    diagnostics.push_str(&String::from_utf8_lossy(&o.stdout));
                }
                Ok(Err(e)) => diagnostics.push_str(&format!("\ncargo test failed to run: {e}")),
                Err(_) => diagnostics.push_str("\ncargo test timed out after 120s"),
            }
        }
        Ok(BuildReport {
            compiled,
            tests_passed,
            diagnostics,
            parsed_diagnostics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiline_shell_payload() {
        let input = "### TOOL CALL: SHELL\nls -la\necho done\n### END TOOL CALL";
        let call = ToolCall::parse(input).expect("should match multi-line body");
        assert_eq!(call.name, "SHELL");
        assert_eq!(call.content, "ls -la\necho done");
    }
}
