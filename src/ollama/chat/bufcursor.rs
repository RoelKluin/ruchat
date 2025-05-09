use crate::error::RuChatError;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use crossterm::terminal;

/// A struct for managing a buffer cursor in a text-based interface.
///
/// This struct provides methods for handling key events, navigating
/// the buffer, and editing text.
pub(crate) struct BufCursor {
    buffer: Vec<String>,
    cursor: (usize, usize),
}

impl BufCursor {
    /// Creates a new `BufCursor` instance.
    ///
    /// # Returns
    ///
    /// A new instance of `BufCursor` with an empty buffer and cursor
    /// positioned at the start.
    pub(crate) fn new() -> Self {
        Self {
            buffer: vec![String::new()],
            cursor: (0, 0),
        }
    }

    /// Reads the current buffer as a single string.
    ///
    /// # Returns
    ///
    /// A `String` containing the contents of the buffer.
    pub(crate) fn read(&self) -> String {
        self.buffer.join("\n")
    }

    /// Returns a view of the buffer for display purposes.
    ///
    /// This function returns a vector of strings representing the
    /// current view of the buffer, limited by the terminal size.
    ///
    /// # Returns
    ///
    /// A `Vec<String>` containing the lines of the buffer to display.
    pub(crate) fn view_buffer(&self) -> Vec<String> {
        let term_lines = (terminal::size().unwrap().1 as usize).saturating_sub(1);
        self.buffer
            .iter()
            .skip(self.cursor.1.saturating_sub(term_lines))
            .take(term_lines)
            .map(|s| s.to_string() + "\r")
            .collect()
    }

    fn debug<D>(&mut self, d: D) -> Result<(), RuChatError>
    where
        D: std::fmt::Debug,
    {
        for c in format!("{:?}", d).chars() {
            self.push(c)?;
        }
        Ok(())
    }

