use crate::error::RuChatError;
use std::io::stdin;
use tokio::io::AsyncWriteExt;

/// A struct for handling input and output operations in RuChat.
///
/// This struct provides methods for reading from standard input and
/// writing to standard output asynchronously.
pub(crate) struct Io {
    stdin: std::io::Stdin,
    stdout: tokio::io::Stdout,
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
        }
    }

    /// Reads a line from standard input.
    ///
    /// If `with_prompt` is true, a prompt is displayed before reading input.
    ///
    /// # Parameters
    ///
    /// - `with_prompt`: A boolean indicating whether to display a prompt.
    ///
    /// # Returns
    ///
    /// A `Result` containing the input line as a `String` or a `RuChatError`.
    pub(crate) async fn read_line(&mut self, with_prompt: bool) -> Result<String, RuChatError> {
        if with_prompt {
            self.stdout.write_all(b"\n> ").await?;
            self.stdout.flush().await?;
        }
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
