mod bufcursor;
mod conversation_tree;
mod event_result;
mod history;
mod pos;
use crate::error::RuChatError;
use crate::ollama::chat::event_result::EventResult;
use crate::ollama::model::get_name;
use bufcursor::BufCursor;
use clap::Parser;
use conversation_tree::ConversationTree;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, DisableMouseCapture, EnableMouseCapture},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ollama_rs::generation::chat::{request::ChatMessageRequest, ChatMessage};
use ollama_rs::Ollama;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use tokio::task;
use tokio::time::{sleep, timeout, Duration};
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

    /// Toggle debugging mode.
    #[clap(short, long, default_value = "false")]
    pub(crate) debug: bool,
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
    // text_view is the text that is currently being displayed
    // Add the current question to the text view
    let mut text_view: Vec<String> = bufcursor.view_buffer();

    let it = chat_history.get_current_question_ids().iter().rev();
    let cp = bufcursor.get_cursor(); // Cursor position editing the question
                                     // the last line is a status line. The second to last line is the last line of the question
    text_view.push("Ask your question (Alt+Enter to send, Esc to quit):".to_string());

    // Clear the screen
    stdout.execute(Clear(ClearType::All))?;
    stdout.execute(MoveTo(0, 0))?;

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
    let offset = text_view.len().saturating_sub(terminal::size()?.1 as usize);
    text_view = text_view.split_off(offset);

    // Render the buffer with selection. NB: cursor = (column, row)
    if let Some(mut start) = bufcursor.get_selection_start() {
        let mut end = cp;
        if start.1 > end.1 || (start.1 == end.1 && start.0 > end.0) {
            std::mem::swap(&mut start, &mut end);
        }

        for (i, line) in text_view.iter().enumerate() {
            let i = i + offset;
            // highlight selected text
            if i >= start.1 && i <= end.1 {
                if i == start.1 && i == end.1 {
                    println!(
                        "{}\x1b[7m{}\x1b[0m{}",
                        &line[..start.0],
                        &line[start.0..end.0],
                        &line[end.0..]
                    );
                } else if i == start.1 {
                    println!("{}\x1b[7m{}\x1b[0m", &line[..start.0], &line[start.0..]);
                } else if i == end.1 {
                    println!("\x1b[7m{}\x1b[0m{}", &line[..end.0], &line[end.0..]);
                } else {
                    println!("\x1b[7m{line}\x1b[0m");
                }
            } else {
                println!("{line}");
            }
        }
    } else {
        print!("{}", text_view.join("\n\r"));
    }

    // Move the cursor to the correct position
    stdout.execute(MoveTo(cp.0.try_into()?, (offset + cp.1).try_into()?))?;
    stdout.flush()?;
    //min(terminal::size()?.1 - 2, text_view.len() as u16 - 2),

    Ok(())
}

fn redraw_line(stdout: &mut io::Stdout, bufcursor: &mut BufCursor) -> Result<(), RuChatError> {
    let cp = bufcursor.get_cursor();
    let line = bufcursor.get_line(cp.1)?;
    stdout.execute(Clear(ClearType::CurrentLine))?;
    stdout.execute(MoveTo(0, cp.1 as u16))?;
    if let Some(mut start) = bufcursor.get_selection_start() {
        let mut end = cp;
        if start.1 > end.1 || (start.1 == end.1 && start.0 > end.0) {
            std::mem::swap(&mut start, &mut end);
        }
        if cp.1 == start.1 {
            println!(
                "{}\x1b[7m{}\x1b[0m{}",
                &line[..start.0],
                &line[start.0..end.0],
                &line[end.0..]
            );
        } else if cp.1 == end.1 {
            println!("\x1b[7m{}\x1b[0m{}", &line[..end.0], &line[end.0..]);
        } else {
            println!("\x1b[7m{line}\x1b[0m");
        }
    } else {
        println!("{line}");
    }
    stdout.execute(MoveTo(cp.0 as u16, cp.1 as u16))?;
    stdout.flush()?;
    Ok(())
}

fn redraw_from_cursor_down(
    stdout: &mut io::Stdout,
    bufcursor: &mut BufCursor,
) -> Result<(), RuChatError> {
    let cp = bufcursor.get_cursor();
    let mut text_view: Vec<String> = bufcursor.view_buffer();
    text_view.push("Ask your question (Alt+Enter to send, Esc to quit):".to_string());
    text_view = text_view.split_off(cp.1);
    stdout.execute(Clear(ClearType::FromCursorDown))?;
    stdout.execute(MoveTo(0, cp.1 as u16))?;
    for (i, line) in text_view.iter().enumerate() {
        let i = i + cp.1;
        if let Some(mut start) = bufcursor.get_selection_start() {
            let mut end = cp;
            if start.1 > end.1 || (start.1 == end.1 && start.0 > end.0) {
                std::mem::swap(&mut start, &mut end);
            }
            if i == start.1 {
                println!(
                    "{}\x1b[7m{}\x1b[0m",
                    &line[..start.0],
                    &line[start.0..end.0]
                );
            } else if i == end.1 {
                println!("\x1b[7m{}\x1b[0m{}", &line[..end.0], &line[end.0..]);
            } else if i >= start.1 && i <= end.1 {
                println!("\x1b[7m{line}\x1b[0m");
            } else {
                println!("{line}");
            }
        } else {
            println!("{line}");
        }
    }

    stdout.execute(MoveTo(cp.0 as u16, cp.1 as u16))?;
    stdout.flush()?;
    Ok(())
}

