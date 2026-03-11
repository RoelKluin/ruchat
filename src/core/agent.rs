pub(crate) mod manager;
pub(crate) mod team;
pub(crate) mod worker;
pub(crate) mod protocol;
pub(crate) mod types;

pub(crate) use team::Team;
use ollama_rs::{Ollama, models::ModelOptions};
use ollama_rs::generation::completion::{GenerationResponse, request::GenerationRequest};
use std::collections::HashMap;
use crate::{Result, RuChatError, options::get_options};
use serde_json::Value;
use tokio_stream::StreamExt;
use tokio::sync::mpsc;
use crate::providers::vector::chroma::query::Query;
use crate::providers::llm::ollama::get_dynamic_history_limit;
use chroma::ChromaHttpClient;
use crate::core::embed::{EmbedArgs, UpsertMode};
use crate::core::orchestrator::TaskType;
use protocol::ToolCall;
use types::Context;

fn get_agent_color(role: &str) -> &str {
    match role {
        "ARCHITECT" => "\x1b[1;32m",
        "WORKER"    => "\x1b[1;34m",
        "VALIDATOR" => "\x1b[1;33m",
        "CRITIC"    => "\x1b[1;31m",
        "PERFORMANCE CRITIC"=> "\x1b[1;94m",
        "SUMMARIZER"   => "\x1b[1;35m",
        _           => "\x1b[0m",
    }
}
const NC: &str = "\x1b[0m";

pub(crate) struct Agent {
    options: ModelOptions,
    config: HashMap<String, Value>,
    pub(super) embed_args: Option<EmbedArgs>
}

impl Agent {
    pub(crate) async fn new(role: &str, options: &str) -> Result<Self> {
        let (options, mut config) = get_options(options).await?;
        config.insert("role".to_string(), Value::String(role.to_string()));
        let embed_args = config.remove("embed_args").and_then(|v| serde_json::from_value(v).ok());

        Ok(Self { options, config, embed_args })
    }
// src/core/agent.rs

    pub(crate) fn apply_task_context(&mut self, task: TaskType) {
        let instruction = match task {
            TaskType::RustRefactor => "Focus on memory safety and idiomatic Result usage.",
            TaskType::GitBisect => "Methodically narrow down the commit range using exit codes.",
            TaskType::ShellAutomation => "Write POSIX compliant scripts with verbose logging.",
            TaskType::DebugCore => "Check for race conditions and verify thread safety.",
            /*TaskType::RustRefactor2 => "Focus on ownership, lifetimes, and idiomatic patterns.",
            TaskType::GitBisect2 => "Analyze commit history to find regression points.",
            TaskType::ShellAutomation2 => "Write robust bash scripts with error handling (set -e).",
            TaskType::DebugCore2 => "Inspect stack traces and memory logs for bottlenecks.",*/
        };
        // Insert into internal config so build_prompt_by_role can see it
        self.config.insert("task_hint".to_string(), serde_json::Value::String(instruction.to_string()));
    }
    pub(crate) fn remove_str(&mut self, key: &str) -> Result<String> {
        self.config.remove(key).and_then(|s| s.as_str().map(|s| s.to_string())).ok_or(RuChatError::Is(format!("No {key} in agent config")))
    }

    pub(crate) fn get_str(&self, key: &str) -> Result<&str> {
        self.config.get(key).and_then(|s| s.as_str()).ok_or(RuChatError::Is(format!("No {key} in agent config")))
    }
    pub (crate) async fn retrieve_and_generate(&self, client: &ChromaHttpClient, ollama: &Ollama, q: Query) -> Result<String> {
        let model = self.get_str("model")?;
        q.query(client, ollama, model).await
    }
    pub(crate) fn get_dynamic_history_limit(&self) -> u64 {
        get_dynamic_history_limit(self.get_str("model").unwrap_or(""))
    }

