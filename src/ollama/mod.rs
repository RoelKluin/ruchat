pub(crate) mod ask;
pub(crate) mod pipe;
pub(crate) mod model;
pub(crate) mod func;
pub(crate) mod chat;
use crate::error::RuChatError;
use crate::args::Args;
use ollama_rs::Ollama;

pub(crate) fn init(args: &Args) -> Result<Ollama, RuChatError> {
    let server = &args.server;
    if args.verbose {
        println!("Connecting to Ollama server at {}", server);
    }
    server
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
        .ok_or_else(|| RuChatError::ArgServerError(server.to_string()))
}


