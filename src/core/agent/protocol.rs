use super::types::Context;
use crate::Result;
use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::process::Command;

pub(crate) enum Tool {
    Memorize { content: String },
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

impl Validation {
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
    pub(crate) async fn execute_shell_script(
        script: &str,
        ctx: &mut Context,
        allow_shell: bool,
    ) -> Result<Self> {
        if !allow_shell {
            ctx.rejections
                .push_str("\nShell execution is disabled (pass --allow-shell to enable).");
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
                ctx.rejections
                    .push_str("\nShell Error: command timed out after 30s");
                return Ok(Validation::Failure("timeout".into()));
            }
        };
        match output {
            Ok(output) if output.status.success() => {
                if script.contains(".rs") {
                    let check_res = Self::run_cargo_check().await?;
                    if let Self::Failure(ref err) = check_res {
                        ctx.rejections
                            .push_str(&format!("\nCargo Check Failed: {err}"));
                    }
                    Ok(check_res)
                } else {
                    Ok(Validation::Success)
                }
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr).to_string();
                ctx.rejections.push_str(&format!("\nShell Error: {err}"));
                Ok(Validation::Failure(err))
            }
            Err(e) => {
                ctx.rejections.push_str(&format!("\nShell Error: {e}"));
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
