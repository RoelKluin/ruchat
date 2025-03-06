use clap::Parser;

#[derive(Parser, Debug)]
pub struct Args {
    #[clap(short, long)]
    pub(crate) prompt: String,

    #[clap(short, long, default_value = "qwen2.5-coder:32b")]
    pub(crate) model: String,

    /// Request a certain output format, the default leaves the text as is
    #[clap(short, long, default_value_t = String::from("text"))]
    pub(crate) output_format: String,

    /// Text files to use as input, seperated by commas
    #[clap(short, long)]
    pub(crate) text_files: Option<String>,

    /// History file to use as input - invokes chat mode. #TODO
    #[clap(short, long)]
    pub(crate) history_file: Option<String>,

    /// Chroma database server address and port
    #[clap(short, long, default_value = "http://localhost:8000")]
    pub(crate) chroma_server: String,

    /// Chroma database name
    #[clap(short, long, default_value = "default")]
    pub(crate) chroma_database: String,

    /// Chroma token for authentication
    #[clap(short, long)]
    pub(crate) chroma_token: Option<String>,

    #[clap(short, long, default_value = "http://0.0.0.0:11434")]
    pub(crate) server: String,

    /// Path to a JSON file to amend default generation options, listed in
    /// https://docs.rs/ollama-rs/latest/ollama_rs/generation/options/struct.GenerationOptions.html
    #[clap(short, long)]
    pub(crate) config: Option<String>,
}
