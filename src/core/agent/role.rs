use super::types::Context;
use crate::{Result, RuChatError};
use std::fmt::Display;
use std::str::FromStr;

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
            Role::Architect => "\x1b[1;32m[Architect]:\n",
            Role::Worker => "\x1b[1;34m[Worker]:\n",
            Role::Validator => "\x1b[1;33m[Validator]:\n",
            Role::Critic => "\x1b[1;31m[Critic]:\n",
            Role::PerformanceCritic => "\x1b[1;94m[Performance Critic]:\n",
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
            Role::Summarizer => "Compress the history of events into a concise summary",
            Role::Librarian => {
                "Formulate a single vector search query against ChromaDB"
            }
        }
    }
    pub(crate) fn no_color() -> &'static str {
        "\x1b[0m"
    }
    pub(crate) fn build_prompt(&self, task: Option<&str>, ctx: &Context, hint: Option<&str>) -> String {
        let system = format!("SYSTEM: You are the {self} agent.\n");
        let task = format!("TASK: {}.", task.unwrap_or(self.get_task()));
        let hint_section =
            hint.map_or_else(|| "".to_string(), |h| format!("CONTEXTUAL HINT: {h}.\n"));
        let goal = format!("GOAL: {}.", ctx.goal);
        match self {
            Role::Architect if ctx.history.is_empty() => format!(
                "{system}{hint_section}{goal}{task}",
            ),
            Role::Architect => format!(
                "{system}{hint_section}\n{goal}{task}HISTORY: {}.",
                ctx.history
            ),
            Self::Worker => format!(
                "{system}{hint_section}DOCUMENTS: {}\nPLAN: {}\n{goal}{task}",
                ctx.documents,
                ctx.context,
            ),
            Role::Summarizer => format!("{system}{task}RAW HISTORY TO COMPRESS: {}", ctx.history),
            Role::Librarian => {
                    let collections_summary = ctx.build_collections_summary();
                    format!(
                    "{system}{hint_section}{goal}{task}\
                    {collections_summary}\n\n\
                    OUTPUT FORMAT — must be valid JSON, nothing else before or after:\n\
                    {{\n\
                      \"query\": string | [string, string, ...],  // search text(s)\n\
                      \"n_results\": integer,                     // 3-15 recommended\n\
                      \"collection\": string,                     // MUST be one of the names listed above\n\
                      \"where\": string | null,                   // SQL-like filter (see rules below)\n\
                      \"ids\": [string, ...] | null,\n\
                      \"include\": [string, ...] | null           // only from the allowed list above\n\
                    }}\n\n\
                    WHERE FILTER RULES (works for ALL collections):\n\
                    - SQL-style: \"key = 'value' AND key2 > 5\"\n\
                    - Use any metadata key listed for the chosen collection\n\
                    - Special key 'document' for content search: \"document CONTAINS 'foo'\" or \"document REGEX 'pattern'\"\n\
                    - Operators: = != <> > >= < <= IN NOTIN CONTAINS NOTCONTAINS LIKE NOTLIKE REGEX NOTREGEX\n\
                    - Logic: AND OR (parentheses supported)\n\
                    - Values: 'string', 123, true/false, [1,2,3], ['a','b'], or JSON sparse vector {{'indices':[0,5],'values':[0.1,0.9]}}\n\n\
                    EXAMPLES (illustrative — prefer the collection-specific ones from config):\n\
                    1. Simple:\n\
                    {{\n\
                      \"query\": \"error handling\",\n\
                      \"n_results\": 6,\n\
                      \"collection\": \"repo_src-all-minilm_l6-v2\"\n\
                    }}\n\n\
                    2. With filter (copy style from config examples):\n\
                    {{\n\
                      \"query\": [\"async\", \"file reading\"],\n\
                      \"n_results\": 5,\n\
                      \"collection\": \"repo_src-all-minilm_l6-v2\",\n\
                      \"where\": \"lang = 'rust' AND size_bytes > 1000\",\n\
                      \"include\": [\"document\", \"metadata\", \"distance\"]\n\
                    }}\n\n\
                    Return ONLY the JSON. Do not add extra keys. Omit optional fields when not needed."
                )
            },
            Role::Validator => format!(
                "{system}\n{task}WORKER_OUTPUT: {}.\n\
                If flawed, respond with 'REJECTED: [reason]'. If perfect, respond with 'VALIDATED'.",
                ctx.output
            ),
            _ => format!(
                "{system}{hint_section}\n{goal}{task}CODE/WORK TO REVIEW: {}",
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
            Role::Summarizer => {
                ctx.history = format!("SUMMARY OF PREVIOUS EVENTS: {}\n", ctx.output)
            }
            _ => {
                if !ctx.output.contains(signal) {
                    ctx.rejections
                        .push_str(&format!("- {}: {}\n", self, ctx.output));
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
