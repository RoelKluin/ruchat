mod bufcursor;
mod conversation_tree;
mod event_result;
mod history;
mod pos;
use crate::error::{RuChatError, Result};
use crate::ollama::OllamaArgs;
use crate::ollama::chat::event_result::EventResult;
use bufcursor::BufCursor;
use clap::{ArgAction, Parser};
use conversation_tree::ConversationTree;
use crossterm::{
    ExecutableCommand,
    cursor::{Hide, MoveTo, Show},
    event::{self, DisableMouseCapture, EnableMouseCapture},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use ollama_rs::generation::chat::{ChatMessage, request::ChatMessageRequest};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use tokio::task;
use tokio::time::{Duration, sleep, timeout};
use tokio_stream::StreamExt;

/// Command-line arguments for interactive chat sessions with a model.
///
/// This struct defines the arguments required to start an interactive
/// chat session with a model, including model details.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct ChatArgs {
    /// Toggle debugging mode.
    #[arg(short, long, action=ArgAction::Count)]
    debug: u8,

    #[command(flatten)]
    ollama_args: OllamaArgs,
}

impl ChatArgs {
    /// Starts an interactive chat session with a model.
    ///
    /// This function enters raw mode and sets up an interactive chat session
    /// with the specified model, allowing the user to enter prompts and receive
    /// responses.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn chat(&self) -> Result<()> {
        // Enter raw mode and alternate screen
        terminal::enable_raw_mode()?;
        match self.chat_raw_mode().await {
            Ok(_) => {}
            Err(e) => {
                terminal::disable_raw_mode()?;
                return Err(e);
            }
        }
        terminal::disable_raw_mode()?;
        Ok(())
    }
    /// Runs the chat session in raw mode.
    ///
    /// This function sets up the chat session in raw mode, allowing the user
    /// to interact with the model in an interactive chat session.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    async fn chat_raw_mode(&self) -> Result<()> {
        let chat_history = Arc::new(Mutex::new(ConversationTree::new()));

        //let running = Arc::new(Mutex::new(true))
        let mut stdout = io::stdout();
        let ollama = self.ollama_args.init()?;
        let model: String = self.ollama_args.get_model(&ollama, "").await?;
        let mut bufcursor = BufCursor::new()?;
        let debug_level = self.debug;
        if debug_level & 0x2 != 0 {
            for c in "Lorem ipsum dolor sit amet,consectetur adipiscing elit,\nsed do eiusmod tempor incididunt ut labore et dolore magna aliqua.\nUt enim ad minim veniam,\nquis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.\nDuis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur.\nExcepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum".chars() {
                if let Err(e) = bufcursor.handle_event(event::Event::Key(event::KeyEvent {
                    code: event::KeyCode::Char(c),
                    modifiers: event::KeyModifiers::NONE,
                    kind: event::KeyEventKind::Press,
                    state: event::KeyEventState::NONE,
                })) {
                    display_err(debug_level > 0, &mut stdout, "Error handling event", 0, 0, e);
                }
            }
        }
        stdout.execute(EnterAlternateScreen)?;
        stdout.execute(EnableMouseCapture)?;

        let history = Arc::new(Mutex::new(vec![]));
        let mut clear_option = Some(ClearType::All);
        loop {
            // Clear the screen
            let res = match clear_option {
                Some(ClearType::All | ClearType::Purge) => {
                    redraw_screen(&mut stdout, &chat_history.lock().unwrap(), &mut bufcursor)
                }
                Some(ClearType::CurrentLine | ClearType::UntilNewLine) => {
                    redraw_screen(&mut stdout, &chat_history.lock().unwrap(), &mut bufcursor)
                    //redraw_line(&mut stdout, &mut bufcursor)?;
                }
                Some(ClearType::FromCursorDown) => {
                    redraw_screen(&mut stdout, &chat_history.lock().unwrap(), &mut bufcursor)
                    //redraw_from_cursor_down(&mut stdout, &mut bufcursor)?;
                }
                Some(ClearType::FromCursorUp) => {
                    redraw_screen(&mut stdout, &chat_history.lock().unwrap(), &mut bufcursor)
                    //redraw_from_cursor_up(&mut stdout, &mut bufcursor)?;
                }
                None => Ok(()),
            };
            clear_option = None;
            let (x, y) = bufcursor.get_cursor();
            match res
                .and_then(|_| event::read().map_err(|e| e.into()))
                .and_then(|e| bufcursor.handle_event(e))
            {
                Ok(EventResult::CursorChange) => {
                    let (x, y) = bufcursor.get_cursor();
                    if let Err(e) = stdout.execute(MoveTo(x as u16, y as u16)) {
                        display_err(debug_level > 0, &mut stdout, "Error: MoveTo", x, y, e);
                    }
                }
                Ok(EventResult::Quit) => break,
                Ok(EventResult::Submit) => {
                    let request = get_chat_message_request(model.clone(), bufcursor.read());
                    let question_id = match chat_history.lock().unwrap().question(bufcursor.drain()) {
                        Ok(id) => id,
                        Err(e) => {
                            display_err(debug_level > 0, &mut stdout, "Error (qid)", x, y, e);
                            continue;
                        }
                    };

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
                        if let Err(e) = stdout.execute(Show) {
                            display_err(
                                debug_level > 0,
                                &mut stdout,
                                "Error: Show cursor: ",
                                x,
                                y,
                                e,
                            );
                        }
                        if let Err(e) = stdout.execute(Clear(ClearType::CurrentLine)) {
                            display_err(debug_level > 0, &mut stdout, "Error: Clear line: ", x, y, e);
                        }
                    });

                    task.await?;
                    clear_option = Some(ClearType::All);
                }
                Ok(EventResult::UnhandledEvent(evt)) => match evt {
                    event::Event::Key(event::KeyEvent { .. }) => {
                        display_err(
                            debug_level & 0x4 != 0,
                            &mut stdout,
                            "Unhandled event",
                            x,
                            y,
                            evt,
                        );
                    }
                    _ => {
                        display_err(
                            debug_level & 0x8 != 0,
                            &mut stdout,
                            "Unhandled event",
                            x,
                            y,
                            evt,
                        );
                    }
                },
                Ok(EventResult::Unchanged) => {}
                Ok(EventResult::UpdateView(ct)) => clear_option = Some(ct),
                Err(e) => display_err(debug_level > 0, &mut stdout, "Error", x, y, e),
            }
        }

        // Leave raw mode and alternate screen
        stdout.execute(LeaveAlternateScreen)?;
        stdout.execute(DisableMouseCapture)?;
        Ok(())
    }
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
) -> Result<()> {
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
    if let Some((start, end)) = bufcursor.normalized_selection() {
        for (i, line) in text_view.iter().enumerate() {
            let i = i + offset;
            // highlight selected text
            if i >= start.1 && i <= end.1 {
                if i == start.1 && i == end.1 {
                    if end.0 > line.len() {
                        // XXX
                        return Err(RuChatError::InvalidCursorPosition(
                            format!("{i} (both), end"),
                            end.0,
                            end.1,
                        ));
                    } else if start.0 > line.len() {
                        return Err(RuChatError::InvalidCursorPosition(
                            format!("{i} (both), start"),
                            start.0,
                            start.1,
                        ));
                    }
                    println!(
                        "{}\x1b[7m{}\x1b[0m{}",
                        &line[..start.0],
                        &line[start.0..end.0],
                        &line[end.0..]
                    );
                } else if i == start.1 {
                    if start.0 > line.len() {
                        return Err(RuChatError::InvalidCursorPosition(
                            format!("{i}, start (only)"),
                            start.0,
                            start.1,
                        ));
                    }
                    println!("{}\x1b[7m{}\x1b[0m", &line[..start.0], &line[start.0..]);
                } else if i == end.1 {
                    if end.0 > line.len() {
                        return Err(RuChatError::InvalidCursorPosition(
                            format!("{i}, end (only)"),
                            end.0,
                            end.1,
                        ));
                    }
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

fn display_err<E>(do_debug: bool, stdout: &mut io::Stdout, msg: &str, x: usize, y: usize, e: E)
where
    E: std::fmt::Debug,
{
    if do_debug {
        match terminal::size().map(|i| i.1.saturating_sub(1)) {
            Err(e2) => println!("Error while trying to get terminal size: {:?}", e2),
            Ok(i) => match stdout.execute(MoveTo(0, i)) {
                Err(e2) => println!("Error while trying to move cursor: {:?}", e2),
                Ok(_) => match stdout.execute(Clear(ClearType::CurrentLine)) {
                    Err(e2) => println!("Error while trying to clear line: {:?}", e2),
                    Ok(_) => {
                        println!("{msg}: {:?}", e);
                        if let Err(e2) = stdout.execute(MoveTo(x as u16, y as u16)) {
                            println!("Error: MoveTo({x}, {y}): {:?}", e2);
                        }
                    }
                },
            },
        }
    }
}

fn redraw_line(stdout: &mut io::Stdout, bufcursor: &mut BufCursor) -> Result<()> {
    let cp = bufcursor.get_cursor();
    let line = bufcursor.get_line(cp.1)?;
    stdout.execute(Clear(ClearType::CurrentLine))?;
    stdout.execute(MoveTo(0, cp.1 as u16))?;
    if let Some((start, end)) = bufcursor.normalized_selection() {
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
) -> Result<()> {
    let cp = bufcursor.get_cursor();
    let mut text_view: Vec<String> = bufcursor.view_buffer();
    text_view.push("Ask your question (Alt+Enter to send, Esc to quit):".to_string());
    text_view = text_view.split_off(cp.1);
    let offset = text_view.len().saturating_sub(terminal::size()?.1 as usize);

    stdout.execute(Clear(ClearType::FromCursorDown))?;
    stdout.execute(MoveTo(0, cp.1 as u16))?;
    for (i, line) in text_view.iter().enumerate() {
        let i = i + offset;
        if let Some((start, end)) = bufcursor.normalized_selection() {
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
) -> Result<()> {
    let cp = bufcursor.get_cursor();
    let mut text_view: Vec<String> = bufcursor.view_buffer();
    text_view.truncate(cp.1);
    let offset = text_view.len().saturating_sub(terminal::size()?.1 as usize);
    stdout.execute(Clear(ClearType::FromCursorUp))?;
    stdout.execute(MoveTo(0, cp.1 as u16))?;
    for (i, line) in text_view.iter().enumerate() {
        let i = i + offset;
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
