use crate::chroma_ls::ChromaLsArgs;
use crate::chroma_query::QueryArgs;
use crate::chroma_similarity_search::SimilarityArgs;
use crate::ollama::{PullArgs, RmArgs};
use crate::ollama_ask::AskArgs;
use crate::ollama_chat::ChatArgs;
use crate::ollama_embed::EmbedArgs;
use crate::ollama_func::FuncArgs;
use crate::ollama_func_struct::FuncStructArgs;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct Args {
    #[clap(subcommand)]
    pub command: Option<Commands>,

    #[clap(short, long, default_value = "http://172.18.0.1:11434")]
    pub(crate) server: String,

    #[clap(short, long, default_value = "false")]
    pub(crate) verbose: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Query command for specific tasks
    Ask(AskArgs),
    Chat(ChatArgs),
    Embed(EmbedArgs),
    Func(FuncArgs),
    FuncStruct(FuncStructArgs),
    Ls,
    Rm(RmArgs),
    Pull(PullArgs),
    Query(QueryArgs),
    Similarity(SimilarityArgs),
    ChromaLs(ChromaLsArgs),
}
