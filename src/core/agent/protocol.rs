use super::types::{Context, TurnKind};
use crate::Result;
use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::process::Command;

pub(crate) enum Tool {
    Memorize { content: String },
    ApplyPatch { diff: String },
    // FIXME: remove:
    Shell { command: String },
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
            // FIXME: remove:
            "SHELL" => Some(Tool::Shell {
                command: self.content.clone(),
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

pub(crate) struct BuildReport {
    pub(crate) compiled: bool,
    pub(crate) tests_passed: bool,
    pub(crate) diagnostics: String,
}

impl Validation {
    pub(crate) async fn apply_patch(diff_text: &str, ctx: &mut Context) -> Result<Self> {
        let patch = match diffy::Patch::from_str(diff_text) {
            Ok(p) => p,
            Err(e) => {
                let round = ctx.turns.last().map_or(0, |t| t.round);
                let content = format!("Patch parse error: {e}");
                ctx.push_turn(round, TurnKind::Rejection, "Validator", content);
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
                let round = ctx.turns.last().map_or(0, |t| t.round);
                let content = format!("Patch apply failed on {target}: {e}");
                ctx.push_turn(round, TurnKind::Rejection, "Validator", content);
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
                .args(["check", "--message-format=short"])
                .output(),
        )
        .await;
        let (compiled, mut diagnostics) = match check {
            Ok(Ok(o)) => (
                o.status.success(),
                String::from_utf8_lossy(&o.stderr).into_owned(),
            ),
            Ok(Err(e)) => (false, format!("cargo check failed to run: {e}")),
            Err(_) => (false, "cargo check timed out after 60s".to_string()),
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
        })
    }

    // FIXME: remove:
    pub(crate) async fn execute_shell_script(
        script: &str,
        ctx: &mut Context,
        allow_shell: bool,
    ) -> Result<Self> {
        if !allow_shell {
            let round = ctx.turns.last().map_or(0, |t| t.round);
            let content = "Shell execution is disabled (pass --allow-shell to enable).".to_string();
            ctx.push_turn(round, TurnKind::Rejection, "Validator", content);
            return Ok(Validation::Skip);
        }
        // Logic to run sed and awk script and capture output
        let run = tokio::time::timeout(
            Duration::from_secs(30),
            Command::new("bash").arg("-c").arg(script).output(),
        )
        .await;
        let output = match run {
            Ok(inner) => inner,
            Err(_) => {
                let round = ctx.turns.last().map_or(0, |t| t.round);
                let content = "Shell command timed out after 30s".to_string();
                ctx.push_turn(round, TurnKind::Rejection, "Validator", content);
                return Ok(Validation::Failure("timeout".into()));
            }
        };
        match output {
            Ok(output) if output.status.success() => {
                if script.contains(".rs") {
                    let check_res = Self::run_cargo_check().await?;
                    if let Self::Failure(ref err) = check_res {
                        let round = ctx.turns.last().map_or(0, |t| t.round);
                        let content = format!("Cargo check failed: {err}");
                        ctx.push_turn(round, TurnKind::Rejection, "Validator", content);
                    }
                    Ok(check_res)
                } else {
                    Ok(Validation::Success)
                }
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr).to_string();
                let round = ctx.turns.last().map_or(0, |t| t.round);
                let content = format!("Shell command failed: {err}");
                ctx.push_turn(round, TurnKind::Rejection, "Validator", content);
                Ok(Validation::Failure(err))
            }
            Err(e) => {
                let round = ctx.turns.last().map_or(0, |t| t.round);
                let content = format!("Shell command execution error: {e}");
                ctx.push_turn(round, TurnKind::Rejection, "Validator", content);
                Ok(Validation::Failure(format!(
                    "Failed to execute sed/awk: {e}"
                )))
            }
        }
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
