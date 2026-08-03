pub(crate) mod manager;
#[cfg(test)]
mod evals;
pub(crate) mod event;
pub(crate) mod json_extract;
pub(crate) mod llm_client;
pub(crate) mod pipeline;
pub(crate) mod protocol;
pub(crate) mod role;
pub(crate) mod templates;
pub(crate) mod team;
pub(crate) mod tokens;
pub(crate) mod tools;
pub(crate) mod types;

use crate::core::embed::{EmbedArgs, UpsertMode};
use crate::core::orchestrator::TaskType;
use event::{send_event, AgentEvent, StreamItem};
use crate::providers::llm::ollama::get_dynamic_history_limit;
use crate::providers::vector::chroma::query::Query;
use crate::{Result, RuChatError, options::get_options};
use llm_client::{LlmClient, VectorStore};
use ollama_rs::generation::chat::ChatMessage;
use ollama_rs::models::ModelOptions;
use protocol::Validation;
use role::Role;
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
pub(crate) use team::Team;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use types::{Context, TurnKind};

pub(crate) struct Agent {
    options: ModelOptions,
    agent_config: HashMap<String, Value>,
    pub(super) embed_args: Option<EmbedArgs>,
    cfg: Value,
}

impl Agent {
    pub(crate) async fn new(
        config: &mut Value,
        role: &str,
        required: bool,
        task_type: Option<&TaskType>,
        cfg: Value,
    ) -> Result<Self> {
        if let Some(agent_val) = config.get(role) {
            // Check if it's a raw JSON string (from CLI) or an Object (from json! macro)
            let options_str = match agent_val.as_str() {
                Some(s) => s.to_string(),
                None => agent_val.to_string(),
            };
            let (options, mut agent_config) = get_options(&options_str).await?;
            agent_config.insert("role".to_string(), Value::String(role.to_string()));
            if let Some(task) = task_type {
                agent_config.insert(
                    "task_hint".to_string(),
                    serde_json::Value::String(task.to_string()),
                );
            }

            let embed_args = agent_config
                .remove("embed_args")
                .and_then(|v| serde_json::from_value(v).ok());

            Ok(Self {
                options,
                agent_config,
                embed_args,
                cfg
            })
        } else if required {
            Err(RuChatError::MissingAgent(role.to_string()))
        } else {
            Err(RuChatError::Is("Optional agent missing".into()))
        }
    }
    pub(crate) fn remove_str(&mut self, key: &str) -> Result<String> {
        let v = self.agent_config.remove(key).ok_or(RuChatError::Is(format!(
            "No {key} to remove in agent config"
        )))?;
        if let Some(s) = v.as_str() {
            Ok(s.to_string())
        } else if v.is_object() {
            Ok(v.to_string())
        } else {
            Err(RuChatError::Is(format!(
                "Value for {key} is not a string in agent config {:?})",
                self.agent_config
            )))
        }
    }

    pub(crate) fn get_str(&self, key: &str) -> Result<&str> {
        self.agent_config
            .get(key)
            .ok_or(RuChatError::Is(format!("No {key} in agent config")))?
            .as_str()
            .ok_or(RuChatError::Is(format!(
                "Value for {key} is not a string in agent config {:?})",
                self.agent_config
            )))
    }
    pub(crate) async fn retrieve_and_generate(
        &self,
        client: &dyn VectorStore,
        ollama: &dyn LlmClient,
        q: Query,
    ) -> Result<String> {
        let model = self.get_str("embed_model").unwrap_or("all-minilm:l6-v2");
        q.query(client, ollama, model).await
    }
    pub(crate) fn get_dynamic_history_limit(&self) -> u64 {
        get_dynamic_history_limit(self.get_str("model").unwrap_or(""))
    }

    pub(super) async fn embed(
        &self,
        prompt: &str,
        mode: UpsertMode,
        ctx: &mut Context,
        msg: &str,
    ) -> Result<()> {
        if let Some(args) = self.embed_args.as_ref() {
            args.embed(prompt, mode, &self.cfg).await
        } else {
            EmbedArgs::default().embed(prompt, mode, &self.cfg).await
        }
        .map(|()| {
            let msg = format!("\n### SYSTEM: {msg}");
            ctx.push_turn(TurnKind::System, "MEMORIZE", msg);
        })
    }

