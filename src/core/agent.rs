pub(crate) mod manager;
pub(crate) mod protocol;
mod role;
pub(crate) mod team;
pub(crate) mod types;
pub(crate) mod worker;

use crate::core::embed::{EmbedArgs, UpsertMode};
use crate::core::orchestrator::TaskType;
use crate::providers::llm::ollama::get_dynamic_history_limit;
use crate::providers::vector::chroma::query::Query;
use crate::{Result, RuChatError, options::get_options};
use chroma::ChromaHttpClient;
use ollama_rs::generation::completion::{GenerationResponse, request::GenerationRequest};
use ollama_rs::{Ollama, models::ModelOptions};
use protocol::{Tool, ToolCall, Validation};
use role::Role;
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
pub(crate) use team::Team;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use types::Context;

pub(crate) struct Agent {
    options: ModelOptions,
    config: HashMap<String, Value>,
    pub(super) embed_args: Option<EmbedArgs>,
}

impl Agent {
    pub(crate) async fn new(
        config: &mut Value,
        role: &str,
        required: bool,
        task_type: Option<&TaskType>,
    ) -> Result<Self> {
        if let Some(agent_val) = config.get(role) {
            // Check if it's a raw JSON string (from CLI) or an Object (from json! macro)
            let options_str = if agent_val.is_string() {
                agent_val.as_str().unwrap().to_string()
            } else {
                agent_val.to_string()
            };
            let (options, mut config) = get_options(&options_str).await?;
            config.insert("role".to_string(), Value::String(role.to_string()));
            if let Some(task) = task_type {
                config.insert(
                    "task_hint".to_string(),
                    serde_json::Value::String(task.to_string()),
                );
            }

            let embed_args = config
                .remove("embed_args")
                .and_then(|v| serde_json::from_value(v).ok());

            Ok(Self {
                options,
                config,
                embed_args,
            })
        } else if required {
            Err(RuChatError::MissingAgent(role.to_string()))
        } else {
            Err(RuChatError::Is("Optional agent missing".into()))
        }
    }
    pub(crate) fn remove_str(&mut self, key: &str) -> Result<String> {
        let v = self.config
            .remove(key)
            .ok_or(RuChatError::Is(format!("No {key} to remove in agent config")))?;
        if v.is_string() {
            Ok(v.as_str().unwrap().to_string())
        } else if v.is_object() {
            Ok(v.to_string())
        } else {
            Err(RuChatError::Is(format!("Value for {key} is not a string in agent config {:?})", self.config)))
        }
    }

    pub(crate) fn get_str(&self, key: &str) -> Result<&str> {
        self.config
            .get(key)
            .ok_or(RuChatError::Is(format!("No {key} in agent config")))?
            .as_str()
            .ok_or(RuChatError::Is(format!("Value for {key} is not a string in agent config {:?})", self.config)))
    }
    pub(crate) async fn retrieve_and_generate(
        &self,
        client: &ChromaHttpClient,
        ollama: &Ollama,
        q: Query,
    ) -> Result<String> {
        let model = self.get_str("model")?;
        q.query(client, ollama, model).await
    }
    pub(crate) fn get_dynamic_history_limit(&self) -> u64 {
        get_dynamic_history_limit(self.get_str("model").unwrap_or(""))
    }

    async fn parse_tool_call(&self, ctx: &mut Context) -> Result<()> {
        if let Some(tool_call) = ToolCall::parse(&ctx.output)
            && tool_call.name.as_str() == "MEMORIZE"
        {
            self.embed(
                tool_call.content.as_str(),
                UpsertMode::Upsert,
                ctx,
                "Information successfully committed to long-term memory.",
            )
            .await?;
        }
        Ok(())
    }
    pub(super) async fn embed(
        &self,
        prompt: &str,
        mode: UpsertMode,
        ctx: &mut Context,
        msg: &str,
    ) -> Result<()> {
        if let Some(args) = self.embed_args.as_ref() {
            args.embed(prompt, mode).await
        } else {
            EmbedArgs::default().embed(prompt, mode).await
        }
        .map(|()| ctx.history.push_str(&format!("\n### SYSTEM: {msg}")))
    }

    pub(crate) async fn query_stream(
        &mut self,
        ollama: &Ollama,
        ctx: &mut Context,
        tx: &mpsc::Sender<Result<Vec<GenerationResponse>>>,
    ) -> Result<()> {
        let role = self.get_str("role")?.to_lowercase();
        let role = Role::from_str(role.as_str())?;

        // Assemble the payload
        let full_prompt = role.build_prompt(self.get_str("task").ok(), ctx, self.get_str("task_hint").ok());

        let model = self.get_str("model")?;
        ctx.trace(
            tx,
            format!("Agent '{role}' is generating with model '{model}' and prompt:\n{full_prompt}"),
        ).await;
        let request =
            GenerationRequest::new(model.to_string(), full_prompt).options(self.options.clone());

        if let Ok(msg) = self.get_str("status_msg") {
            tx.send(Err(RuChatError::StatusUpdate(msg.to_string())))
                .await
                .map_err(|e| RuChatError::Is(e.to_string()))?;
        }
        // Inject the color change into the stream
        tx.send(Err(RuChatError::ColorChange(role.get_color())))
            .await
            .map_err(|e| RuChatError::Is(e.to_string()))?;

        let mut stream = ollama
            .generate_stream(request)
            .await
            .map_err(RuChatError::OllamaError)?;
        if self.get_str("status_msg").is_ok() {
            // Clear the status message after the first chunk arrives
            tx.send(Err(RuChatError::StatusUpdate("\x1b[2K".to_string())))
                .await
                .map_err(|e| RuChatError::Is(e.to_string()))?;
        }

        ctx.output.clear();
        while let Some(res) = stream.next().await {
            let chunk = res.map_err(RuChatError::OllamaError)?;
            for resp in &chunk {
                ctx.output.push_str(&resp.response);
            }
            tx.send(Ok(chunk))
                .await
                .map_err(|e| RuChatError::Is(e.to_string()))?;
        }
        tx.send(Err(RuChatError::ColorChange(Role::no_color())))
            .await
            .map_err(|e| RuChatError::Is(e.to_string()))?;
        self.parse_tool_call(ctx).await?;
        role.update_context(ctx, self.get_str("approval_signal").unwrap_or("APPROVED"));
        Ok(())
    }
    pub(super) async fn execute_and_verify(&self, ctx: &mut Context) -> Result<Validation> {
        let tool_call = match ToolCall::parse(&ctx.output) {
            Some(call) => call,
            None => return Ok(Validation::Skip),
        };

        match tool_call.to_tool() {
            Some(Tool::Shell { command }) => Validation::execute_shell_script(&command, ctx).await,
            Some(Tool::Memorize { content }) => self
                .embed(
                    &content,
                    UpsertMode::Upsert,
                    ctx,
                    "Information successfully memorized.",
                )
                .await
                .map_or_else(
                    |e| Ok(Validation::Failure(e.to_string())),
                    |_| Ok(Validation::Success),
                ),
            None => Ok(Validation::Failure(format!(
                "Unknown tool: {}",
                tool_call.name
            ))),
        }
    }
}
