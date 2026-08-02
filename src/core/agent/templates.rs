use crate::{Result, RuChatError};
use std::collections::HashMap;
use std::path::PathBuf;

/// Root directory containing role prompt templates. Resolved relative to
/// CWD, same convention as `orchestrator::search::read_tags`'s repo-root
/// `tags` file — ruchat is expected to run from the repository root.
/// Override with RUCHAT_TEMPLATES_DIR for testing or alternate layouts.
fn templates_dir() -> PathBuf {
    std::env::var("RUCHAT_TEMPLATES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("agent_role"))
}

/// Loads `<templates_dir>/<name>.md` and renders it against `vars`.
pub(crate) fn render(name: &str, vars: &HashMap<&str, String>) -> Result<String> {
    let path = templates_dir().join(format!("{name}.md"));
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        RuChatError::InternalError(format!(
            "failed to load prompt template '{}': {e} — expected it at repo root under agent_role/",
            path.display()
        ))
    })?;
    Ok(render_str(&raw, vars))
}

fn render_str(template: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = strip_conditional_blocks(template, vars);
    for (key, value) in vars {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}

/// Handles `{{#KEY}}...{{/KEY}}` blocks: keeps the inner text (markers
/// stripped) only if `vars[KEY]` is present and non-empty, otherwise drops
/// the whole block. Single-pass, non-nested — sufficient for the flat
/// optional sections these templates use (prior notes, corrections).
fn strip_conditional_blocks(template: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    loop {
        let Some(start) = rest.find("{{#") else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after_hash = &rest[start + 3..];
        let Some(name_end) = after_hash.find("}}") else {
            out.push_str(&rest[start..]);
            break;
        };
        let key = &after_hash[..name_end];
        let close_tag = format!("{{{{/{key}}}}}");
        let body_start = start + 3 + name_end + 2;
        let Some(close_rel) = rest[body_start..].find(&close_tag) else {
            out.push_str(&rest[start..]);
            break;
        };
        let body = &rest[body_start..body_start + close_rel];
        if vars.get(key).is_some_and(|v| !v.is_empty()) {
            out.push_str(body);
        }
        rest = &rest[body_start + close_rel + close_tag.len()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_block_included_when_present() {
        let mut vars = HashMap::new();
        vars.insert("NOTE", "hello".to_string());
        let out = render_str("A{{#NOTE}} B: {{NOTE}}{{/NOTE}} C", &vars);
        assert_eq!(out, "A B: hello C");
    }

    #[test]
    fn conditional_block_omitted_when_absent() {
        let vars = HashMap::new();
        let out = render_str("A{{#NOTE}} B: {{NOTE}}{{/NOTE}} C", &vars);
        assert_eq!(out, "A C");
    }

    #[test]
    fn conditional_block_omitted_when_empty() {
        let mut vars = HashMap::new();
        vars.insert("NOTE", String::new());
        let out = render_str("A{{#NOTE}} B{{/NOTE}} C", &vars);
        assert_eq!(out, "A C");
    }
}
