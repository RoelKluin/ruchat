use clap::Parser;
use crate::error::{Result, RuChatError};
use std::fs;
use std::process::Command;
use clap::ValueEnum;
use std::collections::HashSet;

#[derive(ValueEnum, Clone, Debug, PartialEq)]
enum StdCapture {
    Stdout,
    Stderr,
    Both,
}

impl StdCapture {
    fn as_str(&self) -> &'static str {
        match self {
            StdCapture::Stdout => "Stdout",
            StdCapture::Stderr => "Stderr",
            StdCapture::Both => "Stderr and Stdout",
        }
    }
}

impl ToString for StdCapture {
    fn to_string(&self) -> String {
        match self {
            StdCapture::Stdout => "stdout".to_string(),
            StdCapture::Stderr => "stderr".to_string(),
            StdCapture::Both => "both".to_string(),
        }
    }
}

impl Default for StdCapture {
    fn default() -> Self {
        StdCapture::Both
    }
}

#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub(super) struct PromptArgs {
    /// Prompt to use.
    #[arg(short, long)]
    prompt: Option<String>,

    /// Text files to use as input, separated by commas.
    #[arg(short = 'i', long)]
    files: Option<String>,

    /// Command to read input from, defaults to 'cat'.
    #[arg(short = 'c', long, default_value = "cat")]
    command: String,

    /// Arguments to pass to the command.
    #[arg(short = 'a', long, num_args = 0..)]
    args: Vec<String>,

    /// Capture standard output, standard error, or both.
    #[arg(short = 's', long, default_value = "both")]
    capture: StdCapture,

    /// Allowed exit codes for the command, separated by commas.
    #[arg(short = 'e', long, default_value = "0")]
    allowed_exit_codes: String,

    /// Specify the prompt using a positional argument.
    positional_prompt: Option<String>,
}

fn andify_list<S: AsRef<str>>(what: &str, items: &[S], q: &str) -> String {
    match items.len() {
        0 => String::new(),
        1 => format!("{what} {q}{}{q}", items[0].as_ref()),
        _ => {
            let all_but_last = items[..items.len() - 1].iter().map(|s| s.as_ref()).collect::<Vec<_>>();
            let last = items[items.len() - 1].as_ref();
            let concat = format!("{q}, {q}");
            format!("{what}s {q}{}{q} and {q}{last}{q}", all_but_last.join(concat.as_str()))
        }
    }
}

impl PromptArgs {
    fn promptless(&self, files: &str) -> Result<String> {
        let allowed_exit_codes: HashSet<i32> = HashSet::from_iter(
            self.allowed_exit_codes.split(',')
                .map(|code| code.trim().parse::<i32>())
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| RuChatError::InvalidExitCodeFormat(format!("{e}")))?
        );
        let v = files.split(',').map(str::trim).collect::<Vec<_>>();
        let files_str = andify_list("the file", &v, "`");
        match self.command {
            ref cmd if cmd == "cat" => {
                let mut combined_content = format!("The contents of {files_str}:\n");
                for file in files.split(',') {
                    let file = file.trim();
                    let content = fs::read_to_string(file)
                        .map_err(|e| RuChatError::FileReadError(format!("{}, {e}", file.to_string())))?;
                    combined_content.push_str("```\n");
                    combined_content.push_str(&content);
                    combined_content.push_str("\n```\n");
                }
                Ok(combined_content)
            },
            ref cmd => {
                let args = self.args.iter().map(String::as_str).collect::<Vec<_>>().join(" ");
                let mut combined_content = format!("The `{cmd} {args}` {} for {files_str}:\n", self.capture.as_str());

                let mut command = Command::new(cmd);
                let mut errors = Vec::new();
                for file in files.split(',') {
                    let child = command.args(&self.args).arg(file.trim()).spawn()?;
                    let output = child.wait_with_output()?;
                    let status = output.status;
                    if self.capture == StdCapture::Stdout || self.capture == StdCapture::Both {
                        if !output.stdout.is_empty() {
                            combined_content.push_str("Stdout:\n```\n");
                            combined_content.push_str(&String::from_utf8_lossy(&output.stdout));
                            combined_content.push_str("\n```\n");
                        } else {
                            combined_content.push_str("No stdout.\n");
                        }
                    }
                    if self.capture == StdCapture::Stderr || self.capture == StdCapture::Both {
                        if !output.stderr.is_empty() {
                            combined_content.push_str("Stderr:\n```\n");
                            combined_content.push_str(&String::from_utf8_lossy(&output.stderr));
                            combined_content.push_str("\n```\n");
                        }
                    }
                    if allowed_exit_codes.contains(&status.code().unwrap_or(-1)) {
                        combined_content.push_str(&format!("`{cmd} {args} {file}` exited with status code {}.\n", status.code().unwrap_or(-1)));
                    } else {
                        errors.push(RuChatError::CommandExitError(
                            cmd.to_string(),
                            status.to_string(),
                        ));
                    }
                }
                if !errors.is_empty() {
                    Err(RuChatError::MultipleCommandExitErrors(andify_list(
                        "the command",
                        &errors.iter().map(|e| match e {
                            RuChatError::CommandExitError(cmd, code) => format!("`{cmd}` exited with code {code}"),
                            _ => "Unknown error".to_string(),
                        }).collect::<Vec<_>>(),
                        "`"
                    )))
                } else {
                    Ok(combined_content)
                }
            }

        }
    }

    pub fn get_prompt(&self) -> Result<String> {
        if self.prompt.is_some() && self.positional_prompt.is_some() && self.prompt != self.positional_prompt {
            Err(RuChatError::ConflictingPrompts)
        } else {
            match self.prompt.as_ref().or(self.positional_prompt.as_ref()) {
                Some(p) if self.files.is_none() => Ok(p.to_string()),
                Some(p) => self.promptless(self.files.as_ref().unwrap()).map(|s| format!("{p}\n{s}")),
                None => self.files.as_ref().map_or(Err(RuChatError::NoPromptProvided), |f| self.promptless(f))
            }
        }
    }
}
