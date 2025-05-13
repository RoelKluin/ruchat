mod bufcursor;
mod conversation_tree;
use crate::error::RuChatError;
use crate::ollama::model::get_name;
use bufcursor::BufCursor;
use clap::Parser;
use conversation_tree::ConversationTree;
use crossterm::{
    ExecutableCommand,
    cursor::{MoveTo, Show, Hide},
    event::{self, DisableMouseCapture, EnableMouseCapture},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use ollama_rs::Ollama;
use ollama_rs::generation::chat::{ChatMessage, request::ChatMessageRequest};
use std::cmp::min;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use tokio::task;
use tokio::time::{Duration, timeout, sleep};
use tokio_stream::StreamExt;

/// Command-line arguments for interactive chat sessions with a model.
///
/// This struct defines the arguments required to start an interactive
/// chat session with a model, including model details.
#[derive(Parser, Debug, Clone, PartialEq)]
pub struct ChatArgs {
    /// The model to use for the chat session.
    #[clap(short, long, default_value = "qwen2.5-coder:14b")]
    pub(crate) model: String,
}

/// Creates a chat message request for the model.
///
/// This function constructs a chat message request using the specified
/// model name and user prompt.
///
/// # Parameters
///
/// - `model_name`: The name of the model to use.
/// - `prompt`: The user prompt to send to the model.
///
/// # Returns
///
/// A `ChatMessageRequest` containing the model name and user prompt.
fn get_chat_message_request(model_name: String, prompt: String) -> ChatMessageRequest {
    ChatMessageRequest::new(model_name, vec![ChatMessage::user(prompt)])
}

/// Redraws the chat screen with the current chat history and buffer cursor.
///
/// This function clears the screen and displays the current chat history
/// and buffer cursor position.
///
/// # Parameters
///
/// - `stdout`: The standard output stream.
/// - `chat_history`: The conversation tree containing the chat history.
/// - `bufcursor`: The buffer cursor for editing the current question.
///
/// # Returns
///
/// A `Result` indicating success or failure.
fn redraw_screen(
    stdout: &mut io::Stdout,
    chat_history: &ConversationTree,
    bufcursor: &mut BufCursor,
) -> Result<(), RuChatError> {
    // Clear the screen
    stdout.execute(Clear(ClearType::All))?;
    stdout.execute(MoveTo(0, 0))?;

    // text_view is the text that is currently being displayed
    // Add the current question to the text view
    let mut text_view: Vec<String> = bufcursor.view_buffer();
    let it = chat_history.get_current_question_ids().iter().rev();
    let cp = bufcursor.get_cursor(); // Cursor position editing the question
    // the last line is a status line. The second to last line is the last line of the question
    text_view.push("Ask your question (Alt+Enter to send, Esc to quit):".to_string());

    for &question_id in it {
        let answer_id = chat_history.get_current_answer_id(question_id);
        if let Some((mut question, mut response)) = chat_history.get_qa(question_id, answer_id) {
            response.push(chat_history.get_answer_nr_of_total(answer_id) + "[Redo][Del]");
            response.append(&mut text_view);
            text_view = response;
            question.push(chat_history.get_question_nr_of_total(question_id) + "[Edit][Del]");
            question.append(&mut text_view);
            text_view = question;
        } else {
            text_view.push("No more questions".to_string());
            break;
        }
    }
    print!("{}", text_view.join("\n\r"));
    stdout.flush()?;
    text_view = text_view.split_off(text_view.len().saturating_sub(terminal::size()?.1 as usize));

    // Move the cursor to the correct position
    stdout.execute(MoveTo(
        cp.0.try_into()?,
        min(terminal::size()?.1 - 2, text_view.len() as u16 - 2),
    ))?;

    Ok(())
}

// Function to display a simple spinner
async fn show_spinner(x: usize, y: usize) {
    let mut stdout = io::stdout();
    let spinner_chars = vec!['⠋', '⠙', '⠹', '⠼', '⠶', '⠦', '⠤'];
    let mut index = 0;
    // get current cursor position

    loop {
        stdout.execute(Hide).expect("Hide cursor");
        stdout.execute(MoveTo(x as u16, y as u16)).expect("Move cursor");
        print!("{}", spinner_chars[index]);
        stdout.flush().unwrap();
        index = (index + 1) % spinner_chars.len();
        sleep(Duration::from_millis(100)).await;
    }
}

/// Runs the chat session in raw mode.
///
/// This function sets up the chat session in raw mode, allowing the user
/// to interact with the model in an interactive chat session.
///
/// # Parameters
///
/// - `ollama`: The Ollama client for generating responses.
/// - `args`: The command-line arguments for the chat session.
///
/// # Returns
///
/// A `Result` indicating success or failure.
async fn chat_raw_mode(ollama: Ollama, args: &ChatArgs) -> Result<(), RuChatError> {
    let chat_history = Arc::new(Mutex::new(ConversationTree::new()));

    //let running = Arc::new(Mutex::new(true))
    let mut stdout = io::stdout();
    let model_name = get_name(&ollama, &args.model).await?;
    let mut bufcursor = BufCursor::new()?;
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;

    let history = Arc::new(Mutex::new(vec![]));
    loop {
        // TODO: not clear the whole screen for every keystroke?
        // Clear the screen
        let (x, y) = bufcursor.get_cursor();
        redraw_screen(&mut stdout, &chat_history.lock().unwrap(), &mut bufcursor)?;

        // Wait for an event
        match bufcursor.handle_key_event(event::read()?)? {
            b'q' => break,
            b'\n' => {
                let request = get_chat_message_request(model_name.to_string(), bufcursor.read());
                let question_id = chat_history.lock().unwrap().question(bufcursor.drain())?;

                let chat_hist_clone = chat_history.clone();
                let hist = history.clone();
                let ol = ollama.clone();

                // Start the spinner in a separate task
                let spinner_handle = task::spawn(show_spinner(x, y));
                let mut stdout = io::stdout();

                let task = task::spawn(async move {
                    let result = ol.send_chat_messages_with_history_stream(hist, request);
                    match timeout(Duration::from_secs(600), result).await {
                        Ok(Ok(mut stream)) => {
                            let mut response = vec!["".to_string()];
                            while let Some(Ok(mut res)) = stream.next().await {
                                let last = response.len() - 1;
                                while let Some((first, second)) =
                                    res.message.content.split_once('\n')
                                {
                                    if response[last].len() + first.len()
                                        > terminal::size().expect("terminal size").0 as usize
                                    {
                                        response.push("".to_string());
                                    }
                                    response[last].push_str(first);
                                    response.push("".to_string());
                                    res.message.content = second.to_string();
                                }
                                if !res.message.content.is_empty() {
                                    if response[last].len() + res.message.content.len()
                                        > terminal::size().expect("terminal size").1 as usize
                                    {
                                        response.push("".to_string());
                                    }
                                    response[last].push_str(&res.message.content);
                                }
                            }
                            let mut chat_hist = chat_hist_clone.lock().unwrap();
                            let _ = chat_hist.add_answer(question_id, response);
                        }
                        Ok(Err(e)) => {
                            let mut chat_hist = chat_hist_clone.lock().unwrap();
                            let _ =
                                chat_hist.add_answer(question_id, vec![format!("Error: {}", e)]);
                        }
                        Err(_) => {
                            let mut chat_hist = chat_hist_clone.lock().unwrap();
                            let _ = chat_hist.add_answer(question_id, vec!["Timeout".to_string()]);
                        }
                    }
                    // Stop the spinner
                    spinner_handle.abort();
                    stdout.execute(Show).unwrap();
                    stdout.execute(Clear(ClearType::CurrentLine)).unwrap();
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

/// Starts an interactive chat session with a model.
///
/// This function enters raw mode and sets up an interactive chat session
/// with the specified model, allowing the user to enter prompts and receive
/// responses.
///
/// # Parameters
///
/// - `ollama`: The Ollama client for generating responses.
/// - `args`: The command-line arguments for the chat session.
///
/// # Returns
///
/// A `Result` indicating success or failure.
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
