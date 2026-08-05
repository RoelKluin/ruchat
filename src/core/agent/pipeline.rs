use crate::Result;
use crate::RuChatError;
use crate::agent::event::StreamItem;
use crate::agent::llm_client::LlmClient;
use crate::core::orchestrator::{DebugBreakpoints, Orchestrator, RunTaskOptions};
use ollama_rs::generation::chat::ChatMessage;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

pub(crate) type PipelineStream = Pin<Box<dyn Stream<Item = Result<StreamItem>> + Send>>;

/// Unifies the ways this crate runs a task — the Stage-machine `Orchestrator`
/// (used by both `ask` and `manager run`, the latter loading its config from
/// a saved `Team` preset) and the non-agentic one-shot `pipe` — behind one
/// `run()` entry point so callers share one rendering loop instead of each
/// hand-rolling their own.
pub(crate) enum AgentPipeline {
    Orchestrator {
        orchestrator: Orchestrator,
        goal: String,
        debug_sequence: Option<String>,
        breakpoints: DebugBreakpoints,
        resume: bool,
        approve_commit: bool,
        trace_timings: bool,
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
    /// Also returns a `CancellationToken` the caller can trigger (a Ctrl-C handler in
    /// `render_pipeline_stream`) to ask a running `Orchestrator` to stop at its next safe
    /// checkpoint instead of the OS killing the process outright — see
    /// `Orchestrator::run_task_stream`'s doc comment for why that distinction matters. The
    /// `OneShot` path returns a token too, for a uniform return type, but nothing ever checks
    /// it: a bare prompt has no trace to save, so there's nothing a graceful stop would buy it
    /// over Ctrl-C just ending the process as before.
    pub(crate) fn run(self) -> (PipelineStream, CancellationToken) {
        match self {
            AgentPipeline::Orchestrator {
                orchestrator,
                goal,
                debug_sequence,
                breakpoints,
                resume,
                approve_commit,
                trace_timings,
            } => {
                let cancel = CancellationToken::new();
                let options = RunTaskOptions {
                    debug_sequence,
                    breakpoints,
                    resume,
                    approve_commit,
                    trace_timings,
                };
                let stream = Box::pin(orchestrator.run_task_stream(goal, options, cancel.clone()));
                (stream, cancel)
            }
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
                (Box::pin(ReceiverStream::new(rx)), CancellationToken::new())
            }
        }
    }
}
