pub struct AgentConfig {
    pub role: String,
    pub model: String,
    pub temperature: f32,
    pub init_prompt: String,
}

pub enum TaskType {
    RustRefactor,
    GitBisect,
    VimAutomation,
    DebugCore,
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
                TaskType::VimAutomation => self.execute_vim_script(&output).await?,
                TaskType::RustRefactor => self.run_cargo_check().await?,
                _ => Validation::Skip,
            };

            // 4. Update TUI State
            self.ui.update_round(round, &plan, &output, &val_result);
            
            if self.critic.approve(&output).await? { break; }
        }
        Ok(())
    }
}
