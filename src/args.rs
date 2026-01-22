use crate::error::Result;
use crate::agent::manager::{Manager, ManagerArgs};
use crate::chroma::delete::{ChromaDeleteArgs, chroma_delete};
use crate::chroma::ls::{ChromaLsArgs, chroma_ls};
use crate::chroma::query::{QueryArgs, query};
use crate::chroma::similarity::{SimilarityArgs, similarity_search};
use crate::embed::{EmbedArgs, embed};
use crate::ollama::OllamaArgs;
use crate::ollama::ask::AskArgs;
use crate::ollama::chat::{ChatArgs, chat};
use crate::ollama::func::func;
use crate::ollama::func::strukt::func_struct;
use crate::ollama::model::ls::list;
use crate::ollama::model::pull::pull;
use crate::ollama::model::rm::remove;
use crate::ollama::server::ServerArgs;
use clap::{Parser, Subcommand};

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

    /// Toggle verbose mode.
    #[arg(short, long, default_value = "false")]
    pub(crate) verbose: bool,
}

impl Args {
    pub(crate) async fn handle_request(self) -> Result<()> {
        let default = Commands::Pipe(AskArgs::default());
        if self.verbose {
            let command_line = std::env::args().collect::<Vec<String>>().join(" ");
            println!("Command line: {}", command_line);
        }
        match self.command.unwrap_or(default) {
            Commands::Ask(args) => args.ask(true).await,
            Commands::Pipe(args) => args.ask(false).await,
            Commands::Chat(args) => chat(args).await,
            Commands::Ls(args) => list(args).await,
            Commands::Rm(args) => remove(args).await,
            Commands::Pull(args) => pull(args).await,
            Commands::Func(args) => func(args).await,
            Commands::FuncStruct(args) => func_struct(args).await,
            Commands::Embed(args) => embed(args).await,
            Commands::Query(args) => query(args).await,
            Commands::Similarity(args) => similarity_search(args).await,
            Commands::ChromaLs(args) => chroma_ls(args).await,
            Commands::ChromaDelete(args) => chroma_delete(args).await,
            Commands::Manager(args) => Manager::execute_command(args).await,
        }
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
    Pipe(AskArgs),
    /// Chat with a language model.
    Chat(ChatArgs),
    /// List models.
    Ls(ServerArgs),
    /// Remove a model.
    Rm(OllamaArgs),
    /// Pull a model from a remote ollama server.
    Pull(OllamaArgs),
    /// Run a function using a language model.
    Func(OllamaArgs),
    /// Run a function using a language model with structured input.
    FuncStruct(OllamaArgs),
    /// Use embedding model to create embeddings in Chroma.
    Embed(EmbedArgs),
    /// Query Chroma database.
    Query(QueryArgs),
    /// Find similar embeddings in Chroma database.
    Similarity(SimilarityArgs),
    /// List Chroma database collections.
    ChromaLs(ChromaLsArgs),
    /// Delete Chroma database collections or entries.
    ChromaDelete(ChromaDeleteArgs),
    /// Manage agents.
    Manager(ManagerArgs),
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
