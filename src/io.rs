use crate::error::RuChatError;
use std::io::stdin;
use tokio::io::AsyncWriteExt;

/// A struct for handling input and output operations in RuChat.
///
/// This struct provides methods for reading from standard input and
/// writing to standard output asynchronously.
pub(super) struct Io {
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
        self.stdout.write_all(line.as_bytes()).await?;
        self.stdout.flush().await?;
        Ok(())
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
        self.stderr.write_all(line.as_bytes()).await?;
        self.stderr.flush().await?;
        Ok(())
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
        self.stdout.write_all(s.as_bytes()).await?;
        self.stdout.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

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
