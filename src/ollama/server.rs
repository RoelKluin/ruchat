use crate::error::{Result, RuChatError};
use clap::Parser;
use ollama_rs::Ollama;

#[derive(Parser, Debug, Default, PartialEq, Clone)]
pub struct ServerArgs {
    /// Address and port of the ollama server.
    #[arg(short, long, default_value = "http://localhost:11434")]
    pub(crate) server: String,
}

impl ServerArgs {
    /// Initializes a connection to an Ollama server.
    ///
    /// This function parses the server address and port from the provided
    /// arguments and establishes a connection to the Ollama server.
    ///
    /// # Parameters
    ///
    /// - `args`: The command-line arguments containing the server information.
    ///
    /// # Returns
    ///
    /// A `Result` containing the `Ollama` client or a `RuChatError`.
    pub(super) fn init(&self) -> Result<Ollama> {
        self.server
            .rsplit_once(':')
            .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
            .ok_or_else(|| RuChatError::ArgServerError(self.server.to_string()))
    }
}
