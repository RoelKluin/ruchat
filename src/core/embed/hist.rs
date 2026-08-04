use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::core::embed::{EmbedArgs, UpsertMode};
use crate::core::orchestrator::git;
use crate::ollama::OllamaArgs;
use crate::{Result, RuChatError};
use chroma::types::{UpdateMetadata, UpdateMetadataValue};
use clap::Parser;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct HistIngestArgs {
    /// Path (directory or file) to scope git history to.
    #[arg(long, default_value = "src")]
    path: String,

    /// Max number of commits to ingest.
    #[arg(long, default_value_t = 200)]
    max_count: u32,

    #[command(flatten)]
    ollama_args: OllamaArgs,
    #[command(flatten)]
    client_config: ChromaClientConfigArgs,
    #[command(flatten)]
    collection_config: ChromaCollectionConfigArgs,
}

#[derive(Debug)]
struct ParsedCommit {
    author: String,
    date: String,
    message: String,
    diff: String,
}

struct Hunk {
    file: String,
    old_start: u32,
    old_ct: u32,
    new_start: u32,
    new_ct: u32,
    text: String,
}

fn parse_show_output(raw: &str) -> Result<ParsedCommit> {
    let (header, rest) = raw
        .split_once('\n')
        .ok_or_else(|| RuChatError::InternalError("git show: empty output".into()))?;
    let (author, date) = header
        .split_once('\u{1f}')
        .ok_or_else(|| RuChatError::InternalError("git show: malformed header".into()))?;
    let (message, diff) = rest
        .split_once('\u{1e}')
        .ok_or_else(|| RuChatError::InternalError("git show: missing sentinel".into()))?;
    Ok(ParsedCommit {
        author: author.to_string(),
        date: date.to_string(),
        message: message.trim().to_string(),
        diff: diff.to_string(),
    })
}