fn redraw_from_cursor_up(
    stdout: &mut io::Stdout,
    bufcursor: &mut BufCursor,
) -> Result<(), RuChatError> {
    let cp = bufcursor.get_cursor();
    let mut text_view: Vec<String> = bufcursor.view_buffer();
    text_view.truncate(cp.1);
    stdout.execute(Clear(ClearType::FromCursorUp))?;
    stdout.execute(MoveTo(0, cp.1 as u16))?;
    for (i, line) in text_view.iter().enumerate() {
        let i = i + cp.1;
        if let Some(mut start) = bufcursor.get_selection_start() {
            let mut end = cp;
            if start.1 > end.1 || (start.1 == end.1 && start.0 > end.0) {
                std::mem::swap(&mut start, &mut end);
            }
            if i == start.1 {
                println!(
                    "{}\x1b[7m{}\x1b[0m",
                    &line[..start.0],
                    &line[start.0..end.0]
                );
            } else if i == end.1 {
                println!("\x1b[7m{}\x1b[0m{}", &line[..end.0], &line[end.0..]);
            } else if i >= start.1 && i <= end.1 {
                println!("\x1b[7m{line}\x1b[0m");
            } else {
                println!("{line}");
            }
        } else {
            println!("{line}");
        }
    }

    stdout.execute(MoveTo(cp.0 as u16, cp.1 as u16))?;
    stdout.flush()?;
    Ok(())
}

// Function to display a simple spinner
async fn show_spinner(x: usize, y: usize) {
    let mut stdout = io::stdout();
    let spinner_chars = ['⠋', '⠙', '⠹', '⠼', '⠶', '⠦', '⠤'];
    let mut index = 0;
    // get current cursor position

    loop {
        stdout.execute(Hide).expect("Hide cursor");
        stdout
            .execute(MoveTo(x as u16, y as u16))
            .expect("Move cursor");
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
    let mut clear_option = Some(ClearType::All);
    loop {
        // TODO: not clear the whole screen for every keystroke?
        // Clear the screen
        let (x, y) = bufcursor.get_cursor();
        match clear_option {
            Some(ClearType::All | ClearType::Purge) => {
                redraw_screen(&mut stdout, &chat_history.lock().unwrap(), &mut bufcursor)?;
            }
            Some(ClearType::CurrentLine | ClearType::UntilNewLine) => {
                redraw_line(&mut stdout, &mut bufcursor)?;
            }
            Some(ClearType::FromCursorDown) => {
                redraw_from_cursor_down(&mut stdout, &mut bufcursor)?;
            }
            Some(ClearType::FromCursorUp) => {
                redraw_from_cursor_up(&mut stdout, &mut bufcursor)?;
            }
            None => {}
        }
        let res = match bufcursor.handle_event(event::read()?) {
            Ok(res) => res,
            Err(e) => {
                if args.debug {
                    if stdout.execute(MoveTo(0, terminal::size()?.1 - 1)).is_err() {
                        break;
                    }
                    if stdout.execute(Clear(ClearType::CurrentLine)).is_err() {
                        break;
                    }
                    println!("Error: {:?}", e);
                    if stdout.execute(MoveTo(x as u16, y as u16)).is_err() {
                        break;
                    }
                    clear_option = None;
                }
                continue;
            }
        };

        // Wait for an event
        match res {
            EventResult::CursorChange => {
                if stdout.execute(MoveTo(x as u16, y as u16)).is_err() {
                    break;
                }
                clear_option = None;
            }
            EventResult::Quit => break,
            EventResult::Submit => {
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
                clear_option = Some(ClearType::All);
            }
            EventResult::UnhandledEvent(e) => {
                if args.debug {
                    stdout.execute(MoveTo(0, terminal::size()?.1 - 1))?;
                    stdout.execute(Clear(ClearType::CurrentLine))?;
                    println!("Unhandled event: {:?}", e);
                    stdout.execute(MoveTo(x as u16, y as u16))?;
                }
            }
            EventResult::Unchanged => clear_option = None,
            EventResult::UpdateView(ct) => clear_option = Some(ct),
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
