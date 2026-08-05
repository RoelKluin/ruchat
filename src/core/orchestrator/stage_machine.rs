use super::Orchestrator;

impl Orchestrator {
    /// One-line, once-per-run summary of which model each configured role uses. Printed a
    /// single time at the start of the run (see `run_stage_machine`) instead of repeating
    /// "querying 'model'..." on every single turn (every role, every round) — each role's own
    /// colored banner already identifies who's speaking once the run is underway, so restating
    /// the model there added noise without new information.
    pub(super) fn model_summary(&self) -> String {
        let mut parts = vec![
            format!(
                "Architect={}",
                self.architect.get_str("model").unwrap_or("?")
            ),
            format!("Worker={}", self.worker.get_str("model").unwrap_or("?")),
        ];
        if let Some(a) = self.scoper.as_ref() {
            parts.push(format!("Scoper={}", a.get_str("model").unwrap_or("?")));
        }
        if let Some(a) = self.librarian.as_ref() {
            parts.push(format!("Librarian={}", a.get_str("model").unwrap_or("?")));
        }
        if let Some(a) = self.validator.as_ref() {
            parts.push(format!("Validator={}", a.get_str("model").unwrap_or("?")));
        }
        for c in &self.critics {
            let label = c
                .get_str("name")
                .or_else(|_| c.get_str("role"))
                .unwrap_or("Critic");
            parts.push(format!("{label}={}", c.get_str("model").unwrap_or("?")));
        }
        if let Some(a) = self.summarizer.as_ref() {
            parts.push(format!("Summarizer={}", a.get_str("model").unwrap_or("?")));
        }
        format!(
            "Models: {} — full prompts logged to ruchat_traces/ as the run progresses.",
            parts.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{base_config, build_test_orchestrator};
    use serde_json::json;

    #[tokio::test]
    async fn model_summary_lists_every_configured_role_once() {
        let mut config = base_config();
        config["Validator"] = json!({ "model": "validator-model" });
        config["Critics"] = json!([{ "model": "critic-model", "name": "Security" }]);
        let orchestrator = build_test_orchestrator(config, vec![], None).await;

        let summary = orchestrator.model_summary();
        assert!(summary.contains("Architect=fake"));
        assert!(summary.contains("Worker=fake"));
        assert!(summary.contains("Validator=validator-model"));
        assert!(summary.contains("Security=critic-model"));
        assert!(summary.contains("ruchat_traces/"));
    }
}
