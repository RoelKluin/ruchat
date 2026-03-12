use super::types::Context;
use crate::Result;
use regex::Regex;
use std::process::Command;
use std::sync::OnceLock;

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
            .get_or_init(|| Regex::new(r"### TOOL CALL: (\w+)\n(.*?)\n### END TOOL CALL").unwrap())
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
    pub(crate) async fn execute_shell_script(script: &str, ctx: &mut Context) -> Result<Self> {
        // Logic to run sed and awk script and capture output
        match Command::new("bash").arg("-c").arg(script).output() {
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
    pub(crate) async fn run_cargo_check() -> Result<Self> {
        let output = Command::new("cargo")
            .args(["check"])
            .output()
            .expect("failed to execute cargo check");

        if output.status.success() {
            Ok(Validation::Success)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Ok(Validation::Failure(stderr))
        }
    }
}
