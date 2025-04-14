use crate::args::{Args, Commands};
use crate::chroma::ls::chroma_ls;
use crate::chroma::query::query;
use crate::chroma::similarity::similarity_search;
use crate::error::RuChatError;
use crate::ollama::ask::{ask, AskArgs};
use crate::ollama::chat::chat;
use crate::embed::embed;
use crate::ollama::model::ls::list;
use crate::ollama::model::rm::remove;
use crate::ollama::model::pull::pull;
use crate::ollama::func::func;
use crate::ollama::func::strukt::func_struct;
use crate::ollama::init;

pub async fn handle_request(args: &Args) -> Result<(), RuChatError> {
    let default = Commands::Ask(AskArgs::default());
    if args.verbose {
        let command_line = std::env::args().collect::<Vec<String>>().join(" ");
        println!("Command line: {}", command_line);
    }
    match args.command.as_ref().unwrap_or(&default) {
        Commands::Ask(ask_args) => ask(init(args)?, ask_args).await?,
        Commands::Chat(chat_args) => chat(init(args)?, chat_args).await?,
        Commands::Embed(embed_args) => embed(init(args)?, embed_args).await?,
        Commands::Func(func_args) => func(init(args)?, func_args).await?,
        Commands::FuncStruct(func_args) => func_struct(init(args)?, func_args).await?,
        Commands::Ls => list(args).await?,
        Commands::Rm(rm_args) => remove(args, rm_args).await?,
        Commands::Pull(pull_args) => pull(args, pull_args).await?,
        Commands::Query(query_args) => query(init(args)?, query_args).await?,
        Commands::Similarity(similarity_args) => similarity_search(similarity_args).await?,
        Commands::ChromaLs(chroma_ls_args) => chroma_ls(chroma_ls_args).await?,
    }
    Ok(())
}
