use crate::chroma_query::QueryArgs;
use crate::ollama::PullArgs;
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

    #[clap(short, long, default_value = "http://0.0.0.0:11434")]
    pub(crate) server: String,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Query command for specific tasks
    Ask(AskArgs),
    Chat(ChatArgs),
    Embed(EmbedArgs),
    Func(FuncArgs),
    FuncStruct(FuncStructArgs),
    List,
    Pull(PullArgs),
    Query(QueryArgs),
}
