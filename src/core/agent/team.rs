use crate::agent::worker::Agent;
use crate::Result;
use ollama_rs::Ollama;
use serde::{Deserialize, Serialize};

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
