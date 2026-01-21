use crate::agent::manager::{Manager, ManagerArgs};
use crate::chroma::delete::{chroma_delete, ChromaDeleteArgs};
use crate::chroma::ls::{chroma_ls, ChromaLsArgs};
use crate::chroma::query::{query, QueryArgs};
use crate::chroma::similarity::{similarity_search, SimilarityArgs};
use crate::embed::{embed, EmbedArgs};
use crate::ollama::ask::{ask, AskArgs};
use crate::ollama::chat::{chat, ChatArgs};
use crate::ollama::func::func;
use crate::ollama::func::strukt::func_struct;
use crate::ollama::model::ls::list;
use crate::ollama::model::pull::pull;
use crate::ollama::model::rm::remove;
use crate::ollama::pipe::{pipe, PipeArgs};
use crate::ollama::server::ServerArgs;
use crate::ollama::OllamaArgs;
use crate::RuChatError;
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
    pub(crate) async fn handle_request(self) -> Result<(), RuChatError> {
        let default = Commands::Pipe(PipeArgs::default());
        if self.verbose {
            let command_line = std::env::args().collect::<Vec<String>>().join(" ");
            println!("Command line: {}", command_line);
        }
        match self.command.unwrap_or(default) {
            Commands::Ask(ask_args) => ask(ask_args).await?,
            Commands::Pipe(pipe_args) => pipe(pipe_args).await?,
            Commands::Chat(chat_args) => chat(chat_args).await?,
            Commands::Ls(ls_args) => list(ls_args).await?,
            Commands::Rm(rm_args) => remove(rm_args).await?,
            Commands::Pull(pull_args) => pull(pull_args).await?,
            Commands::Func(func_args) => func(func_args).await?,
            Commands::FuncStruct(func_args) => func_struct(func_args).await?,
            Commands::Embed(embed_args) => embed(embed_args).await?,
            Commands::Query(query_args) => query(query_args).await?,
            Commands::Similarity(similarity_args) => similarity_search(similarity_args).await?,
            Commands::ChromaLs(chroma_ls_args) => chroma_ls(chroma_ls_args).await?,
            Commands::ChromaDelete(chroma_delete_args) => chroma_delete(chroma_delete_args).await?,
            Commands::Manager(manager_args) => Manager::execute_command(manager_args).await?,
        }
        Ok(())
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
