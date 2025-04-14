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
