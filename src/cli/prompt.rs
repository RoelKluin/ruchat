use crate::{Result, RuChatError};
use clap::Parser;
use clap::ValueHint::FilePath;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub(crate) struct PromptArgs {
    /// The primary prompt or question. Can be provided as a positional argument.
    #[arg(help = "The prompt to send to the model")]
    prompt: Option<String>,

    /// Explicit prompt flag (alternative to positional).
    #[arg(
        short,
        long,
        help = "Explicitly set the prompt",
        help_heading = "Prompt & Context"
    )]
    explicit_prompt: Option<String>,

    /// Text files to inject into the context.
    /// Supports multiple flags: -i file1.txt -i file2.txt or comma-separated.
    #[arg(
        short = 'i',
        long = "input-files",
        value_delimiter = ',',
        value_hint = FilePath,
        help_heading = "Prompt & Context"
    )]
    files: Vec<PathBuf>,

    /// External command to run to generate input context.
    #[arg(
        short = 'c',
        long,
        default_value = "cat",
        help_heading = "Prompt & Context"
    )]
    command: String,

    /// Arguments to pass to the external command.
    #[arg(short = 'A', long, num_args = 0.., help_heading = "Prompt & Context")]
    args: Vec<String>,

    /// Which output streams to capture from the external command.
    #[arg(
        short = 'S',
        long,
        default_value = "both",
        value_parser = ["stdout", "stderr", "both"],
        help_heading = "Command Output Capture",
        hide_short_help = true, hide_long_help = false
    )]
    capture: String,

    /// Expected exit codes from the command. Errors if code doesn't match.
    #[arg(
        short = 'E',
        long,
        value_delimiter = ',',
        default_value = "0",
        help_heading = "Command Output Capture",
        hide_short_help = true,
        hide_long_help = false
    )]
    allowed_exit_codes: Vec<i32>,
}

fn andify_list<S: AsRef<str>>(what: &str, items: &[S], q: &str) -> String {
    match items.len() {
        0 => String::new(),
        1 => format!("{what} {q}{}{q}", items[0].as_ref()),
        _ => {
            let all_but_last = items[..items.len() - 1]
                .iter()
                .map(|s| s.as_ref())
                .collect::<Vec<_>>();
            let last = items[items.len() - 1].as_ref();
            let concat = format!("{q}, {q}");
            format!(
                "{what}s {q}{}{q} and {q}{last}{q}",
                all_but_last.join(concat.as_str())
            )
        }
    }
}

impl PromptArgs {
    fn promptless(&self, files: &[std::path::PathBuf]) -> Result<String> {
        let v: Vec<String> = files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let files_str = andify_list("the file", &v, "`");
        match self.command {
            ref cmd if cmd == "cat" => {
                let mut combined_content = format!("The contents of {files_str}:\n");
                for path in files {
                    let content = std::fs::read_to_string(path).map_err(|e| {
                        RuChatError::FileReadError(format!("{}: {e}", path.display()))
                    })?;

                    combined_content.push_str(&format!("```\n{}\n```\n", content));
                }
                Ok(combined_content)
            }
            ref cmd => {
                let joined_args = self.args.join(" ");
                let mut combined_content = format!(
                    "The `{cmd} {joined_args}` {} for {files_str}:\n",
                    self.capture
                );

                let mut errors = Vec::new();
                for path in files {
                    let output = std::process::Command::new(cmd)
                        .args(&self.args)
                        .arg(path)
                        .output()?;

                    let status = output.status;
                    let exit_code = status.code().unwrap_or(-1);

                    // Append captured output based on settings
                    if self.capture != "stderr" && !output.stdout.is_empty() {
                        combined_content.push_str(&format!(
                            "Stdout:\n```\n{}\n```\n",
                            String::from_utf8_lossy(&output.stdout)
                        ));
                    }
                    if self.capture != "stdout" && !output.stderr.is_empty() {
                        combined_content.push_str(&format!(
                            "Stderr:\n```\n{}\n```\n",
                            String::from_utf8_lossy(&output.stderr)
                        ));
                    }

                    // Check exit codes using the Vec from clap
                    if self.allowed_exit_codes.contains(&exit_code) {
                        combined_content.push_str(&format!(
                            "`{cmd} {joined_args} {}` exited with status {exit_code}.\n",
                            path.display()
                        ));
                    } else {
                        errors.push(RuChatError::CommandExitError(
                            cmd.to_string(),
                            exit_code.to_string(),
                        ));
                    }
                }
                if !errors.is_empty() {
                    let err_msgs: Vec<String> = errors
                        .iter()
                        .map(|e| match e {
                            RuChatError::CommandExitError(c, code) => {
                                format!("`{c}` exited with code {code}")
                            }
                            _ => "Unknown error".into(),
                        })
                        .collect();
                    Err(RuChatError::MultipleCommandExitErrors(andify_list(
                        "the command",
                        &err_msgs,
                        "`",
                    )))
                } else {
                    Ok(combined_content)
                }
            }
        }
    }

