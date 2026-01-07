use crate::chroma::ls::{chroma_ls, ChromaLsArgs};
use crate::chroma::query::{query, QueryArgs};
use crate::chroma::similarity::{similarity_search, SimilarityArgs};
use crate::embed::{embed, EmbedArgs};
use crate::ollama::ask::{ask, AskArgs};
use crate::ollama::chat::{chat, ChatArgs};
use crate::ollama::func::strukt::{func_struct, FuncStructArgs};
use crate::ollama::func::{func, FuncArgs};
use crate::ollama::model::ls::list;
use crate::ollama::model::pull::{pull, PullArgs};
use crate::ollama::model::rm::{remove, RmArgs};
use crate::ollama::pipe::{pipe, PipeArgs};
use crate::RuChatError;
use clap::{Parser, Subcommand};
use ollama_rs::Ollama;

/// Main command line interface for RuChat.
///
/// This struct defines the command-line arguments and options available
/// for the RuChat application. It uses the `clap` crate to parse and
/// handle command-line input.
#[derive(Parser, Debug, PartialEq)]
pub struct Args {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Address and port of the ollama server.
    #[arg(short, long, default_value = "http://localhost:11434")]
    pub(crate) server: String,

    /// Toggle verbose mode.
    #[arg(short, long, default_value = "false")]
    pub(crate) verbose: bool,
}

impl Args {
    pub(crate) async fn handle_request(&self) -> Result<(), RuChatError> {
        let default = Commands::Pipe(PipeArgs::default());
        if self.verbose {
            let command_line = std::env::args().collect::<Vec<String>>().join(" ");
            println!("Command line: {}", command_line);
        }
        match self.command.as_ref().unwrap_or(&default) {
            Commands::Ask(ask_args) => ask(self.init()?, ask_args).await?,
            Commands::Pipe(pipe_args) => pipe(self.init()?, pipe_args).await?,
            Commands::Chat(chat_args) => chat(self.init()?, chat_args).await?,
            Commands::Ls => list(self.init()?).await?,
            Commands::Rm(rm_args) => remove(self, rm_args).await?,
            Commands::Pull(pull_args) => pull(self, pull_args).await?,
            Commands::Func(func_args) => func(self.init()?, func_args).await?,
            Commands::FuncStruct(func_args) => func_struct(self.init()?, func_args).await?,
            Commands::Embed(embed_args) => embed(self.init()?, embed_args).await?,
            Commands::Query(query_args) => query(self.init()?, query_args).await?,
            Commands::Similarity(similarity_args) => similarity_search(similarity_args).await?,
            Commands::ChromaLs(chroma_ls_args) => chroma_ls(chroma_ls_args).await?,
        }
        Ok(())
    }
    fn init(&self) -> Result<Ollama, RuChatError> {
        if self.verbose {
            println!("Connecting to Ollama server at {}", self.server);
        }
        self.server
            .rsplit_once(':')
            .and_then(|(host, port)| port.parse::<u16>().map(|p| Ollama::new(host, p)).ok())
            .ok_or_else(|| RuChatError::ArgServerError(self.server.to_string()))
    }
}

/// Subcommands for RuChat.
///
/// This enum defines the various subcommands that can be executed
/// by the RuChat application. Each variant corresponds to a specific
/// operation or functionality.
#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum Commands {
    /// Query language model using a prompt, you may include file context.
    Ask(AskArgs),
    /// Pipe markdown to language model separated by three hyphens/dashes, asterisks, or underscores.
    Pipe(PipeArgs),
    /// Chat with a language model.
    Chat(ChatArgs),
    /// List models.
    Ls,
    /// Remove a model.
    Rm(RmArgs),
    /// Pull a model from a remote ollama server.
    Pull(PullArgs),
    /// Run a function using a language model.
    Func(FuncArgs),
    /// Run a function using a language model with structured input.
    FuncStruct(FuncStructArgs),
    /// Use embedding model to create embeddings in Chroma.
    Embed(EmbedArgs),
    /// Query Chroma database.
    Query(QueryArgs),
    /// Find similar embeddings in Chroma database.
    Similarity(SimilarityArgs),
    /// List Chroma database collections.
    ChromaLs(ChromaLsArgs),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_args_parsing() {
        let args = Args::parse_from(&["test", "--server", "http://localhost:8080", "--verbose"]);
        assert_eq!(args.server, "http://localhost:8080");
        assert!(args.verbose);
    }

    #[test]
    fn test_subcommand_parsing() {
        let args = Args::parse_from(&["test", "ask"]);
        match args.command {
            Some(Commands::Ask(_)) => assert!(true),
            _ => assert!(false, "Expected Ask subcommand"),
        }
    }
    #[tokio::test]
    async fn test_handle_request_default() {
        let args = Args::parse_from(&["test", "-h"]);
        let result = args.handle_request().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_request_ask() {
        let args = Args::parse_from(&["test", "ls"]);
        let result = args.handle_request().await;
        assert!(result.is_ok());
    }
}
