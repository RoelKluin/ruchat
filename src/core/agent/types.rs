use crate::{RuChatError, Result};
use ollama_rs::generation::completion::GenerationResponse;
use tokio::sync::mpsc;

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
    pub(crate) async fn trace(&mut self, tx: &mpsc::Sender<Result<Vec<GenerationResponse>>>, err: String) {
        if !err.is_empty() {
            self.rejections.push_str(&format!("\n{err}"));
            tx.send(Err(RuChatError::Trace(err))).await.ok();
        }
        let trace_output = format!(
            "# Orchestration Trace\n\n## Goal\n{}\n\n## Context\n{}\n\n## History\n{}\n\n## Rejections\n{}",
            self.get_goal(), self.context, self.history, self.rejections
        );
        let _ = tokio::fs::write(".ruchat_trace.md", trace_output).await;
    }
}
