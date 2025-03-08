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

#[derive(Parser, Debug, Clone, Default)]
pub struct AskArgs {
    #[clap(short, long, default_value = "qwen2.5-coder:32b")]
    pub(crate) model: String,

    #[clap(short, long)]
    pub(crate) prompt: Option<String>,

    /// Request a certain output format, the default leaves the text as is
    #[clap(short, long, default_value_t = String::from("text"))]
    pub(crate) output_format: String,

    /// Text files to use as input, seperated by commas
    #[clap(short = 'i', long)]
    pub(crate) text_files: Option<String>,

    /// Path to a JSON file to amend default generation options, listed in
    /// https://docs.rs/ollama-rs/latest/ollama_rs/generation/options/struct.GenerationOptions.html
    #[clap(short, long)]
    pub(crate) config: Option<String>,
}

#[derive(Parser, Debug, Clone)]
pub struct ChatArgs {
    #[clap(short, long, default_value = "qwen2.5-coder:32b")]
    pub(crate) model: String,
}

#[derive(Parser, Debug, Clone)]
pub struct EmbedArgs {
    #[clap(short, long, default_value = "nomic-embed-text:latest")]
    pub(crate) model: String,

    #[clap(short, long)]
    pub(crate) prompt: String,

    /// Chroma database server address and port
    #[clap(short = 'C', long, default_value = "http://localhost:8000")]
    pub(crate) chroma_server: String,

    /// Chroma database name
    #[clap(short = 'd', long, default_value = "default")]
    pub(crate) chroma_database: String,

    /// Chroma token for authentication
    #[clap(short = 't', long)]
    pub(crate) chroma_token: Option<String>,

    /// Chroma database collection name
    #[clap(short, long, default_value = "default")]
    pub(crate) collection: String,

    /// Chroma database metadata, comma separated key:value pairs
    #[clap(short, long, default_value = "version:0.01")]
    pub(crate) metadata: Option<String>,
}

#[derive(Parser, Debug, Clone)]
pub struct FuncArgs {
    #[clap(short, long, default_value = "qwen2.5-coder:32b")]
    pub(crate) model: String,
}

#[derive(Parser, Debug, Clone)]
pub struct FuncStructArgs {
    #[clap(short, long, default_value = "qwen2.5-coder:32b")]
    pub(crate) model: String,
}

#[derive(Parser, Debug, Clone)]
pub struct PullArgs {
    #[clap(short, long)]
    pub(crate) model: String,
}

#[derive(Parser, Debug, Clone)]
pub struct QueryArgs {
    #[clap(short, long)]
    pub(crate) query: String,

    #[clap(short, long, default_value = "1")]
    pub(crate) count: usize,

    /// Chroma database collection name
    #[clap(short, long, default_value = "default")]
    pub(crate) collection: String,

    /// Chroma database metadata, comma separated key:value pairs
    #[clap(short, long)]
    pub(crate) metadata: Option<String>,

    /// Chroma database server address and port
    #[clap(short = 'C', long, default_value = "http://localhost:8000")]
    pub(crate) chroma_server: String,

    /// Chroma database name
    #[clap(short = 'd', long, default_value = "default")]
    pub(crate) chroma_database: String,

    /// Chroma token for authentication
    #[clap(short = 't', long)]
    pub(crate) chroma_token: Option<String>,
}
