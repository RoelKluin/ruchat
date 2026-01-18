use crate::agent::Team;
use crate::config::{load_manager, save_manager}; // We will add these
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use ollama_rs::Ollama;
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug, Clone, PartialEq)]
pub struct ManagerArgs {
    #[command(subcommand)]
    pub command: ManagerCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum ManagerCommands {
    /// Initialize a new manager config
    Init,
    /// Run the active team
    Run,
    /// List teams
    List,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Manager {
    pub teams: Vec<Team>,
    pub active_team: usize,
}

impl Manager {
    pub fn new() -> Self {
        Self {
            teams: Vec::new(),
            active_team: 0,
        }
    }

    pub fn current_team(&self) -> Result<&Team> {
        self.teams
            .get(self.active_team)
            .ok_or_else(|| anyhow!("Active team index out of bounds"))
    }

    pub fn current_team_mut(&mut self) -> Result<&mut Team> {
        self.teams
            .get_mut(self.active_team)
            .ok_or_else(|| anyhow!("Active team index out of bounds"))
    }

    pub async fn execute_command(ollama: Ollama, args: &ManagerArgs) -> Result<()> {
        let config_path = "ruchat_manager.json"; // Hardcoded for now, or move to args

        match args.command {
            ManagerCommands::Init => {
                let manager = Manager::new();
                save_manager(config_path, &manager).await?;
                println!("Initialized empty manager at {}", config_path);
            }
            ManagerCommands::Run => {
                let mut manager = load_manager(config_path).await?;
                // We pass the ollama instance down to the team -> agent
                manager.run_active(&ollama).await?;
            }
            ManagerCommands::List => {
                let manager = load_manager(config_path).await?;
                for (i, team) in manager.teams.iter().enumerate() {
                    let active = if i == manager.active_team { "*" } else { " " };
                    println!("[{}] {} - {}", active, i, team.name);
                }
            }
        }
        Ok(())
    }

    pub async fn run_active(&mut self, ollama: &Ollama) -> Result<()> {
        let team = self.current_team_mut()?;
        team.execute(ollama).await
    }
}
