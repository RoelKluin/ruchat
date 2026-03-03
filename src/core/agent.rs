pub(crate) mod manager;
pub(crate) mod team;
pub(crate) mod worker;

pub(crate) use team::Team;
use ollama_rs::models::ModelOptions;
use ollama_rs::generation::completion::GenerationResponseStream;
use ollama_rs::{Ollama, generation::completion::request::GenerationRequest};
use std::collections::HashMap;
use crate::{Result, RuChatError};
use crate::options::get_options;
use serde_json::Value;

pub(crate) fn get_agent_color(role: &str) -> &str {
    match role.to_lowercase().as_str() {
        "architect" => "\x1b[1;32m",
        "worker"    => "\x1b[1;34m",
        "validator" => "\x1b[1;33m",
        "critic"    => "\x1b[1;31m",
        "summary"   => "\x1b[1;35m",
        "performance"=> "\x1b[1;94m",
        _           => "\x1b[0m",
    }
}
const NC: &str = "\x1b[0m";

pub(crate) struct Context {
    pub(crate) history: String,
    pub(crate) output: String,
    pub(crate) context: String,
}

impl Context {
    pub(crate) fn new(goal: String) -> Self {
        Self {
            history: String::new(),
            output: String::new(),
            context: format!("Goal: {goal}\n\n"),
        }
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
        let system = config.get_mut("system").ok_or(RuChatError::Is("Missing system prompt".to_string()))?;
        let system_str = system.as_str().ok_or(RuChatError::Is("System prompt must be a string".to_string()))?;
        *system = Value::String(format!("SYSTEM: You are the {role}. {system_str}.\n\n"));
        Ok(Self { options, config })
    }
    pub(crate) fn update(&self, context: &mut Context) -> bool {
        let role = self.get_str("role").unwrap_or("unknown");
        let output = context.output.clone();
        let color = get_agent_color(role);

        match role {
            "Architect" => {
                // The Architect's output becomes the plan for the Worker
                context.history.push_str(&format!("### Architect Plan\n{color}{output}{NC}\n\n"));
                context.context = format!("PLAN:\n{output}");
                true
            }
            "Worker" => {
                // The Worker's output is what the Critic reviews
                context.history.push_str(&format!("### Worker Implementation\n{color}{output}{NC}\n\n"));
                context.context = format!("IMPLEMENTATION TO REVIEW:\n{output}");
                true
            }
            "Critic" => {
                context.history.push_str(&format!("### Critic Feedback\n{color}{output}{NC}\n\n"));
                if output.contains("APPROVED") {
                    false // Stop the loop
                } else {
                    // Feedback loop: pass critique back as context
                    context.context = format!("CRITIQUE RECEIVED:\n{output}\nPlease refine the previous work.");
                    true
                }
            }
            _ => true,
        }
    }
    pub(crate) fn get_str(&self, key: &str) -> Result<&str> {
        self.config.get(key).and_then(|s| s.as_str()).ok_or(RuChatError::Is(format!("No {key} in agent config")))
    }

    pub(crate) async fn query_stream(
        &self,
        ollama: &Ollama,
        context: &Context,
    ) -> Result<GenerationResponseStream> {
        let full_prompt = format!("{}CONTEXT:\n{}", self.get_str("system")?, context.context.as_str());
        let request = GenerationRequest::new(self.get_str("model")?.to_string(), full_prompt)
            .options(self.options.clone());

        ollama.generate_stream(request).await
            .map_err(RuChatError::OllamaError)
    }
}