    /// Handles a key event and updates the buffer and cursor accordingly.
    ///
    /// This function processes key events to perform actions such as
    /// moving the cursor, editing text, and handling special keys.
    ///
    /// # Parameters
    ///
    /// - `evt`: The key event to handle.
    ///
    /// # Returns
    ///
    /// A `Result` containing a byte representing the key action or a `RuChatError`.
    pub(crate) fn handle_key_event(&mut self, evt: Event) -> Result<u8, RuChatError> {
        match evt {
            Event::Key(KeyEvent {
                code: KeyCode::Backspace,
                ..
            }) => {
                // FIXME: the terminal backspace key is not working
                self.backspace()?;
            }
            Event::Key(KeyEvent {
                code: key,
                modifiers: KeyModifiers::NONE,
                ..
            }) => match key {
                KeyCode::Backspace => self.backspace()?,
                KeyCode::Delete => self.delete()?,
                KeyCode::Enter => return Ok(b'\n'),
                KeyCode::Left => self.move_left()?,
                KeyCode::Right => self.move_right()?,
                KeyCode::Up => self.move_up()?,
                KeyCode::Down => self.move_down()?,
                KeyCode::Home => self.cursor.0 = 0,
                KeyCode::End => self.cursor.0 = self.line_len()?,
                KeyCode::Esc => return Ok(b'q'),
                KeyCode::Char(c) => self.push(c)?,
                _ => {}
            },
            Event::Key(KeyEvent {
                code: key,
                modifiers: KeyModifiers::SHIFT,
                ..
            }) => match key {
                KeyCode::Left => self.move_word_left()?,
                KeyCode::Right => self.move_word_right()?,
                KeyCode::Enter => self.enter(),
                KeyCode::Char(c) => self.push(c.to_ascii_uppercase())?,
                _ => {}
            },
            Event::Key(KeyEvent {
                modifiers: KeyModifiers::CONTROL,
                code: key,
                ..
            }) => match key {
                KeyCode::Char('h') => self.backspace()?,
                KeyCode::Char('w') => self.delete_word()?,
                KeyCode::Char('a') => self.cursor.0 = 0,
                KeyCode::Char('e') => self.cursor.0 = self.line_len()?,
                KeyCode::Char('d') => self.delete()?,
                KeyCode::Char('n') => self.move_down()?,
                KeyCode::Char('p') => self.move_up()?,
                KeyCode::Char('b') => self.move_left()?,
                KeyCode::Char('f') => self.move_right()?,
                KeyCode::Char('c') => return Ok(b'q'),
                _ => {}
            },
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(_),
                column,
                row,
                ..
            }) => {
                let row = row as usize;
                let column = column as usize;
                if row < self.buffer.len() && column < self.buffer[row].len() + 1 {
                    self.cursor = (column, row);
                }
            }
            /*Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..
            }) => {
                self.cursor.1 = self.cursor.1.saturating_sub(1);
            }
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                ..
            }) => {
                self.cursor.1 += 1;
            }*/
            x => {
                //self.debug(x)?;
            }
        }
        Ok(b'\0')
    }

    /// Drains the buffer and returns its contents.
    ///
    /// This function clears the buffer and returns its contents as a vector
    /// of strings.
    ///
    /// # Returns
    ///
    /// A `Vec<String>` containing the contents of the buffer.
    pub(crate) fn drain(&mut self) -> Vec<String> {
        let buffer = self.buffer.clone();
        self.buffer = vec![String::new()];
        self.cursor = (0, 0);
        buffer
    }

    /// Sets the cursor position.
    ///
    /// # Parameters
    ///
    /// - `cursor`: The new cursor position as a tuple of (column, row).
    pub(crate) fn set_cursor(&mut self, cursor: (usize, usize)) {
        self.cursor = cursor;
    }

    /// Gets the current cursor position.
    ///
    /// # Returns
    ///
    /// A tuple of (column, row) representing the current cursor position.
    pub(crate) fn get_cursor(&self) -> (usize, usize) {
        self.cursor
    }

    fn len(&self) -> usize {
        self.buffer.iter().map(|line| line.len()).sum::<usize>() + self.buffer.len() - 1
    }

    fn line_len(&self) -> Result<usize, RuChatError> {
        Ok(self
            .buffer
            .get(self.cursor.1)
            .ok_or(RuChatError::Cursor1OutOfBounds)?
            .len())
    }

    fn cursor_position(&self) -> usize {
        self.buffer
            .iter()
            .take(self.cursor.1)
            .map(|line| line.len())
            .sum::<usize>()
            + self.cursor.0
    }

    fn write(&mut self, line: &str) {
        self.buffer = line.lines().map(|line| line.to_string()).collect();
        self.cursor = (0, 0);
    }

    fn push(&mut self, c: char) -> Result<(), RuChatError> {
        self.buffer
            .get_mut(self.cursor.1)
            .ok_or(RuChatError::Cursor1OutOfBounds)?
            .insert(self.cursor.0, c);
        self.cursor.0 += 1;
        Ok(())
    }

    fn append_line(&mut self, line: &str) -> Result<(), RuChatError> {
        *self
            .buffer
            .get_mut(self.cursor.1)
            .ok_or(RuChatError::Cursor1OutOfBounds)? += line;
        Ok(())
    }

    fn pop(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 > 0 {
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .remove(self.cursor.0 - 1);
            self.cursor.0 -= 1;
        } else if self.cursor.1 > 0 {
            let line = self.buffer.remove(self.cursor.1);
            self.cursor.1 -= 1;
            self.cursor.0 = self.line_len()?;
            self.append_line(&line)?;
        }
        Ok(())
    }

    fn backspace(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 > 0 {
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .remove(self.cursor.0 - 1);
            self.cursor.0 -= 1;
        } else if self.cursor.1 > 0 {
            let line = self.buffer.remove(self.cursor.1);
            self.cursor.1 -= 1;
            self.cursor.0 = self.line_len()?;
            self.append_line(&line)?;
        }
        Ok(())
    }
    fn delete_word(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 > 0 {
            let mut i = self.cursor.0;
            while i > 0 && self.is_whitespace(i - 1)? {
                i -= 1;
            }
            while i > 0 && !self.is_whitespace(i - 1)? {
                i -= 1;
            }
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .drain(i..self.cursor.0);
            self.cursor.0 = i;
        } else if self.cursor.1 > 0 {
            let line = self.buffer.remove(self.cursor.1);
            self.cursor.1 -= 1;
            self.cursor.0 = self.line_len()?;
            self.append_line(&line)?;
        }
        Ok(())
    }

    fn is_whitespace(&self, i: usize) -> Result<bool, RuChatError> {
        Ok(self
            .buffer
            .get(self.cursor.1)
            .ok_or(RuChatError::Cursor1OutOfBounds)?
            .chars()
            .nth(i)
            .ok_or(RuChatError::Cursor0OutOfBounds)?
            .is_whitespace())
    }

    fn move_word_left(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 > 0 {
            let mut i = self.cursor.0;
            while i > 0 && self.is_whitespace(i - 1)? {
                i -= 1;
            }
            while i > 0 && !self.is_whitespace(i - 1)? {
                i -= 1;
            }
            self.cursor.0 = i;
        } else if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
            self.cursor.0 = self.line_len()?;
        }
        Ok(())
    }

    fn move_word_right(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 < self.line_len()? {
            let mut i = self.cursor.0;
            while i < self.line_len()? && self.is_whitespace(i)? {
                i += 1;
            }
            while i < self.line_len()? && !self.is_whitespace(i)? {
                i += 1;
            }
            self.cursor.0 = i;
        } else if self.cursor.1 < self.buffer.len() - 1 {
            self.cursor.1 += 1;
            self.cursor.0 = 0;
        }
        Ok(())
    }

    fn delete(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 < self.line_len()? {
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .remove(self.cursor.0);
        } else if self.cursor.1 < self.buffer.len() - 1 {
            let line = self.buffer.remove(self.cursor.1 + 1);
            self.append_line(&line)?;
        }
        Ok(())
    }

    fn enter(&mut self) {
        self.buffer.insert(self.cursor.1 + 1, String::new());
        self.cursor.1 += 1;
        self.cursor.0 = 0;
    }

    fn move_left(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
        } else if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
            self.cursor.0 = self.line_len()?;
        }
        Ok(())
    }

    fn move_right(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 < self.line_len()? {
            self.cursor.0 += 1;
        } else if self.cursor.1 < self.buffer.len() - 1 {
            self.cursor.1 += 1;
            self.cursor.0 = 0;
        }
        Ok(())
    }

    fn move_up(&mut self) -> Result<(), RuChatError> {
        if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
            self.cursor.0 = self.cursor.0.min(self.line_len()?);
        }
        Ok(())
    }

    fn move_down(&mut self) -> Result<(), RuChatError> {
        if self.cursor.1 < self.buffer.len() - 1 {
            self.cursor.1 += 1;
            self.cursor.0 = self.cursor.0.min(self.line_len()?);
        }
        Ok(())
    }
}
