use crate::Result;
use crate::agent::event::{AgentEvent, StreamItem};
use crate::agent::pipeline::PipelineStream;
use crate::io::Io;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

/// Resolves once the process is asked to stop — Ctrl-C on every platform, plus SIGTERM on Unix
/// (what a plain `kill <pid>` sends, and what a closed terminal/dropped SSH session can trigger
/// too). Both need to reach the same graceful-cancellation path as Ctrl-C below, not the OS
/// default of an unconditional, immediate kill that gives `finalize_trace` no chance to run.
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Consumes a `PipelineStream` and renders it to the terminal — shared by
/// `ask.rs`'s `pipe` command and `Manager::execute_command`'s `Run` arm, so
/// Manager's output goes through the same colored/status/trace handling
/// instead of plain `println!`.
///
/// Also races the stream against `wait_for_shutdown_signal`. Before this, Ctrl-C/SIGTERM used
/// the OS default (kill the process immediately), which never gave
/// `Orchestrator::run_task_stream`'s spawned task a chance to reach `finalize_trace` —
/// confirmed live 2026-08-05: several multi-round traces still sat unarchived even after
/// `finalize_trace` was made to run on every in-process `Err`, because these runs were never
/// returning an `Err` at all, they were being killed from outside the program. The first
/// shutdown signal now triggers `cancel` (which `run_stage_machine_loop` checks once per stage
/// transition) and keeps the stream open so the run can still archive itself; a second signal
/// force-exits immediately, since a graceful stop can still take a while if a long tool call or
/// LLM stream is already in flight.
pub(crate) async fn render_pipeline_stream(
    mut stream: PipelineStream,
    cancel: CancellationToken,
    cio: &mut Io,
) -> Result<()> {
    let mut status_line_active = false;
    let mut cancel_requested = false;
    loop {
        let res = tokio::select! {
            next = stream.next() => match next {
                Some(res) => res,
                None => break,
            },
            () = wait_for_shutdown_signal() => {
                if cancel_requested {
                    // Already asked once and the run hasn't wrapped up — honor the second
                    // signal as "stop waiting", same as the OS default would have done.
                    std::process::exit(130); // 128 + SIGINT, the conventional shell exit code
                }
                cancel_requested = true;
                cancel.cancel();
                if status_line_active {
                    cio.clear_status_line().await?;
                    status_line_active = false;
                }
                cio.write_line(
                    "\n\x1b[33m[cancelling — waiting for the run to save its trace; \
                     send the signal again to force quit]\x1b[0m\n",
                )
                .await?;
                continue;
            }
        };
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
