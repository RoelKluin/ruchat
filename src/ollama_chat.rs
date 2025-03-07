use crate::args::{Args, ChatArgs};
use crate::chat_io::ChatIO;
use crate::error::Error;
use crate::ollama::get_model_name;
use ollama_rs::generation::chat::{request::ChatMessageRequest, ChatMessage};
use ollama_rs::Ollama;
use std::sync::{Arc, Mutex};
use tokio_stream::StreamExt;

pub fn get_chat_message_request(model_name: String, prompt: String) -> ChatMessageRequest {
    ChatMessageRequest::new(model_name, vec![ChatMessage::user(prompt)])
}

pub async fn chat(ollama: Ollama, args: &Args, _chat_args: &ChatArgs) -> Result<(), Error> {
    let mut cio = ChatIO::new();
    let history = Arc::new(Mutex::new(vec![]));
    let model_name = get_model_name(&ollama, &args.model).await?;
    loop {
        let input = cio.read_line().await?;
        if input.eq_ignore_ascii_case("q") {
            break;
        }
        let request = get_chat_message_request(model_name.to_string(), input.to_string());
        let mut stream = ollama
            .send_chat_messages_with_history_stream(history.clone(), request)
            .await
            .map_err(|_| Error::StreamWriteError(std::io::ErrorKind::Other.into()))?;

        let mut response = String::new();
        while let Some(Ok(res)) = stream.next().await {
            cio.write_line(&res.message.content).await?;
            response += res.message.content.as_str();
        }
    }
    Ok(())
}
