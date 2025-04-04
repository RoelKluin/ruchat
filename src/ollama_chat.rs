use crate::bufcursor::BufCursor;
use crate::chat_io::ChatIO;
use crate::conversation_tree::ConversationTree;
use crate::error::RuChatError;
use crate::ollama::get_model_name;
use clap::Parser;
use crossterm::{
    cursor::{self, MoveDown, MoveLeft, MoveRight, MoveTo, MoveUp},
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ollama_rs::generation::chat::{request::ChatMessageRequest, ChatMessage};
use ollama_rs::Ollama;
use std::cmp::min;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::sync::Mutex as TokioMutex;
use tokio::task;
use tokio::time::{sleep, timeout, Duration};
use tokio_stream::StreamExt;

#[derive(Parser, Debug, Clone)]
pub struct ChatArgs {
    #[clap(short, long, default_value = "qwen2.5-coder:32b")]
    pub(crate) model: String,
}

fn get_chat_message_request(model_name: String, prompt: String) -> ChatMessageRequest {
    ChatMessageRequest::new(model_name, vec![ChatMessage::user(prompt)])
}

fn redraw_screen(
    stdout: &mut io::Stdout,
    chat_history: &ConversationTree,
    bufcursor: &mut BufCursor,
) -> Result<(), RuChatError> {
    // Clear the screen
    stdout.execute(Clear(ClearType::All))?;
    stdout.execute(MoveTo(0, 0))?;

    let term_lines = terminal::size()?.1 as usize;
    // text_view is the text that is currently being displayed
    // Add the current question to the text view
    let mut text_view: Vec<String> = bufcursor.view_buffer();
    let mut it = chat_history.get_current_question_ids().iter().rev();
    let cp = bufcursor.get_cursor(); // Cursor position editing the question
    let mut cursor_offset = cp.1.try_into()?;
    // the last line is a status line. The second to last line is the last line of the question
    text_view.push("Enter your question (Esc to quit):".to_string());

    while text_view.len() < term_lines {
        let question_id = match it.next() {
            Some(id) => *id,
            None => break,
        };
        let answer_id = chat_history.get_current_answer_id(question_id);
        if let Some((mut question, mut response)) = chat_history.get_qa(question_id, answer_id) {
            response.push(chat_history.get_answer_nr_of_total(answer_id) + "[Redo][Del]");
            cursor_offset += response.len() as u16;
            response.append(&mut text_view);
            text_view = response;
            if text_view.len() < term_lines {
                question.push(chat_history.get_question_nr_of_total(question_id) + "[Edit][Del]");
                cursor_offset += question.len() as u16;
                question.append(&mut text_view);
                text_view = question;
            }
        } else {
            text_view.push("No more questions".to_string());
            break;
        }
    }
    if text_view.len() > term_lines {
        text_view = text_view.split_off(term_lines);
    }
    print!("{}", text_view.join("\n\r"));
    stdout.flush()?;

    // Move the cursor to the correct position
    stdout.execute(MoveTo(cp.0.try_into()?, cursor_offset))?;

    Ok(())
}

async fn display_runner(running: Arc<Mutex<bool>>) {
    let mut position = 0;
    let runner_chars = vec!['|', '/', '-', '\\'];
    while *running.lock().unwrap() {
        print!("\r{}", runner_chars[position]);
        position = (position + 1) % runner_chars.len();
        sleep(Duration::from_millis(100)).await;
    }
    print!("\r "); // Clear the runner character
}

async fn generate_response(question: String) -> Result<String, &'static str> {
    // Simulate server response generation
    sleep(Duration::from_secs(3)).await;
    Ok(format!("Response to: {}", question))
}

async fn handle_question() {
    // Simulate handling a question
    sleep(Duration::from_secs(5)).await;
}

async fn chat_raw_mode(ollama: Ollama, args: &ChatArgs) -> Result<(), RuChatError> {
    let chat_history = Arc::new(Mutex::new(ConversationTree::new()));

    //let running = Arc::new(Mutex::new(true))
    let mut stdout = io::stdout();
    let mut bufcursor = BufCursor::new();
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;

    let history = Arc::new(Mutex::new(vec![]));
    let model_name = get_model_name(&ollama, &args.model).await?;
    loop {
        // TODO: not clear the whole screen for every keystroke?
        // Clear the screen
        redraw_screen(&mut stdout, &chat_history.lock().unwrap(), &bufcursor)?;

        // Wait for an event
        match bufcursor.handle_key_event(event::read()?)? {
            b'q' => break,
            b'\n' => {
                let request = get_chat_message_request(model_name.to_string(), bufcursor.read());
                let question_id = chat_history.lock().unwrap().add_question(bufcursor.drain());

                let chat_hist_clone = chat_history.clone();
                let hist = history.clone();
                let ol = ollama.clone();

                let task = task::spawn(async move {
                    let result = ol.send_chat_messages_with_history_stream(hist, request);
                    match timeout(Duration::from_secs(10), async { result.await }).await {
                        Ok(Ok(mut stream)) => {
                            let mut response = vec![];
                            while let Some(Ok(res)) = stream.next().await {
                                response.push(res.message.content);
                            }
                            let mut chat_hist = chat_hist_clone.lock().unwrap();
                            let _ = chat_hist.answer(question_id, response);
                        }
                        Ok(Err(e)) => {
                            let mut chat_hist = chat_hist_clone.lock().unwrap();
                            let _ = chat_hist.answer(question_id, vec![format!("Error: {}", e)]);
                        }
                        Err(_) => {
                            let mut chat_hist = chat_hist_clone.lock().unwrap();
                            let _ = chat_hist.answer(question_id, vec!["Timeout".to_string()]);
                        }
                    }
                });

                task.await?;
            }
            _ => {}
        }
    }

    // Leave raw mode and alternate screen
    stdout.execute(LeaveAlternateScreen)?;
    stdout.execute(DisableMouseCapture)?;
    Ok(())
}

pub(crate) async fn chat(ollama: Ollama, args: &ChatArgs) -> Result<(), RuChatError> {
    // Enter raw mode and alternate screen
    terminal::enable_raw_mode()?;
    match chat_raw_mode(ollama, args).await {
        Ok(_) => {}
        Err(e) => {
            terminal::disable_raw_mode()?;
            return Err(e);
        }
    }
    terminal::disable_raw_mode()?;
    Ok(())
}
