use crate::RuChatError;
use std::io::stdin;
use tokio::io::{AsyncWrite, AsyncWriteExt};

/// A struct for handling input and output operations in RuChat.
///
/// This struct provides methods for reading from standard input and
/// writing to standard output asynchronously.
pub(crate) struct Io {
    stdin: std::io::Stdin,
    stdout: tokio::io::Stdout,
    stderr: tokio::io::Stderr,
}

impl Io {
    /// Creates a new `Io` instance.
    ///
    /// # Returns
    ///
    /// A new instance of `Io` with standard input and output initialized.
    pub(crate) fn new() -> Self {
        Self {
            stdin: stdin(),
            stdout: tokio::io::stdout(),
            stderr: tokio::io::stderr(),
        }
    }

    /// Reads a line from standard input.
    ///
    /// # Returns
    ///
    /// A `Result` containing the input line as a `String` or a `RuChatError`.
    pub(crate) async fn read_line(&mut self) -> Result<String, RuChatError> {
        let mut input = String::new();
        self.stdin.read_line(&mut input)?;
        Ok(input.trim_end().to_string())
    }

    /// Writes a line to standard output.
    ///
    /// # Parameters
    ///
    /// - `line`: The line to write to standard output.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn write_line(&mut self, line: &str) -> Result<(), RuChatError> {
        write_flushed(&mut self.stdout, line.as_bytes()).await
    }

    /// Writes a line to standard error.
    ///
    /// # Parameters
    ///
    /// - `line`: The line to write to standard error.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn write_error_line(&mut self, line: &str) -> Result<(), RuChatError> {
        write_flushed(&mut self.stderr, line.as_bytes()).await
    }

    /// Writes a string to standard output.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to write to standard output.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn write(&mut self, s: &str) -> Result<(), RuChatError> {
        write_flushed(&mut self.stdout, s.as_bytes()).await
    }

    /// Returns the cursor to column 0 and clears to end of line — use before
    /// writing real content whenever a `\r`-based status/spinner line may
    /// still be showing. A bare `\r` alone (as previously used for status
    /// updates) does NOT clear trailing characters from a longer prior
    /// frame, which is what produces interleaved/corrupted output when a
    /// status line and streamed content share the same line.
    pub(crate) async fn clear_status_line(&mut self) -> Result<(), RuChatError> {
        write_flushed(&mut self.stdout, b"\r\x1b[2K").await
    }
}

async fn write_flushed<W: AsyncWrite + Unpin>(w: &mut W, bytes: &[u8]) -> Result<(), RuChatError> {
    w.write_all(bytes).await?;
    w.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_write_line() {
        let mut io = Io::new();
        let line = "Hello, world!";
        let result = io.write_line(line).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_write() {
        let mut io = Io::new();
        let text = "Hello, world!";
        let result = io.write(text).await;
        assert!(result.is_ok());
    }
}