    fn build_prompt_by_role(&self, role: &str, system: &str, ctx: &Context) -> String {
        let hint = self.get_str("task_hint").unwrap_or_default();
        let hint_section = if hint.is_empty() {
            String::new()
        } else {
            format!("\nCONTEXTUAL HINT: {hint}")
        };

        match role {
            "ARCHITECT" => format!(
                "{system}{hint_section}\nGOAL: {}\nHISTORY: {}\nTASK: Plan implementation.",
                ctx.get_goal(), ctx.history
            ),
            "WORKER" => format!(
                "{system}{hint_section}\nDOCUMENTS: {}\nPLAN: {}\nGOAL: {}",
                ctx.documents, ctx.context, ctx.get_goal()
            ),
            "SUMMARIZER" => format!(
                "{system}\nRAW HISTORY TO COMPRESS: {}",
                ctx.history
            ),
            "LIBRARIAN" => format!(
                "{system}{hint_section}\nGOAL: {goal}\nTASK: Formulate a JSON Query. \
                You can query collections: 'technical_docs', 'project_memory', or 'web_cache'.\n\
                OUTPUT FORMAT: {{\"query_texts\": [\"...\"], \"n_results\": 5, \"collection\": \"...\"}}",
                goal = ctx.get_goal()
            ),
            "VALIDATOR" => format!(
                "{system}\nWORKER_OUTPUT: {}\nTASK: Identify technical flaws or incomplete logic. \
                If flawed, respond with 'REJECTED: [reason]'. If perfect, respond with 'VALIDATED'.",
                ctx.output
            ),
            _ => format!(
                "{system}{hint_section}\nGOAL: {}\nCODE/WORK TO REVIEW: {}",
                ctx.get_goal(), ctx.context
            ),
        }
    }

    fn parse_tool_call(&self, output: &str) -> Option<ToolCall> {
        ToolCall::parse(output)
    }
    pub(super) async fn embed(&self, prompt: &str, mode: UpsertMode) -> Result<()> {
        if let Some(args) = self.embed_args.as_ref() {
            args.embed(prompt, mode).await
        } else {
            EmbedArgs::default().embed(prompt, mode).await
        }
    }

    pub(crate) async fn query_stream(
        &mut self,
        ollama: &Ollama,
        round: u64,
        ctx: &mut Context,
        tx: &mpsc::Sender<Result<Vec<GenerationResponse>>>
    ) -> Result<()> {
        let role = self.get_str("role")?.to_uppercase();
        let role = role.as_str();

        let system = if round == 1 {
            let dense_signal = "Instruction: Use Delimiters (###) for sections. Avoid pleasantries. If providing code, provide ONLY code.";
            format!("SYSTEM: You are the {role} agent. TASK: {}. {}",
                self.get_str("task")?,
                dense_signal)
        } else {
            format!("System: Continue your role as {role}. Focus on high-signal density.")
        };

        // Assemble the payload
        let full_prompt = self.build_prompt_by_role(role, &system, ctx);

        let model = self.get_str("model")?;
        let request = GenerationRequest::new(model.to_string(), full_prompt)
            .options(self.options.clone());

        if let Ok(msg) = self.get_str("status_msg") {
            tx.send(Err(RuChatError::StatusUpdate(msg.to_string()))).await
                .map_err(|e| RuChatError::Is(e.to_string()))?;
        }
        // Inject the color change into the stream
        tx.send(Err(RuChatError::ColorChange(get_agent_color(role).to_string())))
            .await
            .map_err(|e| RuChatError::Is(e.to_string()))?;

        let mut stream = ollama.generate_stream(request).await.map_err(RuChatError::OllamaError)?;
        if  self.get_str("status_msg").is_ok() {
            // Clear the status message after the first chunk arrives
            tx.send(Err(RuChatError::StatusUpdate("\x1b[2K".to_string()))).await
                .map_err(|e| RuChatError::Is(e.to_string()))?;
        }

        ctx.output.clear();
        while let Some(res) = stream.next().await {
            let chunk = res.map_err(RuChatError::OllamaError)?;
            for resp in &chunk {
                ctx.output.push_str(&resp.response);
            }
            tx.send(Ok(chunk)).await.map_err(|e| RuChatError::Is(e.to_string()))?;
        }
        tx.send(Err(RuChatError::ColorChange(NC.to_string())))
            .await
            .map_err(|e| RuChatError::Is(e.to_string()))?;
        let output = &ctx.output;
        if let Some(tool_call) = self.parse_tool_call(&output) {
            match tool_call.name.as_str() {
                "MEMORIZE" => {
                    self.embed(tool_call.content.as_str(), UpsertMode::Upsert).await?;
                    ctx.history.push_str("\n### SYSTEM: Information successfully committed to long-term memory.");
                }
                _ => { /* Handle other tools like shell execution */ }
            }
        }
        ctx.history.push_str(&format!("### {role} response:\n{output}\n\n"));

        match role {
            "ARCHITECT" => ctx.context = format!("PLAN:\n{output}"),
            "WORKER"    => ctx.context = format!("IMPLEMENTATION:\n{output}"),
            "SUMMARIZER" => ctx.history = format!("SUMMARY OF PREVIOUS EVENTS: {}\n", output),
            _ => {
                let signal = self.get_str("approval_signal").unwrap_or("APPROVED");
                if !output.contains(signal) {
                    ctx.rejections.push_str(&format!("- {role}: {output}\n"));
                }
            }
        }
        Ok(())
    }
}
