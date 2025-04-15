use crate::chroma::ls::ChromaLsArgs;
use crate::chroma::query::QueryArgs;
use crate::chroma::similarity::SimilarityArgs;
use crate::ollama::model::pull::PullArgs;
use crate::ollama::model::rm::RmArgs;
use crate::ollama::ask::AskArgs;
use crate::ollama::chat::ChatArgs;
use crate::embed::EmbedArgs;
use crate::ollama::func::FuncArgs;
use crate::ollama::func::strukt::FuncStructArgs;
use clap::{Parser, Subcommand};

/// main command line interface for RuChat
#[derive(Parser, Debug)]
pub struct Args {
    #[clap(subcommand)]
    pub command: Option<Commands>,

    /// address and port of the ollama server
    #[clap(short, long, default_value = "http://172.18.0.1:11434")]
    pub(crate) server: String,

    /// toggle verbose mode
    #[clap(short, long, default_value = "false")]
    pub(crate) verbose: bool,
}

/// Subcommands for RuChat
#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Query language model using a prompt, you may including file context
    Ask(AskArgs),
    /// Chat with a language model
    Chat(ChatArgs),
    /// List models
    Ls,
    /// Remove a model
    Rm(RmArgs),
    /// Pull a model from a remote ollama server
    Pull(PullArgs),
    /// Run a function using a language model
    Func(FuncArgs),
    /// Run a function using a language model with structured input
    FuncStruct(FuncStructArgs),
    /// use embedding model to create embeddings in Chroma
    Embed(EmbedArgs),
    /// Query Chroma database
    Query(QueryArgs),
    /// Find similar embeddings in Chroma database
    Similarity(SimilarityArgs),
    /// List Chroma database collections
    ChromaLs(ChromaLsArgs),
}
