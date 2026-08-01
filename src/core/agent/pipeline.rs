use crate::agent::event::{AgentEvent, StreamItem};
use crate::Result;
use ollama_rs::Ollama;
use tokio::sync::mpsc;

/// Common execution interface for RuChat's two agent-execution models.
///
/// These are INTENTIONALLY NOT merged into one internal implementation:
/// - `Orchestrator` (`core::orchestrator`) is a full stage machine
///   (Plan → Retrieve → Implement → Test → Validate → Critique → Reconcile →
///   Commit) with retry/escalation semantics and an optional RAG Librarian.
/// - `Team` (`core::agent::team`) is a flat, sequential pipe of
///   `worker::Agent` instances with no validation, critique, or retry — a
///   deliberately simpler model for straight-line multi-step prompts that
///   don't need the stage machine's guardrails.
///
/// `ollama` is ignored by pipelines that already own their own client
/// (`Orchestrator`, constructed with one in `Orchestrator::new`); it's part
/// of the trait because `Team` does not currently own one and is
/// constructed fresh from config by `Manager` each run. This asymmetry is a
/// known wart of unifying two models with different client-lifetime
/// management, not hidden here.
#[async_trait::async_trait]
pub(crate) trait AgentPipeline {
    async fn run(&mut self, ollama: &Ollama, tx: mpsc::Sender<Result<StreamItem>>) -> Result<()>;
}
