use super::types::{Context, TurnKind};
use crate::agent::tools::{prompt_scoper_tool_catalog, prompt_tool_catalog};
use crate::core::agent::templates;
use crate::{Result, RuChatError};
use std::collections::HashMap;
use std::fmt::Display;
use std::str::FromStr;

pub(crate) enum Role {
    Architect,
    Worker,
    Validator,
    Librarian,
    Critic,
    PerformanceCritic,
    Scoper,
    Summarizer,
}

impl Role {
    /// Splits the previously-concatenated prompt into a system message
    /// (role framing, task, tool catalog) and a user message (goal +
    /// retrieved/untrusted content), so the two ride as distinct chat
    /// messages instead of one string. Callers needing the old single-string
    /// shape (e.g. `ctx.trace` logging) can still concatenate the pair.
    pub(crate) fn build_chat_messages(
        &self,
        task: Option<&str>,
        ctx: &Context,
        hint: Option<&str>, // still accepted for signature compatibility;
        // needs verification which arms actually use it —
        // none of the templates above reference it, so
        // check callers before dropping the parameter entirely
        approval_signal: Option<&str>,
    ) -> Result<(String, String)> {
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("GOAL", ctx.goal.clone());
        let system = format!(
            "You are the {self} agent. TASK: {}.{}",
            task.unwrap_or(self.get_task()),
            hint.map_or_else(String::new, |h| format!(" CONTEXTUAL HINT: {h}.")),
        );

        let template_name = match self {
            Self::Scoper => {
                vars.insert("DOCUMENTS", ctx.documents_view(ctx.round));
                vars.insert("COLLECTIONS", ctx.build_collections_summary());
                vars.insert("TOOL_CATALOG", prompt_scoper_tool_catalog("// "));
                let prior_notes: String = ctx
                    .turns
                    .iter()
                    .filter(|t| t.round == ctx.round && t.kind == TurnKind::System)
                    .map(|t| t.content.clone())
                    .collect::<Vec<_>>()
                    .join("\n");
                vars.insert("PRIOR_NOTES", prior_notes);
                "scoper"
            }
            Self::Architect => {
                vars.insert("PLAN", ctx.context_view());
                vars.insert("DOCUMENTS", ctx.documents_view(ctx.round));
                vars.insert("HISTORY", ctx.history_view(ctx.round.saturating_sub(1)));
                "architect"
            }
            Self::Worker => {
                vars.insert("DOCUMENTS", ctx.documents_view(ctx.round));
                vars.insert("PLAN", ctx.context_view());
                vars.insert("HISTORY", ctx.history_view(ctx.round.saturating_sub(1)));
                vars.insert("TOOLS", prompt_tool_catalog("-"));
                "worker"
            }
            Self::Validator => {
                vars.insert("WORKER_OUTPUT", ctx.output.clone());
                "validator"
            }
            Self::Summarizer => {
                vars.insert("HISTORY", ctx.history_view(ctx.round));
                "summarizer"
            }
            Self::Librarian => {
                vars.insert("COLLECTIONS", ctx.build_collections_summary());
                let correction: String = ctx
                    .turns
                    .iter()
                    .filter(|t| t.round == ctx.round && t.kind == TurnKind::System)
                    .map(|t| t.content.clone())
                    .collect::<Vec<_>>()
                    .join("\n");
                vars.insert("CORRECTION", correction);
                "librarian"
            }
            Self::Critic | Self::PerformanceCritic => {
                vars.insert("CODE", ctx.context_view());
                vars.insert("SIGNAL", approval_signal.unwrap_or("APPROVED").to_string());
                vars.insert(
                    "CONCERN",
                    task.unwrap_or("general code quality").to_string(),
                );
                "critic"
            }
        };

        let user_text = templates::render(template_name, &vars)?;
        Ok((system, user_text)) // adjust first element if any role previously had
                                // real system-prompt content distinct from user_text —
                                // needs verification against the pre-refactor arms'
                                // actual tuple shape, which wasn't fully shown to me
    }

    pub(crate) fn get_color(&self) -> &'static str {
        match self {
            Role::Architect => "\x1b[1;32m[Architect]:\n",
            Role::Worker => "\x1b[1;34m[Worker]:\n",
            Role::Validator => "\x1b[1;33m[Validator]:\n",
            Role::Critic => "\x1b[1;31m[Critic]:\n",
            Role::PerformanceCritic => "\x1b[1;94m[Performance Critic]:\n",
            Role::Scoper => "\x1b[1;96m[Scoper]:\n",
            Role::Summarizer => "\x1b[1;35m[Summarizer]:\n",
            Role::Librarian => "\x1b[1;36m[Librarian]:\n",
        }
    }
    pub(crate) fn get_task(&self) -> &'static str {
        match self {
            Role::Architect => "Write a plan for the worker agent to implement",
            Role::Worker => "Implement the plan.",
            Role::Validator => "Identify technical flaws or incomplete logic",
            Role::Critic => "Identify any issues in the work",
            Role::PerformanceCritic => "Identify any performance issues in the work",
            Role::Scoper => "Determine whether enough repo-specific information is known to plan a solution; if not, specify what to look up",
            Role::Summarizer => "Compress the history of events into a concise summary",
            Role::Librarian => "Formulate a single vector search query against ChromaDB",
        }
    }
    pub(crate) fn no_color() -> &'static str {
        "\x1b[0m"
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
            "performancecritic" => Ok(Role::PerformanceCritic),
            "scoper" => Ok(Role::Scoper),
            "summarizer" => Ok(Role::Summarizer),
            // Multiple critics are named "Critic_0", "Critic_1", ... (see
            // Orchestrator::new / debug_stage_machine) so each Agent's `role`
            // config value is that indexed name, not the bare "critic" this
            // match used to require exclusively — every multi-critic agent
            // failed `query_stream` with InvalidRole until this was added.
            s if s == "critic" || s.starts_with("critic_") => Ok(Role::Critic),
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
            Role::Scoper => "Scoper",
            Role::Summarizer => "Summarizer",
            Role::Librarian => "Librarian",
        };
        write!(f, "{role_str}")
    }
}
