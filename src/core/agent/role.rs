use super::types::Context;
use std::str::FromStr;
use crate::{RuChatError, Result};
use std::fmt::Display;

pub(crate) enum Role {
    Architect,
    Worker,
    Validator,
    Librarian,
    Critic,
    PerformanceCritic,
    Summarizer,
}

impl Role {
    pub(crate) fn get_color(&self) -> &'static str {
        match self {
            Role::Architect => "\x1b[1;32m",
            Role::Worker => "\x1b[1;34m",
            Role::Validator => "\x1b[1;33m",
            Role::Critic => "\x1b[1;31m",
            Role::PerformanceCritic => "\x1b[1;94m",
            Role::Summarizer => "\x1b[1;35m",
            Role::Librarian => "\x1b[1;36m",
        }
    }
    pub(crate) fn no_color() -> &'static str {
        "\x1b[0m"
    }
    pub(crate) fn build_prompt(&self, system: &str, ctx: &Context, hint: Option<&str>) -> String {
        let hint_section = hint.map_or_else(|| "".to_string(), |h| format!("\nCONTEXTUAL HINT: {h}"));
        match self {
            Role::Architect => format!(
                "{system}{hint_section}\nGOAL: {}\nHISTORY: {}\nTASK: Plan implementation.",
                ctx.get_goal(),
                ctx.history
            ),
            Self::Worker
                => format!(
                "{system}{hint_section}\nDOCUMENTS: {}\nPLAN: {}\nGOAL: {}",
                ctx.documents,
                ctx.context,
                ctx.get_goal()
            ),
            Role::Summarizer => format!("{system}\nRAW HISTORY TO COMPRESS: {}", ctx.history),
            Role::Librarian => format!(
                "{system}{hint_section}\nGOAL: {goal}\nTASK: Formulate a JSON Query. \
                You can query collections: 'technical_docs', 'project_memory', or 'web_cache'.\n\
                OUTPUT FORMAT: {{\"query_texts\": [\"...\"], \"n_results\": 5, \"collection\": \"...\"}}",
                goal = ctx.get_goal()
            ),
            Role::Validator => format!(
                "{system}\nWORKER_OUTPUT: {}\nTASK: Identify technical flaws or incomplete logic. \
                If flawed, respond with 'REJECTED: [reason]'. If perfect, respond with 'VALIDATED'.",
                ctx.output
            ),
            _ => format!(
                "{system}{hint_section}\nGOAL: {}\nCODE/WORK TO REVIEW: {}",
                ctx.get_goal(),
                ctx.context
            ),
        }
    }
    pub(crate) fn update_context(&self, ctx: &mut Context, signal: &str) {
        ctx.history
            .push_str(&format!("### {self} response:\n{}\n\n", ctx.output));
        match self {
            Role::Architect => ctx.context = format!("PLAN:\n{}", ctx.output),
            Role::Worker => ctx.context = format!("IMPLEMENTATION:\n{}", ctx.output),
            Role::Summarizer => ctx.history = format!("SUMMARY OF PREVIOUS EVENTS: {}\n", ctx.output),
            _ => {
                if !ctx.output.contains(signal) {
                    ctx.rejections.push_str(&format!("- {}: {}\n", self, ctx.output));
                }
            }
        }
    }
}

impl FromStr for Role {
    type Err = RuChatError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "architect" => Ok(Role::Architect),
            "worker" => Ok(Role::Worker),
            "validator" => Ok(Role::Validator),
            "librarian" => Ok(Role::Librarian),
            "critic" => Ok(Role::Critic),
            "performancecritic" => Ok(Role::PerformanceCritic),
            "summarizer" => Ok(Role::Summarizer),
            s => Err(RuChatError::InvalidRole(s.to_string())),
        }
    }
}

impl Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let role_str = match self {
            Role::Architect => "Architect",
            Role::Worker => "Worker",
            Role::Validator => "Validator",
            Role::Critic => "Critic",
            Role::PerformanceCritic => "Performance Critic",
            Role::Summarizer => "Summarizer",
            Role::Librarian => "Librarian",
        };
        write!(f, "{role_str}")
    }
}