/// Best-effort unified-diff hunk splitter. Handles the common case (text
/// files, standard `diff --git`/`@@ -a,b +c,d @@` headers). NEEDS
/// VERIFICATION against real repo history containing renames, binary files,
/// and mode-only changes, which this does not specially handle — such
/// sections will simply yield zero hunks for that file rather than erroring.
fn parse_diff_hunks(diff: &str) -> Vec<Hunk> {
    static FILE_RE: OnceLock<Regex> = OnceLock::new();
    static HUNK_RE: OnceLock<Regex> = OnceLock::new();
    let file_re = FILE_RE.get_or_init(|| Regex::new(r"(?m)^diff --git a/(.+?) b/(.+)$").unwrap());
    let hunk_re = HUNK_RE
        .get_or_init(|| Regex::new(r"(?m)^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap());

    let mut hunks = Vec::new();
    let file_starts: Vec<usize> = file_re.find_iter(diff).map(|m| m.start()).collect();
    for (i, &start) in file_starts.iter().enumerate() {
        let end = file_starts.get(i + 1).copied().unwrap_or(diff.len());
        let section = &diff[start..end];
        let file = file_re
            .captures(section)
            .and_then(|c| c.get(2))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();

        let hunk_starts: Vec<usize> = hunk_re.find_iter(section).map(|m| m.start()).collect();
        for (j, &hstart) in hunk_starts.iter().enumerate() {
            let hend = hunk_starts.get(j + 1).copied().unwrap_or(section.len());
            let htext = &section[hstart..hend];
            if let Some(caps) = hunk_re.captures(htext) {
                hunks.push(Hunk {
                    file: file.clone(),
                    old_start: caps[1].parse().unwrap_or(0),
                    old_ct: caps
                        .get(2)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(1),
                    new_start: caps[3].parse().unwrap_or(0),
                    new_ct: caps
                        .get(4)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(1),
                    text: htext.to_string(),
                });
            }
        }
    }
    hunks
}

impl HistIngestArgs {
    pub(crate) async fn ingest(&self, cfg: &Value) -> Result<()> {
        let hashes = git::git_log_hashes(Some(&self.path), Some(self.max_count)).await?;
        let mut items: Vec<(String, UpdateMetadata)> = Vec::new();

        for hash in &hashes {
            let raw = git::git_show(hash).await?;
            let commit = parse_show_output(&raw)?;

            let mut msg_meta = UpdateMetadata::default();
            msg_meta.insert("commit".into(), UpdateMetadataValue::Str(hash.clone()));
            msg_meta.insert(
                "author".into(),
                UpdateMetadataValue::Str(commit.author.clone()),
            );
            msg_meta.insert("date".into(), UpdateMetadataValue::Str(commit.date.clone()));
            msg_meta.insert("type".into(), UpdateMetadataValue::Str("message".into()));
            items.push((commit.message, msg_meta));

            for hunk in parse_diff_hunks(&commit.diff) {
                let mut meta = UpdateMetadata::default();
                meta.insert("commit".into(), UpdateMetadataValue::Str(hash.clone()));
                meta.insert(
                    "author".into(),
                    UpdateMetadataValue::Str(commit.author.clone()),
                );
                meta.insert("date".into(), UpdateMetadataValue::Str(commit.date.clone()));
                meta.insert("file".into(), UpdateMetadataValue::Str(hunk.file));
                meta.insert("type".into(), UpdateMetadataValue::Str("hunk".into()));
                meta.insert(
                    "old_start".into(),
                    UpdateMetadataValue::Int(hunk.old_start as i64),
                );
                meta.insert(
                    "old_ct".into(),
                    UpdateMetadataValue::Int(hunk.old_ct as i64),
                );
                meta.insert(
                    "new_start".into(),
                    UpdateMetadataValue::Int(hunk.new_start as i64),
                );
                meta.insert(
                    "new_ct".into(),
                    UpdateMetadataValue::Int(hunk.new_ct as i64),
                );
                items.push((hunk.text, meta));
            }
        }

        let embed_args = EmbedArgs::new_for_ingestion(
            self.ollama_args.clone(),
            self.client_config.clone(),
            self.collection_config.clone(),
        );
        embed_args
            .embed_raw_items(items, UpsertMode::Upsert, cfg)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Matches `git::git_show`'s real format string exactly:
    // `--format=%an%x1f%ad%n%B%x1e` followed by the patch. Revived 2026-08-04 — this module had
    // zero tests before (it wasn't even compiled in), so these are the first regression coverage
    // for parsing logic that was previously only ever exercised, if at all, before it bit-rotted.
    #[test]
    fn parse_show_output_splits_author_date_message_and_diff() {
        let raw = "Roel Kluin\u{1f}Tue Aug 4 06:29:01 2026 +0200\n\
            Fix a thing\n\n\
            Longer explanation.\u{1e}diff --git a/foo.rs b/foo.rs\n\
            @@ -1,2 +1,3 @@\n+new line\n";
        let commit = parse_show_output(raw).unwrap();
        assert_eq!(commit.author, "Roel Kluin");
        assert_eq!(commit.date, "Tue Aug 4 06:29:01 2026 +0200");
        assert_eq!(commit.message, "Fix a thing\n\nLonger explanation.");
        assert!(commit.diff.starts_with("diff --git a/foo.rs"));
    }

    #[test]
    fn parse_show_output_errors_clearly_on_missing_sentinel() {
        let raw = "Roel Kluin\u{1f}Tue Aug 4 2026\nno record separator here";
        let err = parse_show_output(raw).unwrap_err();
        assert!(format!("{err}").contains("missing sentinel"));
    }

    #[test]
    fn parse_diff_hunks_splits_a_two_file_diff_with_correct_line_ranges() {
        let diff = "diff --git a/a.rs b/a.rs\n\
            index 111..222 100644\n\
            --- a/a.rs\n\
            +++ b/a.rs\n\
            @@ -1,2 +1,3 @@\n\
             fn a() {}\n\
            +fn a2() {}\n\
            diff --git a/b.rs b/b.rs\n\
            index 333..444 100644\n\
            --- a/b.rs\n\
            +++ b/b.rs\n\
            @@ -5,1 +5,2 @@\n\
            -fn b() {}\n\
            +fn b2() {}\n";
        let hunks = parse_diff_hunks(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].file, "a.rs");
        assert_eq!((hunks[0].old_start, hunks[0].old_ct), (1, 2));
        assert_eq!((hunks[0].new_start, hunks[0].new_ct), (1, 3));
        assert_eq!(hunks[1].file, "b.rs");
        assert_eq!((hunks[1].old_start, hunks[1].new_start), (5, 5));
    }

    #[test]
    fn parse_diff_hunks_returns_empty_for_a_non_diff_string() {
        assert!(parse_diff_hunks("not a diff at all").is_empty());
    }
}
