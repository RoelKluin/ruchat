use crate::args::Args;
use crate::error::RuChatError;
use crate::ollama::get_model_name;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::Ollama;

pub(crate) async fn embed(ollama: Ollama, args: &Args) -> Result<(), RuChatError> {
    let model_name = get_model_name(&ollama, &args.model).await?;
    let request = GenerateEmbeddingsRequest::new(model_name, "Why is the sky blue?".into());
    let _res = ollama.generate_embeddings(request).await?;
    Ok(())
}
