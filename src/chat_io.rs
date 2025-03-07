use crate::error::RuChatError;
use std::io::stdin;
use tokio::io::AsyncWriteExt;

pub(crate) struct ChatIO {
    stdin: std::io::Stdin,
    stdout: tokio::io::Stdout,
}

impl ChatIO {
    pub(crate) fn new() -> Self {
        Self {
            stdin: stdin(),
            stdout: tokio::io::stdout(),
        }
    }

    pub(crate) async fn read_line(&mut self) -> Result<String, RuChatError> {
        self.stdout.write_all(b"\n> ").await?;
        self.stdout.flush().await?;

        let mut input = String::new();
        self.stdin.read_line(&mut input)?;
        Ok(input.trim_end().to_string())
    }

    pub(crate) async fn write_line(&mut self, line: &str) -> Result<(), RuChatError> {
        self.stdout.write_all(line.as_bytes()).await?;
        self.stdout.flush().await?;
        Ok(())
    }
}
