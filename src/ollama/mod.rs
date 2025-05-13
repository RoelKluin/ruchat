pub(crate) mod ask;
pub(crate) mod chat;
pub(crate) mod func;
pub(crate) mod model;
pub(crate) mod pipe;
use crate::args::Args;
use crate::error::RuChatError;
use ollama_rs::Ollama;

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
pub(crate) fn init(args: &Args) -> Result<Ollama, RuChatError> {
    if args.verbose {
        println!("Connecting to Ollama server at {}", args.server);
    }
    args.server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| RuChatError::ArgServerError(args.server.to_string()))
}
