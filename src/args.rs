use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct Args {
    #[clap(subcommand)]
    pub command: Option<Commands>,

    #[clap(short, long, default_value = "qwen2.5-coder:32b")]
    pub(crate) model: String,

    /// Chroma database server address and port
    #[clap(short = 'C', long, default_value = "http://localhost:8000")]
    pub(crate) chroma_server: String,

    /// Chroma database name
    #[clap(short = 'd', long, default_value = "default")]
    pub(crate) chroma_database: String,

    /// Chroma token for authentication
    #[clap(short = 't', long)]
    pub(crate) chroma_token: Option<String>,

    #[clap(short, long, default_value = "http://0.0.0.0:11434")]
    pub(crate) server: String,

    /// Path to a JSON file to amend default generation options, listed in
    /// https://docs.rs/ollama-rs/latest/ollama_rs/generation/options/struct.GenerationOptions.html
    #[clap(short, long)]
    pub(crate) config: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Query command for specific tasks
    Ask(AskArgs),
    Chat,
    Embed(EmbedArgs),
    Func,
    FuncStruct,
    List,
    Pull,
}

#[derive(Parser, Debug)]
pub struct AskArgs {
    #[clap(short, long, default_value = "What do you make of this?")]
    pub(crate) prompt: String,

    /// Request a certain output format, the default leaves the text as is
    #[clap(short, long, default_value_t = String::from("text"))]
    pub(crate) output_format: String,

    /// Text files to use as input, seperated by commas
    #[clap(short = 'i', long)]
    pub(crate) text_files: Option<String>,
}

#[derive(Parser, Debug)]
pub struct EmbedArgs {
    #[clap(short, long, default_value = "What do you make of this?")]
    pub(crate) prompt: String,

    /// Chroma database collection name
    #[clap(short, long, default_value = "default")]
    pub(crate) collection: String,

    /// Chroma database metadata, comma separated key:value pairs
    #[clap(short, long, default_value = "version:0.01")]
    pub(crate) metadata: Option<String>,
}
