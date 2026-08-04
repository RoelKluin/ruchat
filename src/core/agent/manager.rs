use crate::agent::llm_client::LlmClient;
use crate::agent::pipeline::AgentPipeline;
use crate::agent::Team;
use crate::io::Io;
use crate::ollama::ServerArgs;
use crate::orchestrator::Orchestrator;
use crate::serde::{load_manager, save_manager};
use crate::tui::render::render_pipeline_stream;
use crate::{Result, RuChatError};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ManagerArgs {
    /// Optional path to manager config file
    #[arg(short, long)]
    pub path: Option<String>,

    #[command(subcommand)]
    pub command: ManagerCommands,

    #[command(flatten)]
    server_args: ServerArgs,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub(crate) enum ManagerCommands {
    /// Initialize a new manager config
    Init,
    /// Run the active team through the Orchestrator stage machine
    Run,
    /// List teams
    List,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct Manager {
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
            .ok_or(RuChatError::ActiveTeamIndexOutOfBounds)
    }

    pub async fn execute_command(args: ManagerArgs, cfg: &Value) -> Result<()> {
        let config_path = args
            .path
            .clone()
            .unwrap_or_else(|| "ruchat_manager.json".into());

        match args.command {
            ManagerCommands::Init => {
                let name = "Default Team".to_string();
                let goal = "Achieve the default goal.".to_string();
                // A minimal Architect+Worker preset — same shape as `ask --team-model`.
                let config = serde_json::json!({
                    "Architect": { "model": "qwen2.5-coder:7b" },
                    "Worker": { "model": "qwen2.5-coder:7b" },
                });
                let mut manager = Manager::new();
                manager.teams.push(Team::new(name, goal, config));
                save_manager(config_path.as_str(), &manager).await?;
                println!("Initialized empty manager at {}", config_path);
            }
            ManagerCommands::Run => {
                let manager = load_manager(config_path.as_str()).await?;
                let team = manager.current_team()?;
                let ollama: Arc<dyn LlmClient> = Arc::new(args.server_args.init()?);
                // `ruchat manager`/Team presets have no provider-selection field yet (unlike
                // `ask`'s `--chat-provider`) — both chat and embed stay on the same Ollama
                // client, preserving current behavior exactly. Deferred, not attempted here.
                let orchestrator =
                    Orchestrator::new(team.config.clone(), ollama.clone(), ollama, cfg).await?;
                let pipeline = AgentPipeline::Orchestrator {
                    orchestrator,
                    goal: team.goal.clone(),
                    debug_sequence: None,
                    breakpoints: Default::default(),
                    resume: false,
                    approve_commit: false,
                };
                let mut cio = Io::new();
                render_pipeline_stream(pipeline.run(), &mut cio).await?;
            }
            ManagerCommands::List => {
                let manager = load_manager(config_path.as_str()).await?;
                for (i, team) in manager.teams.iter().enumerate() {
                    let active = if i == manager.active_team { "*" } else { " " };
                    println!("[{}] {} - {}", active, i, team.name);
                }
            }
        }
        Ok(())
    }
}
