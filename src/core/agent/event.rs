use crate::{Result, RuChatError};
use ollama_rs::generation::completion::GenerationResponse;
use tokio::sync::mpsc;

/// Out-of-band signals sent alongside generation output on the agent stream.
/// These are UI/telemetry signals, never treated as failures by consumers —
/// genuine errors stay on `Result::Err` of `StreamItem`, not encoded here.
#[derive(Debug, Clone)]
pub(crate) enum AgentEvent {
    /// ANSI color code to prefix subsequent output with (or reset via `Role::no_color()`).
    ColorChange(&'static str),
    /// Transient status line (e.g. "... thinking"), cleared on first real chunk.
    StatusUpdate(String),
    /// Persistent trace/debug message, written to `.ruchat_trace.md` and optionally
    /// surfaced to the user.
    Trace(String),
    /// Coarse completion progress in percent, `[0.0, 100.0]`. Not yet emitted by
    /// any caller — reserved for future use (e.g. multi-stage progress bars).
    Progress(f32),
}

/// A single item on the agent output stream: either generated content or an
/// out-of-band event.
#[derive(Debug)]
pub(crate) enum StreamItem {
    Chunk(Vec<GenerationResponse>),
    Event(AgentEvent),
}

/// Sends an `AgentEvent` on the stream, mapping the channel-closed case to
/// `RuChatError::Is` the same way existing call sites did.
pub(crate) async fn send_event(
    tx: &mpsc::Sender<Result<StreamItem>>,
    event: AgentEvent,
) -> Result<()> {
    tx.send(Ok(StreamItem::Event(event)))
        .await
        .map_err(|e| RuChatError::Is(e.to_string()))
}
