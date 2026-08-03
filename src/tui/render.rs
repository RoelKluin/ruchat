use crate::agent::event::{AgentEvent, StreamItem};
use crate::agent::pipeline::PipelineStream;
use crate::io::Io;
use crate::Result;
use tokio_stream::StreamExt;

/// Consumes a `PipelineStream` and renders it to the terminal — shared by
/// `ask.rs`'s `pipe` command and `Manager::execute_command`'s `Run` arm, so
/// Manager's output goes through the same colored/status/trace handling
/// instead of plain `println!`.
pub(crate) async fn render_pipeline_stream(mut stream: PipelineStream, cio: &mut Io) -> Result<()> {
    let mut status_line_active = false;
    while let Some(res) = stream.next().await {
        match res {
            Ok(StreamItem::Chunk(responses)) => {
                if status_line_active {
                    cio.clear_status_line().await?;
                    status_line_active = false;
                }
                for resp in responses {
                    cio.write_line(&resp.response).await?;
                }
            }
            Ok(StreamItem::ChatChunk(chunk)) => {
                if status_line_active {
                    cio.clear_status_line().await?;
                    status_line_active = false;
                }
                cio.write_line(&chunk.message.content).await?;
            }
            Ok(StreamItem::Event(AgentEvent::ColorChange(ansi_code))) => {
                // Leading newline, deliberately: `ansi_code` embeds the next role's banner
                // text (e.g. "\x1b[1;34m[Worker]:\n" — see `Role::get_color()`), but carries no
                // newline of its own *before* it. A role's streamed content very often doesn't
                // end in a trailing newline (e.g. a fenced code block's closing "```"), so
                // without this the next role's banner glues directly onto the end of the
                // previous one's last line (`` ```[Architect]: ``). Writing this unconditionally
                // guarantees every banner starts on its own fresh line regardless of what was
                // printed just before it, including the plain reset code (`Role::no_color()`)
                // sent at the end of every turn.
                cio.write_line(&format!("\n{ansi_code}")).await?;
            }
            Ok(StreamItem::Event(AgentEvent::StatusUpdate(msg))) => {
                cio.clear_status_line().await?;
                cio.write_line(&format!("\x1b[2m   ... {msg} \x1b[0m\r\x1b[2K"))
                    .await?;
                status_line_active = !msg.is_empty();
            }
            Ok(StreamItem::Event(AgentEvent::Trace(msg))) => {
                if status_line_active {
                    cio.clear_status_line().await?;
                    status_line_active = false;
                }
                cio.write_line(&format!("\n\x1b[90m[TRACE] {msg}\x1b[0m\n"))
                    .await?;
            }
            Ok(StreamItem::Event(AgentEvent::Progress(pct))) => {
                cio.clear_status_line().await?;
                cio.write_line(&format!("\x1b[2m   ... {pct:.0}% \x1b[0m\r\x1b[2K"))
                    .await?;
                status_line_active = true;
            }
            Ok(StreamItem::Event(AgentEvent::Done)) => break,
            Err(e) => return Err(e),
        }
    }
    cio.write_line("\x1b[0m").await?;
    Ok(())
}
