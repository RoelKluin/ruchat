pub(crate) mod manager;
pub(crate) mod team;
pub(crate) mod worker;

pub(crate) use team::Team;
use ollama_rs::{Ollama, models::ModelOptions};
use ollama_rs::generation::completion::{GenerationResponse, request::GenerationRequest};
use std::collections::HashMap;
use crate::{Result, RuChatError, options::get_options};
use serde_json::Value;
use tokio_stream::StreamExt;
use tokio::sync::mpsc;
use crate::providers::vector::chroma::query::Query;
use chroma::ChromaHttpClient;

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

pub(crate) struct Context {
    goal: String,
    pub(crate) history: String,
    pub(crate) output: String,
    pub(crate) context: String,
    pub(crate) rejections: String,
    pub(crate) documents: String,
}

impl Context {
    pub(crate) fn new(goal: String) -> Self {
        Self {
            goal: format!("Goal: {goal}\n\n"),
            history: String::new(),
            output: String::new(),
            context: String::new(),
            rejections: String::new(),
            documents: String::new(),
        }
    }
    pub(crate) fn get_goal(&self) -> &str {
        &self.goal
    }
    pub(crate) fn is_approved(&self) -> bool {
        self.rejections.is_empty()
    }
}

pub struct Agent {
    options: ModelOptions,
    config: HashMap<String, Value>,
}

impl Agent {
    pub(crate) async fn new(role: &str, options: &str) -> Result<Self> {
        let (options, mut config) = get_options(options).await?;
        config.insert("role".to_string(), Value::String(role.to_string()));

        Ok(Self { options, config })
    }
    pub(crate) fn remove_str(&mut self, key: &str) -> Result<String> {
        self.config.remove(key).and_then(|s| s.as_str().map(|s| s.to_string())).ok_or(RuChatError::Is(format!("No {key} in agent config")))
    }

    pub(crate) fn get_str(&self, key: &str) -> Result<&str> {
        self.config.get(key).and_then(|s| s.as_str()).ok_or(RuChatError::Is(format!("No {key} in agent config")))
    }
    pub (crate) async fn retrieve_and_generate(&self, client: &ChromaHttpClient, ollama: &Ollama, query: &str) -> Result<String> {
        let model = self.get_str("model")?;
        let q: Query = serde_json::from_str(query).map_err(RuChatError::SerdeError)?;
        q.query(client, ollama, model).await
    }
    fn build_prompt_by_role(&self, role: &str, system: &str, ctx: &Context) -> String {
        match role {
            "ARCHITECT" => format!(
                "{system}\nGOAL: {}\nHISTORY: {}\nTASK: Plan implementation.",
                ctx.get_goal(), ctx.history
            ),
            "WORKER" => format!(
                "{system}\nDOCUMENTS: {}\nPLAN: {}\nGOAL: {}",
                ctx.documents, ctx.context, ctx.get_goal()
            ),
            "SUMMARIZER" => format!(
                "{system}\nRAW HISTORY TO COMPRESS: {}",
                ctx.history
            ),
            _ => format!(
                "{system}\nGOAL: {}\nCODE/WORK TO REVIEW: {}",
                ctx.get_goal(), ctx.context // context is worker implementation
            ),
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
