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
        "performance critic"=> "\x1b[1;94m",
        "summarizer"   => "\x1b[1;35m",
        _           => "\x1b[0m",
    }
}
const NC: &str = "\x1b[0m";

pub(crate) struct Context {
    goal: String,
    pub(crate) history: String,
    pub(crate) output: String,
    pub(crate) context: String,
}

impl Context {
    pub(crate) fn new(goal: String) -> Self {
        Self {
            goal: format!("Goal: {goal}\n\n"),
            history: String::new(),
            output: String::new(),
            context: String::new(),
        }
    }
    pub(crate) fn get_goal(&self) -> &str {
        &self.goal
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

        let task = config.get_mut("task").ok_or(RuChatError::Is("Missing task description in agent config".to_string()))?;
        let task_str = task.as_str().ok_or(RuChatError::Is("Task description must be a string".to_string()))?.to_string();
        *task = Value::String(format!("SYSTEM: You are the {role} agent. TASK: {task_str}.\n\n"));

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
            "Validator" => {
                context.history.push_str(&format!("### Validation Report\n{color}{output}{NC}\n\n"));
                if output.contains("PASSED") {
                    true
                } else {
                    context.context = format!("VALIDATION FAILURE:\n{output}");
                    true
                }
            }
            "Critic" | "Performance Critic" | "Safety Critic" => {
                context.history.push_str(&format!("### {} Review\n{color}{output}{NC}\n\n", role.to_uppercase()));
                if output.contains("APPROVED") {
                    false // Signal to stop if approved
                } else {
                    context.context = format!("CRITIC REJECTION: {output}");
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
        &mut self,
        ollama: &Ollama,
        round: u64,
        ctx: &Context,
    ) -> Result<GenerationResponseStream> {
        let role = self.get_str("role")?.to_lowercase();

        // Define Dense Signal Instructions (Bash: DENSE_SIGNAL)
        let dense_signal = "Instruction: Use Delimiters (###) for sections. Avoid pleasantries. If providing code, provide ONLY code.";

        // Determine if we use INIT or REINIT (Bash: OPTS[role_init] = role_reinit)
        let system_instruction = if round == 1 {
            format!("System: You are the {} agent. {}. {}",
                role.to_uppercase(),
                self.get_str("task")?,
                dense_signal)
        } else {
            format!("System: Continue your role as {}. Focus on high-signal density.", role.to_uppercase())
        };

        // Assemble the payload (Bash: ARCHITECT_INIT + Context + Task)
        let full_prompt = match role.as_str() {
            "architect" => format!(
                "{}\nGOAL: {}\n\nHISTORY:\n{}\n\nTASK: {}",
                system_instruction, ctx.get_goal(), ctx.history, ctx.context
            ),
            "worker" => format!(
                "{}\nPLAN TO EXECUTE:\n{}\n\nHISTORY:\n{}",
                system_instruction, ctx.context, ctx.history
            ),
            "critic" | "safety critic" | "performance critic" => format!(
                "{}\nGOAL: {}\n\nWORKER OUTPUT TO REVIEW:\n{}",
                system_instruction, ctx.get_goal(), ctx.context
            ),
            _ => format!("{}\nCONTEXT:\n{}", system_instruction, ctx.context),
        };

        let model = self.get_str("model")?;
        let request = GenerationRequest::new(model.to_string(), full_prompt)
            .options(self.options.clone());

        ollama.generate_stream(request).await.map_err(RuChatError::OllamaError)
    }
}
