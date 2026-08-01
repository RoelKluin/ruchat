use crate::agent::event::{send_event, AgentEvent, StreamItem};
use crate::agent::pipeline::PipelineStream;
use crate::agent::worker::Agent;
use crate::Result;
use ollama_rs::Ollama;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

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
    /// Retained for any existing non-streaming callers/tests; delegates to
    /// `run_streaming` internally so the two don't drift.
    pub async fn execute(&mut self, ollama: &Ollama) -> Result<()> {
        let (tx, _rx) = mpsc::channel(100);
        self.execute_streaming(ollama, &tx).await
    }

    /// Consumes the team and streams `AgentEvent::Trace` events for each
    /// step instead of `println!`-ing directly, so CLI rendering (colors,
    /// status lines, trace-file writing) is handled uniformly by whatever
    /// consumes the stream — see `render_pipeline_stream` in `tui/render.rs`.
    pub(crate) fn run_streaming(mut self, ollama: Ollama) -> PipelineStream {
        let (tx, rx) = mpsc::channel(100);
        tokio::spawn(async move {
            let result = self.execute_streaming(&ollama, &tx).await;
            if let Err(e) = result {
                let _ = tx.send(Err(e)).await;
            }
        });
        Box::pin(ReceiverStream::new(rx))
    }

    async fn execute_streaming(
        &mut self,
        ollama: &Ollama,
        tx: &mpsc::Sender<Result<StreamItem>>,
    ) -> Result<()> {
        send_event(
            tx,
            AgentEvent::Trace(format!(
                "Team '{}' executing goal: {}",
                self.name, self.goal
            )),
        )
        .await?;

        let mut context = String::new();
        for agent in &mut self.agents {
            context = agent.process(ollama, context).await?;
            send_event(
                tx,
                AgentEvent::Trace(format!("[{}] {}", agent.name, context)),
            )
            .await?;
        }

        send_event(tx, AgentEvent::Trace(format!("Final Output: {context}"))).await?;
        Ok(())
    }
}
