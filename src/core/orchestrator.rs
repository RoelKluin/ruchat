use crate::agent::Agent;
use crate::ui::orchestrator_ui::OrchestratorUI;
use anyhow::Result;
use std::process::Command;

pub(crate) struct AgentConfig {
    pub role: String,
    pub model: String,
    pub temperature: f32,
    pub init_prompt: String,
}

pub(crate) enum TaskType {
    RustRefactor,
    GitBisect,
    ShellAutomation,
    DebugCore,
}

pub(crate) enum Validation {
    Success,
    Failure(String),
    Skip,
}

pub(crate) struct Orchestrator {
    pub architect: Agent,
    pub worker: Agent,
    pub critic: Agent,
    pub current_context: String,
    pub iterations: usize,
    pub ui: OrchestratorUI,
}

impl Orchestrator {
    pub async fn run_loop(&mut self, task: TaskType) -> Result<()> {
        for round in 1..=self.iterations {
            // 1. Architect Phase
            let plan = self.architect.query(&self.current_context).await?;
            
            // 2. Worker Phase
            let output = self.worker.query(&plan).await?;
            
            // 3. Validation Logic (Vim/Compiler/Cargo)
            let val_result = match task {
                TaskType::ShellAutomation => self.execute_shell_script(&output).await?,
                TaskType::RustRefactor => self.run_cargo_check().await?,
                _ => Validation::Skip,
            };

            // 4. Update UI State
            self.ui.update_round(round, &plan, &output, &val_result);
            
            if self.critic.approve(&output).await? { break; }
        }
        Ok(())
    }
    async fn execute_shell_script(&self, script: &str) -> Result<Validation> {
        // Logic to run sed and awk script and capture output
        let mut stderr = String::new();
        match Command::new("bash")
            .arg("-c")
            .arg(script)
            .output() {
                Ok(output) if output.status.success() => Ok(Validation::Success),
                Ok(output) => {
                    Ok(Validation::Failure(String::from_utf8_lossy(&output.stderr)))
                }
                Err(e) => {
                    Ok(Validation::Failure(format!("Failed to execute sed/awk: {e}")))
                }
            }
        }
    }
    async fn run_cargo_check(&self) -> Result<Validation> {
        let output = Command::new("cargo")
            .args(["check"])
            .output()
            .expect("failed to execute cargo check");

        if output.status.success() {
            Ok(Validation::Success)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Ok(Validation::Failure(stderr))
        }
    }
}
