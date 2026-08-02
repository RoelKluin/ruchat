mod cli;
mod core;
mod providers;
mod tui;
mod utils;

pub use crate::utils::error::{Result, RuChatError};

/// Retries an async expression with exponential backoff (250ms, 500ms,
/// 1000ms) on transient errors only, up to `utils::retry::MAX_ATTEMPTS`.
///
/// Implemented as a macro rather than a generic higher-order function
/// because call sites capture multiple distinct `&mut` borrows at once
/// (e.g. `&mut self.architect`, `ctx`) — a stored `FnMut` closure re-borrowing
/// those across loop iterations runs into exactly the kind of borrow-checker
/// friction this crate already documents elsewhere (see the `Orchestrator`
/// borrow-ordering notes). The macro instead re-evaluates the expression
/// textually on each attempt, producing a fresh borrow — and a fresh
/// `Future` — per iteration, which sidesteps the issue entirely.
#[macro_export]
macro_rules! retry_transient {
    ($e:expr) => {{
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            match $e.await {
                Ok(v) => break Ok(v),
                Err(e) if attempt < $crate::utils::retry::MAX_ATTEMPTS
                    && $crate::utils::retry::is_transient(&e) =>
                {
                    tracing::warn!(attempt, error = %e, "transient error, retrying");
                    tokio::time::sleep($crate::utils::retry::backoff_delay(attempt)).await;
                }
                Err(e) => break Err(e),
            }
        }
    }};
}

use args::Args;
use clap::Parser;
pub(crate) use cli::{args, options, serde};
pub(crate) use core::{agent, orchestrator};
pub(crate) use providers::llm::ollama;
pub(crate) use providers::vector::chroma;
pub(crate) use tui::io;

/// Runs the RuChat application.
///
/// This function initializes the environment logger, parses command-line
/// arguments, and handles the request based on the provided arguments.
///
/// # Returns
///
/// This function returns a `Result` indicating success or failure. On success,
/// it returns `Ok(())`. On failure, it returns an `Err` containing a `RuChatError`.
///
/// # Errors
///
/// This function will return an error if the command-line arguments cannot be
/// parsed or if handling the request fails.
pub async fn run() -> Result<()> {
    let args = Args::parse();
    args.handle_request().await
}
