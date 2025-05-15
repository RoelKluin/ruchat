use crate::error::RuChatError;
use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyModifiers,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    MouseEvent, MouseEventKind
    }
};
use crossterm::terminal;

/// A struct for managing a buffer cursor in a text-based interface.
///
/// This struct provides methods for handling key events, navigating
/// the buffer, and editing text.
pub(crate) struct BufCursor {
    buffer: Vec<String>,
    cursor: (usize, usize),
    copy_buffer: Vec<String>,
    selection_start: Option<(usize, usize)>
}

impl BufCursor {
    /// Creates a new `BufCursor` instance.
    ///
    /// # Returns
    ///
    /// A new instance of `BufCursor` with an empty buffer and cursor
    /// positioned at the start.
    pub(crate) fn new() -> Result<Self, RuChatError> {
        Ok(Self {
            buffer: vec![String::new()],
            copy_buffer: vec![],
            cursor: (0, 0),
            selection_start: None,
        })
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
                code: KeyCode::Enter,
                modifiers: KeyModifiers::ALT,
                ..
            }) => return Ok(b'\n'),
            Event::Key(KeyEvent {
                code: KeyCode::Backspace,
                ..
            }) => self.backspace()?,
            Event::Key(KeyEvent {
                code: key,
                modifiers: m,
                ..
            }) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                if m == KeyModifiers::SHIFT {
                    if self.selection_start.is_none() {
                        self.selection_start = Some(self.cursor)
                    }
                } else {
                    self.selection_start = None;
                }
                match key {
                    KeyCode::Backspace => self.backspace()?,
                    KeyCode::Delete if self.selection_start.is_none() => self.delete()?,
                    KeyCode::Delete => self.delete_selection()?,
                    KeyCode::Enter => self.enter(), // + SHIFT is disfunctional, use ALT.
                    KeyCode::Left => self.move_left()?,
                    KeyCode::Right => self.move_right()?,
                    KeyCode::Up => self.move_up()?,
                    KeyCode::Down => self.move_down()?,
                    KeyCode::Home => self.cursor.0 = 0,
                    KeyCode::End => self.cursor.0 = self.line_len()?,
                    KeyCode::Esc => return Ok(b'q'),
                    KeyCode::Char(c) if m == KeyModifiers::NONE => self.push(c)?,
                    KeyCode::Char(c) => self.push(c.to_ascii_uppercase())?,
                    _ => {}
                }
            },
            Event::Key(KeyEvent {
                modifiers: m,
                code: key,
                ..
            }) if m == KeyModifiers::CONTROL || m == (KeyModifiers::SHIFT | KeyModifiers::CONTROL) => {
                if m == KeyModifiers::SHIFT {
                    if self.selection_start.is_none() {
                        self.selection_start = Some(self.cursor)
                    }
                } else {
                    self.selection_start = None;
                }
                match key {
                    KeyCode::Left => self.move_word_left()?,
                    KeyCode::Right => self.move_word_right()?,
                    KeyCode::Delete => self.delete_word()?,
                    KeyCode::Char('h') => self.backspace()?,
                    KeyCode::Char('w') => self.delete_word()?,
                    KeyCode::Char('d') => self.delete()?,
                    KeyCode::Char('n') => self.move_down()?,
                    KeyCode::Char('p') => self.move_up()?,
                    KeyCode::Char('b') => self.move_word_left()?,
                    KeyCode::Char('f') => self.move_word_right()?,
                    _ => {}
                }
            },
            Event::Key(KeyEvent {
                modifiers: m,
                code: key,
                ..
            }) if m == KeyModifiers::CONTROL || m == (KeyModifiers::SHIFT | KeyModifiers::CONTROL) => {
                match key {
                    KeyCode::Char('a') => self.select_all(),
                    KeyCode::Char('c') => self.copy(),
                    KeyCode::Char('x') => self.cut()?,
                    KeyCode::Char('v') => self.paste()?,
                    _ => {}
                }
            },
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(_),
                column,
                row,
                ..
            }) => {
                self.set_cursor(column as usize, row as usize);
            }
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..
            }) => {
                self.move_up()?;
            }
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                ..
            }) => {
                self.move_down()?;
            }
            x => {
                //self.debug(x)?;
            }
        }
        Ok(b'\0')
    }
    fn delete_word(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 < self.line_len()? {
            let mut i = self.cursor.0;
            while i < self.line_len()? && self.is_whitespace(i)? {
                i += 1;
            }
            while i < self.line_len()? && !self.is_whitespace(i)? {
                i += 1;
            }
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .drain(self.cursor.0..i);
        } else if self.cursor.1 < self.buffer.len() - 1 {
            let line = self.buffer.remove(self.cursor.1 + 1);
            self.append_line(&line)?;
        }
        Ok(())
    }

    fn select_all(&mut self) {
        self.selection_start = Some((0, 0));
        self.cursor = (self.line_len().unwrap(), self.buffer.len() - 1);
    }
    fn copy(&mut self) {
        if let Some(start) = self.selection_start {
            let start = start.0.min(self.cursor.0);
            let end = self.cursor.0.max(self.selection_start.unwrap().0);
            self.copy_buffer = self.buffer
                .get(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)
                .map(|line| line[start..end].to_string())
                .map(|s| vec![s])
                .unwrap_or_default();
        }
    }
    fn cut(&mut self) -> Result<(), RuChatError> {
        self.copy();
        self.delete_selection()?;
        Ok(())
    }
    fn paste(&mut self) -> Result<(), RuChatError> {
        if let Some(line) = self.copy_buffer.pop() {
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .insert_str(self.cursor.0, &line);
            self.cursor.0 += line.len();
        }
        Ok(())
    }

    fn delete_selection(&mut self) -> Result<(), RuChatError> {
        if let Some(start) = self.selection_start {
            let start = start.0.min(self.cursor.0);
            let end = self.cursor.0.max(self.selection_start.unwrap().0);
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .drain(start..end);
            self.cursor.0 = start;
            self.selection_start = None;
        }
        Ok(())
    }

    pub(crate) fn get_selection_start(&self) -> Option<(usize, usize)> {
        self.selection_start
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
    pub(crate) fn set_cursor(&mut self, column: usize, row: usize) {
        if row < self.buffer.len() && column < self.buffer[row].len() + 1 {
            self.cursor = (column, row);
        }
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

    fn backspace(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 > 0 {
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .remove(self.cursor.0 - 1);
            self.cursor.0 -= 1;
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
        }
        Ok(())
    }

    fn move_right(&mut self) -> Result<(), RuChatError> {
        if self.cursor.0 < self.line_len()? {
            self.cursor.0 += 1;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, MouseButton, MouseEventKind};

    #[test]
    fn test_bufcursor_new() {
        let cursor = BufCursor::new().unwrap();
        assert_eq!(cursor.buffer, vec![String::new()]);
        assert_eq!(cursor.cursor, (0, 0));
    }

    #[test]
    fn test_bufcursor_read() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        assert_eq!(cursor.read(), "Hello, world!");
    }

    #[test]
    fn test_bufcursor_view_buffer() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        assert_eq!(cursor.view_buffer(), vec!["Hello, world!\r"]);
    }

    #[test]
    fn test_bufcursor_set_cursor() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.set_cursor(5, 0);
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_handle_key_event() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        let event = Event::Key(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        cursor.handle_key_event(event).unwrap();
        assert_eq!(cursor.read(), "Hello, world!a");
    }

    #[test]
    fn test_bufcursor_backspace() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.backspace().unwrap();
        assert_eq!(cursor.read(), "Hell, world!");
    }

    #[test]
    fn test_bufcursor_delete() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.delete().unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_move_left() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.move_left().unwrap();
        assert_eq!(cursor.get_cursor(), (4, 0));
    }

    #[test]
    fn test_bufcursor_move_right() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.move_right().unwrap();
        assert_eq!(cursor.get_cursor(), (6, 0));
    }

    #[test]
    fn test_bufcursor_move_up() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!\nThis is a test.");
        cursor.cursor = (5, 1);
        cursor.move_up().unwrap();
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_move_down() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!\nThis is a test.");
        cursor.cursor = (5, 0);
        cursor.move_down().unwrap();
        assert_eq!(cursor.get_cursor(), (5, 1));
    }

    #[test]
    fn test_bufcursor_move_word_left() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.move_word_left().unwrap();
        assert_eq!(cursor.get_cursor(), (4, 0));
    }

    #[test]
    fn test_bufcursor_move_word_right() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.move_word_right().unwrap();
        assert_eq!(cursor.get_cursor(), (6, 0));
    }

    #[test]
    fn test_bufcursor_delete_word() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.delete_word().unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_push() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.push('a').unwrap();
        assert_eq!(cursor.read(), "Helloa, world!");
    }

    #[test]
    fn test_bufcursor_append_line() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.append_line("This is a test.").unwrap();
        assert_eq!(cursor.read(), "Hello, world!This is a test.");
    }

    #[test]
    fn test_bufcursor_drain() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        let drained = cursor.drain();
        assert_eq!(drained, vec!["Hello, world!"]);
        assert_eq!(cursor.read(), "");
    }

    #[test]
    fn test_bufcursor_is_whitespace() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        assert_eq!(cursor.is_whitespace(5).unwrap(), false);
        assert_eq!(cursor.is_whitespace(6).unwrap(), true);
    }

    #[test]
    fn test_bufcursor_line_len() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        assert_eq!(cursor.line_len().unwrap(), 13);
    }

    #[test]
    fn test_bufcursor_len() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        assert_eq!(cursor.len(), 13);
    }

    #[test]
    fn test_bufcursor_get_cursor() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.set_cursor(5, 0);
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_debug() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.debug("Debugging").unwrap();
        assert_eq!(cursor.read(), "Hello, world!Debugging");
    }

    #[test]
    fn test_bufcursor_handle_key_event_backspace() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Backspace,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hell, world!");
    }

    #[test]
    fn test_bufcursor_handle_key_event_delete() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Delete,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_handle_key_event_enter() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hello, world!\n");
    }

    #[test]
    fn test_bufcursor_handle_key_event_move_left() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Left,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (4, 0));
    }

    #[test]
    fn test_bufcursor_handle_key_event_move_right() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Right,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (6, 0));
    }

    #[test]
    fn test_bufcursor_handle_key_event_move_up() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!\nThis is a test.");
        cursor.cursor = (5, 1);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_handle_key_event_move_down() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!\nThis is a test.");
        cursor.cursor = (5, 0);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (5, 1));
    }

    #[test]
    fn test_bufcursor_handle_key_event_move_word_left() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (4, 0));
    }

    #[test]
    fn test_bufcursor_handle_key_event_move_word_right() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('f'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (6, 0));
    }

    #[test]
    fn test_bufcursor_handle_key_event_delete_word() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_handle_key_event_select_all() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_selection_start(), Some((0, 0)));
    }

    #[test]
    fn test_bufcursor_handle_key_event_copy() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.copy_buffer, vec!["Hello, world!"]);
    }

    #[test]
    fn test_bufcursor_handle_key_event_cut() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_handle_key_event_paste() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('v'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hello, world!");
    }

    #[test]
    fn test_bufcursor_handle_key_event_delete_selection() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.cursor = (5, 0);
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_handle_key_event_set_cursor() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_handle_key_event_scroll_up() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (0, 0));
    }

    #[test]
    fn test_bufcursor_handle_key_event_scroll_down() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (0, 0));
    }

    #[test]
    fn test_bufcursor_handle_key_event_debug() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hello, world!d");
    }

    #[test]
    fn test_bufcursor_handle_key_event_other() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hello, world!x");
    }

    #[test]
    fn test_bufcursor_handle_key_event_invalid() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('z'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hello, world!z");
    }

    #[test]
    fn test_bufcursor_handle_key_event_invalid_key() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Key(KeyEvent {
            code: KeyCode::Char('z'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.read(), "Hello, world!z");
    }

    #[test]
    fn test_bufcursor_handle_key_event_invalid_mouse() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_handle_key_event_invalid_scroll() {
        let mut cursor = BufCursor::new().unwrap();
        cursor.write("Hello, world!");
        cursor.handle_key_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }))
        .unwrap();
        assert_eq!(cursor.get_cursor(), (0, 0));
    }
}
