use crate::agent::event::{AgentEvent, StreamItem};
use crate::agent::pipeline::AgentPipeline;
use crate::agent::worker::Agent;
use crate::Result;
use ollama_rs::Ollama;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Team {
    pub name: String,
    pub goal: String,
    pub agents: Vec<Agent>,
}

impl Team {
    pub fn new(name: String, goal: String, agents: Vec<Agent>) -> Self {
        Self { name, goal, agents }
    }
    pub async fn execute(&mut self, ollama: &Ollama) -> Result<()> {
        println!("Team '{}' executing goal: {}", self.name, self.goal);

        // Defaulting to sequential chain execution for now.
        // Data flow needs to be defined: Pipe output of A to input of B?
        let mut context = String::new();

        for agent in &mut self.agents {
            context = agent.process(ollama, context).await?;
        }

        println!("Final Output: {}", context);
        Ok(())
    }
}

#[async_trait::async_trait]
impl AgentPipeline for Team {
    /// Same sequential-pipe logic as `execute`, but each agent's output is
    /// emitted as an `AgentEvent::Trace` on `tx` instead of printed directly
    /// — lets a caller that already consumes `Orchestrator`'s `StreamItem`
    /// stream (e.g. `ask.rs`) consume a `Team` run through the identical
    /// interface, uniformly.
    async fn run(&mut self, ollama: &Ollama, tx: mpsc::Sender<Result<StreamItem>>) -> Result<()> {
        let _ = tx
            .send(Ok(StreamItem::Event(AgentEvent::Trace(format!(
                "Team '{}' executing goal: {}",
                self.name, self.goal
            )))))
            .await;

        let mut context = String::new();
        for agent in &mut self.agents {
            context = agent.process(ollama, context).await?;
            let _ = tx
                .send(Ok(StreamItem::Event(AgentEvent::Trace(format!(
                    "[{}] {}",
                    agent.name, context
                )))))
                .await;
        }
        Ok(())
    }
}
