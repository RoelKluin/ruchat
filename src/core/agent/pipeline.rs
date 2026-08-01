use crate::agent::event::StreamItem;
use crate::agent::llm_client::LlmClient;
use crate::core::agent::Team;
use crate::core::orchestrator::Orchestrator;
use crate::Result;
use crate::RuChatError;
use ollama_rs::generation::chat::ChatMessage;
use ollama_rs::Ollama;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tokio_stream::StreamExt;

pub(crate) type PipelineStream = Pin<Box<dyn Stream<Item = Result<StreamItem>> + Send>>;

/// Unifies the two ways this crate runs a multi-agent task — the Stage-
/// machine `Orchestrator` (ask.rs's `pipe` command) and the sequential
/// `Team` (Manager's saved-team runner) — behind one `run()` entry point so
/// callers share one rendering loop instead of each hand-rolling their own
/// (Manager previously used bare `println!`, orchestrator used the
/// StreamItem/AgentEvent match in ask.rs).
pub(crate) enum AgentPipeline {
    Orchestrator {
        orchestrator: Orchestrator,
        goal: String,
        debug_sequence: Option<String>,
    },
    Team {
        team: Team,
        ollama: Ollama,
    },
    /// The non-agentic `pipe` path — no Architect/Worker config, just a bare
    /// prompt sent straight to the model. Kept as its own variant (rather
    /// than a separate method on `AskArgs`) so `ask()` renders it through
    /// the same `render_pipeline_stream` loop as the other two, instead of
    /// a second hand-rolled rendering path.
    OneShot {
        ollama: Arc<dyn LlmClient>,
        model: String,
        prompt: String,
    },
}

impl AgentPipeline {
    pub(crate) fn run(self) -> PipelineStream {
        match self {
            AgentPipeline::Orchestrator {
                orchestrator,
                goal,
                debug_sequence,
            } => Box::pin(orchestrator.run_task_stream(goal, debug_sequence)),
            AgentPipeline::Team { team, ollama } => team.run_streaming(ollama),
            AgentPipeline::OneShot {
                ollama,
                model,
                prompt,
            } => {
                let (tx, rx) = mpsc::channel(100);
                tokio::spawn(async move {
                    let result: Result<()> = async {
                        let messages = vec![ChatMessage::user(prompt)];
                        let mut stream = ollama.chat_stream(&model, messages).await?;
                        while let Some(chunk) = stream.next().await {
                            let chunk = chunk?;
                            tx.send(Ok(StreamItem::ChatChunk(chunk)))
                                .await
                                .map_err(|e| RuChatError::Is(e.to_string()))?;
                        }
                        Ok(())
                    }
                    .await;
                    if let Err(e) = result {
                        let _ = tx.send(Err(e)).await;
                    }
                });
                Box::pin(ReceiverStream::new(rx))
            }
        }
    }
}