    pub(crate) async fn query_stream(
        &mut self,
        ollama: &dyn LlmClient,
        ctx: &mut Context,
        tx: &mpsc::Sender<Result<StreamItem>>,
    ) -> Result<()> {
        let role = self.get_str("role")?.to_lowercase();
        let role = Role::from_str(role.as_str())?;

        let model = self.get_str("model")?;

        // System instructions and retrieved/untrusted content now ride as
        // distinct chat messages instead of one concatenated string — see
        // prior review note on prompt-injection surface via `documents_view`.
        let (system_text, user_text) = role.build_chat_messages(
            self.get_str("task").ok(),
            ctx,
            self.get_str("task_hint").ok(),
            self.get_str("approval_signal").ok(),
        )?;
        // Still refreshes this run's live file under `ruchat_traces/` with this turn's full
        // context/history before every query (useful when inspecting a stuck run), but doesn't
        // announce it on the visible stream — each role already gets its own colored banner
        // (`role.get_color()`, sent below), so a "[Role's input] querying 'model'..." line on
        // every single turn (every role, every round) added noise without telling the user
        // anything the banner didn't already. The model each role uses is now summarized once,
        // at the very start of the run — see `Orchestrator::run_stage_machine`.
        ctx.trace(tx, String::new()).await;
        let messages = vec![ChatMessage::system(system_text), ChatMessage::user(user_text)];

        if let Ok(msg) = self.get_str("status_msg") {
            send_event(tx, AgentEvent::StatusUpdate(msg.to_string())).await?;
        }
        // Inject the color change into the stream
        send_event(tx, AgentEvent::ColorChange(role.get_color())).await?;

        let mut stream = ollama.chat_stream(model, messages).await?;

        ctx.output.clear();
        while let Some(res) = stream.next().await {
            let chunk = res?; // already RuChatError via ChatStream's Item type
            ctx.output.push_str(&chunk.message.content);
            tx.send(Ok(StreamItem::ChatChunk(chunk)))
                .await
                .map_err(|e| RuChatError::Is(e.to_string()))?;
        }
        send_event(tx, AgentEvent::ColorChange(Role::no_color())).await?;
        Ok(())
    }
    pub(super) async fn execute_and_verify(&self, ctx: &mut Context) -> Result<Validation> {
        let call = match tools::parse_tool_call(&ctx.output) {
            Ok(call) => call,
            Err(_) => return Ok(Validation::Skip),
        };

        match call.tool {
            tools::ToolName::ApplyPatch => {
                let diff = call.args["diff"].as_str().unwrap_or_default();
                Validation::apply_patch(diff, ctx).await
            }
            tools::ToolName::ReplaceInFile => {
                let path = call.args["path"].as_str().unwrap_or_default();
                let old_string = call.args["old_string"].as_str().unwrap_or_default();
                let new_string = call.args["new_string"].as_str().unwrap_or_default();
                Validation::replace_in_file(path, old_string, new_string, ctx).await
            }
            tools::ToolName::Memorize => self
                .embed(
                    call.args["content"].as_str().unwrap_or_default(),
                    UpsertMode::Upsert,
                    ctx,
                    "Information successfully memorized.",
                )
                .await
                .map_or_else(
                    |e| Ok(Validation::Failure(e.to_string())),
                    |_| Ok(Validation::Success),
                ),
            // Retrieve/GitLog/GitBlame/GitDiff/CargoCheck/CargoClippy/etc. are dispatched
            // earlier in Stage::Implement, not here — reaching one of them at verify-time means
            // the Worker called a read-only tool a second time in the same round (its one
            // information-lookup reask was already spent) instead of acting on what it already
            // has. Give it a specific, actionable correction rather than a generic "unexpected
            // tool" dump — this is a common failure mode for local models that keep
            // "researching" instead of switching to making the change.
            other => Ok(Validation::Failure(format!(
                "refused: you called '{other:?}' again instead of applying a change. You \
                already used this round's one information-lookup, and its result is already in \
                your context above — re-running the same (or any other) read-only tool won't \
                add anything new. You must now emit exactly one apply_patch, replace_in_file, \
                or memorize tool_call to actually make the change."
            ))),
        }
    }
}
