/// Hard-wraps `line` at `max_len` characters, breaking on word boundaries — shared by
/// `orchestrator::git::wrap_commit_message_body` (80 chars, a backstop for commit message
/// bodies) and `orchestrator::run_summary`'s trace-summary wrapping (120 chars). A single word
/// longer than `max_len` is left intact rather than split mid-word.
pub(crate) fn wrap_line(line: &str, max_len: usize) -> String {
    if line.chars().count() <= max_len {
        return line.to_string();
    }
    let mut wrapped = String::new();
    let mut current_len = 0;
    for word in line.split(' ') {
        let word_len = word.chars().count();
        if current_len == 0 {
            wrapped.push_str(word);
            current_len = word_len;
        } else if current_len + 1 + word_len <= max_len {
            wrapped.push(' ');
            wrapped.push_str(word);
            current_len += 1 + word_len;
        } else {
            wrapped.push('\n');
            wrapped.push_str(word);
            current_len = word_len;
        }
    }
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_line_leaves_short_lines_alone() {
        let line = "short line";
        assert_eq!(wrap_line(line, 80), line);
    }

    #[test]
    fn wrap_line_breaks_long_lines_at_word_boundaries_under_the_limit() {
        let line = "This explains why the change was made in quite a bit more detail \
            than usual, spanning well past the eighty character line limit we want to enforce.";
        let wrapped = wrap_line(line, 80);
        for l in wrapped.lines() {
            assert!(
                l.chars().count() <= 80,
                "line exceeds 80 chars: {l:?} ({} chars)",
                l.chars().count()
            );
        }
        // No words lost or reordered by wrapping.
        assert_eq!(
            wrapped.replace('\n', " "),
            line,
            "wrapping must not change the words themselves"
        );
    }

    #[test]
    fn wrap_line_respects_a_different_max_len() {
        let line = "one two three four five six seven eight nine ten eleven twelve";
        let wrapped = wrap_line(line, 20);
        for l in wrapped.lines() {
            assert!(l.chars().count() <= 20, "line exceeds 20 chars: {l:?}");
        }
    }

    #[test]
    fn wrap_line_does_not_split_a_single_word_longer_than_max_len() {
        let line = "supercalifragilisticexpialidocious";
        assert_eq!(wrap_line(line, 10), line);
    }
}