    pub(crate) fn get_prompt(&self) -> Result<String> {
        if self.explicit_prompt.is_some()
            && self.prompt.is_some()
            && self.prompt != self.explicit_prompt
        {
            Err(RuChatError::ConflictingPrompts)
        } else {
            match self.prompt.as_ref().or(self.explicit_prompt.as_ref()) {
                Some(p) if self.files.is_empty() => Ok(p.to_string()),
                Some(p) => {
                    let context = self.promptless(&self.files)?;
                    Ok(format!("{p}\n{context}"))
                }
                None => {
                    if self.files.is_empty() {
                        Err(RuChatError::NoPromptProvided)
                    } else {
                        self.promptless(&self.files)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn args(prompt: Option<&str>, explicit: Option<&str>, files: Vec<PathBuf>) -> PromptArgs {
        PromptArgs {
            prompt: prompt.map(str::to_string),
            explicit_prompt: explicit.map(str::to_string),
            files,
            command: "cat".to_string(),
            args: Vec::new(),
            capture: "both".to_string(),
            allowed_exit_codes: vec![0],
        }
    }

    #[test]
    fn andify_list_empty() {
        assert_eq!(andify_list("the file", &[] as &[&str], "`"), "");
    }

    #[test]
    fn andify_list_one() {
        assert_eq!(andify_list("the file", &["a.txt"], "`"), "the file `a.txt`");
    }

    #[test]
    fn andify_list_two() {
        assert_eq!(
            andify_list("the file", &["a.txt", "b.txt"], "`"),
            "the files `a.txt` and `b.txt`"
        );
    }

    #[test]
    fn andify_list_three() {
        assert_eq!(
            andify_list("the file", &["a.txt", "b.txt", "c.txt"], "`"),
            "the files `a.txt`, `b.txt` and `c.txt`"
        );
    }

    #[test]
    fn get_prompt_positional_only() {
        let a = args(Some("hello"), None, vec![]);
        assert_eq!(a.get_prompt().unwrap(), "hello");
    }

    #[test]
    fn get_prompt_explicit_only() {
        let a = args(None, Some("hello"), vec![]);
        assert_eq!(a.get_prompt().unwrap(), "hello");
    }

    #[test]
    fn get_prompt_matching_positional_and_explicit_is_ok() {
        let a = args(Some("hello"), Some("hello"), vec![]);
        assert_eq!(a.get_prompt().unwrap(), "hello");
    }

    #[test]
    fn get_prompt_conflicting_positional_and_explicit_errors() {
        let a = args(Some("hello"), Some("world"), vec![]);
        assert!(matches!(
            a.get_prompt(),
            Err(RuChatError::ConflictingPrompts)
        ));
    }

    #[test]
    fn get_prompt_none_errors() {
        let a = args(None, None, vec![]);
        assert!(matches!(a.get_prompt(), Err(RuChatError::NoPromptProvided)));
    }

    #[test]
    fn get_prompt_with_prompt_and_files_injects_cat_context() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "file contents").unwrap();

        let a = args(Some("hello"), None, vec![file]);
        let out = a.get_prompt().unwrap();
        assert!(out.starts_with("hello\n"));
        assert!(out.contains("file contents"));
    }

    #[test]
    fn get_prompt_files_only_no_prompt_uses_promptless() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "file contents").unwrap();

        let a = args(None, None, vec![file]);
        let out = a.get_prompt().unwrap();
        assert!(out.contains("file contents"));
        assert!(!out.starts_with("hello"));
    }

    #[test]
    fn get_prompt_missing_file_errors() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.txt");

        let a = args(Some("hello"), None, vec![missing]);
        assert!(matches!(a.get_prompt(), Err(RuChatError::FileReadError(_))));
    }

    #[test]
    fn promptless_external_command_captures_exit_code() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "irrelevant").unwrap();

        let mut a = args(None, None, vec![]);
        a.command = "true".to_string();
        assert!(a.promptless(&[file]).is_ok());
    }

    #[test]
    fn promptless_external_command_rejects_unexpected_exit_code() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "irrelevant").unwrap();

        let mut a = args(None, None, vec![]);
        a.command = "false".to_string();
        assert!(matches!(
            a.promptless(&[file]),
            Err(RuChatError::MultipleCommandExitErrors(_))
        ));
    }
}
