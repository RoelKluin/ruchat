use crate::error::RuChatError;
use crate::ollama::chat::event_result::EventResult;
use crate::ollama::chat::history::{Edit, EditKind, History};
use crate::ollama::chat::pos::Pos;
use crossterm::terminal;
use crossterm::{
    cursor::{DisableBlinking, EnableBlinking},
    event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind},
    execute,
    terminal::ClearType,
};
use std::io;

/// A struct for managing a buffer cursor in a text-based interface.
///
/// This struct provides methods for handling key events, navigating
/// the buffer, and editing text.
pub(crate) struct BufCursor {
    history: History,
    buffer: Vec<String>,
    cursor: (usize, usize),
    copy_buffer: Vec<String>,
    selection_start: Option<(usize, usize)>,
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
            history: History::new(50),
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

    pub(crate) fn get_line(&self, line: usize) -> Result<&str, RuChatError> {
        self.buffer
            .get(line)
            .ok_or(RuChatError::Cursor1OutOfBounds)
            .map(|s| s.as_str())
    }

    /// Handles an event and updates the buffer and cursor accordingly.
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
    pub(crate) fn handle_event(&mut self, evt: Event) -> Result<EventResult, RuChatError> {
        match evt {
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::ALT,
                ..
            }) => Ok(EventResult::Submit),
            Event::Key(KeyEvent {
                code: key,
                modifiers,
                ..
            }) if modifiers == KeyModifiers::NONE || modifiers == KeyModifiers::SHIFT => {
                match key {
                    KeyCode::Esc => Ok(EventResult::Quit),
                    KeyCode::Backspace => self.backspace(),
                    KeyCode::Delete => self.delete(),
                    KeyCode::End => self.move_to_end_of_line(modifiers),
                    KeyCode::Home => self.move_to_start_of_line(modifiers),
                    _ => {
                        match key {
                            KeyCode::Char(c) if modifiers == KeyModifiers::NONE => self.push(c),
                            KeyCode::Char(c) => self.push(c.to_ascii_uppercase()),
                            KeyCode::Enter => self.enter(), // + SHIFT is disfunctional, use ALT.
                            KeyCode::Down => self.move_down(modifiers),
                            KeyCode::Left => self.move_left(modifiers),
                            KeyCode::Right => self.move_right(modifiers),
                            KeyCode::Up => self.move_up(modifiers),
                            _ => Ok(EventResult::UnhandledEvent(evt)),
                        }
                    }
                }
            }
            Event::Key(KeyEvent {
                code: key,
                modifiers,
                ..
            }) if modifiers == KeyModifiers::CONTROL
                || modifiers == (KeyModifiers::SHIFT | KeyModifiers::CONTROL) =>
            {
                match key {
                    KeyCode::Char('z') => self.undo(),
                    KeyCode::Char('r') => self.redo(),
                    _ => match key {
                        KeyCode::Char('a') => self.select_all(),
                        KeyCode::Char('c') => self.copy(),
                        KeyCode::Char('d') => self.delete(),
                        KeyCode::Char('h') => self.backspace(),
                        KeyCode::Char('n') => self.move_down(modifiers),
                        KeyCode::Char('p') => self.move_up(modifiers),
                        KeyCode::Char('v') => self.paste(),
                        KeyCode::Char('x') => self.cut(),
                        KeyCode::Delete | KeyCode::Char('w') => self.delete_word(),
                        KeyCode::Left | KeyCode::Char('b') => self.move_word_left(modifiers),
                        KeyCode::Right | KeyCode::Char('f') => self.move_word_right(modifiers),
                        _ => Ok(EventResult::UnhandledEvent(evt)),
                    },
                }
            }
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(_),
                column,
                row,
                ..
            }) => self.set_cursor(column as usize, row as usize),
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                modifiers: m,
                ..
            }) => self.move_up(m),
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                modifiers: m,
                ..
            }) => self.move_down(m),
            _ => Ok(EventResult::UnhandledEvent(evt)),
        }
    }

    fn push_history(&mut self, kind: EditKind, before: Pos, after_offset: usize) {
        let (col, row) = self.cursor;
        let after = Pos::new(col, row, after_offset);
        let edit = Edit::new(kind, before, after);
        self.history.push(edit);
    }

    fn ammend_selection(&mut self, m: KeyModifiers, ct: ClearType) -> EventResult {
        if (m & KeyModifiers::SHIFT) != KeyModifiers::NONE {
            if self.selection_start.is_none() {
                execute!(io::stdout(), DisableBlinking).unwrap();
                self.selection_start = Some(self.cursor)
            }
            return EventResult::UpdateView(ct);
        } else if self.selection_start.is_some() {
            execute!(io::stdout(), EnableBlinking).unwrap();
            self.selection_start = None;
        }
        EventResult::CursorChange
    }

    fn delete_word(&mut self) -> Result<EventResult, RuChatError> {
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
            Ok(EventResult::ct(ClearType::UntilNewLine))
        } else if self.cursor.1 < self.buffer.len() - 1 {
            let line = self.buffer.remove(self.cursor.1 + 1);
            self.append_line(&line)
        } else {
            Ok(EventResult::Unchanged)
        }
    }

    fn select_all(&mut self) -> Result<EventResult, RuChatError> {
        let end_pos = (self.line_len().unwrap(), self.buffer.len() - 1);
        if self.selection_start != Some((0, 0)) || self.cursor != end_pos {
            self.selection_start = Some((0, 0));
            self.cursor = end_pos;
            Ok(EventResult::ct(ClearType::All))
        } else {
            Ok(EventResult::Unchanged)
        }
    }
    fn copy(&mut self) -> Result<EventResult, RuChatError> {
        if let Some(start) = self.selection_start {
            let start = start.0.min(self.cursor.0);
            let end = self.cursor.0.max(self.selection_start.unwrap().0);
            self.copy_buffer = self
                .buffer
                .get(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)
                .map(|line| line[start..end].to_string())
                .map(|s| vec![s])
                .unwrap_or_default();
        }
        Ok(EventResult::Unchanged)
    }
    fn cut(&mut self) -> Result<EventResult, RuChatError> {
        _ = self.copy()?;
        self.delete()
    }
    fn paste(&mut self) -> Result<EventResult, RuChatError> {
        if let Some(line) = self.copy_buffer.pop() {
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .insert_str(self.cursor.0, &line);
            self.cursor.0 += line.len();
        }
        Ok(EventResult::ct(ClearType::UntilNewLine))
    }

    /// Undo the last modification. This method returns if the undo modified text contents or not in the chat.
    pub fn undo(&mut self) -> Result<EventResult, RuChatError> {
        if let Some((cursor, ct)) = self.history.undo(&mut self.buffer) {
            self.selection_start = None;
            self.cursor = cursor;
            Ok(EventResult::UpdateView(ct))
        } else {
            Ok(EventResult::Unchanged)
        }
    }

    /// Redo the last undo change. This method returns if the redo modified text contents or not in the chat.
    pub fn redo(&mut self) -> Result<EventResult, RuChatError> {
        if let Some((cursor, ct)) = self.history.redo(&mut self.buffer) {
            self.selection_start = None;
            self.cursor = cursor;
            Ok(EventResult::UpdateView(ct))
        } else {
            Ok(EventResult::Unchanged)
        }
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
    pub(crate) fn set_cursor(
        &mut self,
        column: usize,
        row: usize,
    ) -> Result<EventResult, RuChatError> {
        if row < self.buffer.len() {
            if column < self.buffer[row].len() + 1 {
                self.cursor = (column, row);
                Ok(EventResult::CursorChange)
            } else {
                Err(RuChatError::Cursor0OutOfBounds)
            }
        } else {
            Err(RuChatError::Cursor1OutOfBounds)
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

    fn push(&mut self, c: char) -> Result<EventResult, RuChatError> {
        let (col, row) = self.cursor;
        let line = &mut self.buffer[row];
        let i = line
            .char_indices()
            .nth(col)
            .map(|(i, _)| i)
            .unwrap_or(line.len());
        line.insert(self.cursor.0, c);
        self.cursor.0 += 1;
        self.push_history(
            EditKind::InsertChar(c),
            Pos::new(col, row, i),
            i + c.len_utf8(),
        );
        Ok(EventResult::ct(ClearType::UntilNewLine))
    }

    fn line_len(&self) -> Result<usize, RuChatError> {
        Ok(self
            .buffer
            .get(self.cursor.1)
            .ok_or(RuChatError::Cursor1OutOfBounds)?
            .len())
    }

    fn append_line(&mut self, line: &str) -> Result<EventResult, RuChatError> {
        *self
            .buffer
            .get_mut(self.cursor.1)
            .ok_or(RuChatError::Cursor1OutOfBounds)? += line;
        Ok(EventResult::ct(ClearType::FromCursorDown))
    }

    fn backspace(&mut self) -> Result<EventResult, RuChatError> {
        if self.cursor.0 > 0 {
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .remove(self.cursor.0 - 1);
            self.cursor.0 -= 1;
            Ok(EventResult::ct(ClearType::CurrentLine))
        } else {
            Ok(EventResult::Unchanged)
        }
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

    fn move_word_left(&mut self, m: KeyModifiers) -> Result<EventResult, RuChatError> {
        if self.cursor.0 > 0 {
            let res = self.ammend_selection(m, ClearType::CurrentLine);
            let mut i = self.cursor.0;
            while i > 0 && self.is_whitespace(i - 1)? {
                i -= 1;
            }
            while i > 0 && !self.is_whitespace(i - 1)? {
                i -= 1;
            }
            self.cursor.0 = i;
            Ok(res)
        } else {
            Ok(EventResult::Unchanged)
        }
    }

    fn move_word_right(&mut self, m: KeyModifiers) -> Result<EventResult, RuChatError> {
        if self.cursor.0 < self.line_len()? {
            let res = self.ammend_selection(m, ClearType::CurrentLine);
            let mut i = self.cursor.0;
            while i < self.line_len()? && self.is_whitespace(i)? {
                i += 1;
            }
            while i < self.line_len()? && !self.is_whitespace(i)? {
                i += 1;
            }
            self.cursor.0 = i;
            Ok(res)
        } else {
            Ok(EventResult::Unchanged)
        }
    }

    fn move_to_end_of_line(&mut self, m: KeyModifiers) -> Result<EventResult, RuChatError> {
        let mut res = EventResult::Unchanged;
        if self.cursor.1 < self.buffer.len() - 1 {
            let res = self.ammend_selection(m, ClearType::CurrentLine);
            self.cursor.0 = self.line_len()?;
            Ok(res)
        } else {
            Ok(EventResult::Unchanged)
        }
    }
    fn move_to_start_of_line(&mut self, m: KeyModifiers) -> Result<EventResult, RuChatError> {
        let mut res = EventResult::Unchanged;
        if self.cursor.1 > 0 {
            let res = self.ammend_selection(m, ClearType::CurrentLine);
            self.cursor.0 = 0;
            Ok(res)
        } else {
            Ok(EventResult::Unchanged)
        }
    }

    fn delete(&mut self) -> Result<EventResult, RuChatError> {
        if let Some(start) = self.selection_start {
            let res = if self.cursor.0 == start.0 {
                EventResult::UpdateView(ClearType::UntilNewLine)
            } else {
                EventResult::UpdateView(ClearType::FromCursorDown)
            };
            let start = start.0.min(self.cursor.0);
            let end = self.cursor.0.max(self.selection_start.unwrap().0);
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .drain(start..end);
            self.cursor.0 = start;
            self.selection_start = None;
            Ok(res)
        } else if self.cursor.0 < self.line_len()? {
            self.buffer
                .get_mut(self.cursor.1)
                .ok_or(RuChatError::Cursor1OutOfBounds)?
                .remove(self.cursor.0);
            Ok(EventResult::ct(ClearType::UntilNewLine))
        } else if self.cursor.1 < self.buffer.len() - 1 {
            let line = self.buffer.remove(self.cursor.1 + 1);
            self.append_line(&line)
        } else {
            Ok(EventResult::Unchanged)
        }
    }

    fn enter(&mut self) -> Result<EventResult, RuChatError> {
        self.buffer.insert(self.cursor.1 + 1, String::new());
        self.cursor.1 += 1;
        self.cursor.0 = 0;
        Ok(EventResult::ct(ClearType::FromCursorDown))
    }

    fn move_left(&mut self, m: KeyModifiers) -> Result<EventResult, RuChatError> {
        if self.cursor.0 > 0 {
            let res = self.ammend_selection(m, ClearType::CurrentLine);
            self.cursor.0 -= 1;
            Ok(res)
        } else {
            Ok(EventResult::Unchanged)
        }
    }

    fn move_right(&mut self, m: KeyModifiers) -> Result<EventResult, RuChatError> {
        if self.cursor.0 < self.line_len()? {
            let res = self.ammend_selection(m, ClearType::CurrentLine);
            self.cursor.0 += 1;
            Ok(res)
        } else {
            Ok(EventResult::Unchanged)
        }
    }

    fn move_up(&mut self, m: KeyModifiers) -> Result<EventResult, RuChatError> {
        if self.cursor.1 > 0 {
            let res = self.ammend_selection(m, ClearType::FromCursorDown);
            self.cursor.1 -= 1;
            self.cursor.0 = self.cursor.0.min(self.line_len()?);
            Ok(res)
        } else {
            Ok(EventResult::Unchanged)
        }
    }

    fn move_down(&mut self, m: KeyModifiers) -> Result<EventResult, RuChatError> {
        if self.cursor.1 < self.buffer.len() - 1 {
            let res = self.ammend_selection(m, ClearType::FromCursorUp);
            self.cursor.1 += 1;
            self.cursor.0 = self.cursor.0.min(self.line_len()?);
            Ok(res)
        } else {
            Ok(EventResult::Unchanged)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, MouseButton, MouseEventKind};
    fn write(cursor: &mut BufCursor, text: &str) {
        for c in text.chars() {
            cursor.push(c).unwrap();
        }
    }

    #[test]
    fn test_bufcursor_new() {
        let cursor = BufCursor::new().unwrap();
        assert_eq!(cursor.buffer, vec![String::new()]);
        assert_eq!(cursor.cursor, (0, 0));
    }

    #[test]
    fn test_bufcursor_read() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        assert_eq!(cursor.read(), "Hello, world!");
        cursor.cursor = (5, 0);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::SHIFT,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('x'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        cursor.push(' ').unwrap();
        for c in "mister".chars() {
            cursor
                .handle_event(Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }))
                .unwrap();
        }
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('v'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello mister, world!");
    }

    #[test]
    fn test_bufcursor_view_buffer() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        assert_eq!(cursor.view_buffer(), vec!["Hello, world!\r"]);
    }

    #[test]
    fn test_bufcursor_set_cursor() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.set_cursor(5, 0);
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_handle_event() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        let event = Event::Key(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        cursor.handle_event(event).unwrap();
        assert_eq!(cursor.read(), "Hello, world!a");
    }

    #[test]
    fn test_bufcursor_backspace() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor.backspace().unwrap();
        assert_eq!(cursor.read(), "Hell, world!");
    }

    #[test]
    fn test_bufcursor_delete() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor.delete().unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_move_left() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor.move_left(KeyModifiers::NONE).unwrap();
        assert_eq!(cursor.get_cursor(), (4, 0));
    }

    #[test]
    fn test_bufcursor_move_right() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor.move_right(KeyModifiers::NONE).unwrap();
        assert_eq!(cursor.get_cursor(), (6, 0));
    }

    #[test]
    fn test_bufcursor_move_up() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!\nThis is a test.");
        cursor.cursor = (5, 1);
        cursor.move_up(KeyModifiers::NONE).unwrap();
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_move_down() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!\nThis is a test.");
        cursor.cursor = (5, 0);
        cursor.move_down(KeyModifiers::NONE).unwrap();
        assert_eq!(cursor.get_cursor(), (5, 1));
    }

    #[test]
    fn test_bufcursor_move_word_left() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor.move_word_left(KeyModifiers::NONE).unwrap();
        assert_eq!(cursor.get_cursor(), (4, 0));
    }

    #[test]
    fn test_bufcursor_move_word_right() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor.move_word_right(KeyModifiers::NONE).unwrap();
        assert_eq!(cursor.get_cursor(), (6, 0));
    }

    #[test]
    fn test_bufcursor_delete_word() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor.delete_word().unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_push() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor.push('a').unwrap();
        assert_eq!(cursor.read(), "Helloa, world!");
    }

    #[test]
    fn test_bufcursor_append_line() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.append_line("This is a test.").unwrap();
        assert_eq!(cursor.read(), "Hello, world!This is a test.");
    }

    #[test]
    fn test_bufcursor_drain() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        let drained = cursor.drain();
        assert_eq!(drained, vec!["Hello, world!"]);
        assert_eq!(cursor.read(), "");
    }

    #[test]
    fn test_bufcursor_is_whitespace() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        assert_eq!(cursor.is_whitespace(5).unwrap(), false);
        assert_eq!(cursor.is_whitespace(6).unwrap(), true);
    }

    #[test]
    fn test_bufcursor_line_len() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        assert_eq!(cursor.line_len().unwrap(), 13);
    }

    #[test]
    fn test_bufcursor_get_cursor() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.set_cursor(5, 0);
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_handle_event_backspace() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hell, world!");
    }

    #[test]
    fn test_bufcursor_handle_event_delete() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Delete,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_handle_event_enter() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello, world!\n");
    }

    #[test]
    fn test_bufcursor_handle_event_move_left() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (4, 0));
    }

    #[test]
    fn test_bufcursor_handle_event_move_right() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (6, 0));
    }

    #[test]
    fn test_bufcursor_handle_event_move_up() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!\nThis is a test.");
        cursor.cursor = (5, 1);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_handle_event_move_down() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!\nThis is a test.");
        cursor.cursor = (5, 0);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (5, 1));
    }

    #[test]
    fn test_bufcursor_handle_event_move_word_left() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (4, 0));
    }

    #[test]
    fn test_bufcursor_handle_event_move_word_right() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (6, 0));
    }

    #[test]
    fn test_bufcursor_handle_event_delete_word() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_handle_event_select_all() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_selection_start(), Some((0, 0)));
    }

    #[test]
    fn test_bufcursor_handle_event_copy() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        // Select all
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.copy_buffer, vec!["Hello, world!"]);
    }

    #[test]
    fn test_bufcursor_handle_event_cut() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('x'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello world!");
    }

    #[test]
    fn test_bufcursor_handle_event_paste() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('v'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello, world!");
    }

    #[test]
    fn test_bufcursor_handle_event_delete_selection() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor.cursor = (5, 0);
        for _ in 0..=7 {
            cursor
                .handle_event(Event::Key(KeyEvent {
                    code: KeyCode::Right,
                    modifiers: KeyModifiers::CONTROL,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }))
                .unwrap();
        }

        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello!");
    }

    #[test]
    fn test_bufcursor_handle_event_set_cursor() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 5,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_handle_event_scroll_up() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (0, 0));
    }

    #[test]
    fn test_bufcursor_handle_event_scroll_down() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (0, 0));
    }

    #[test]
    fn test_bufcursor_handle_event_debug() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello, world!d");
    }

    #[test]
    fn test_bufcursor_handle_event_other() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('x'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello, world!x");
    }

    #[test]
    fn test_bufcursor_handle_event_invalid() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('z'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello, world!z");
    }

    #[test]
    fn test_bufcursor_handle_event_invalid_key() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char('z'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.read(), "Hello, world!z");
    }

    #[test]
    fn test_bufcursor_handle_event_invalid_mouse() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 5,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (5, 0));
    }

    #[test]
    fn test_bufcursor_handle_event_invalid_scroll() {
        let mut cursor = BufCursor::new().unwrap();
        write(&mut cursor, "Hello, world!");
        cursor
            .handle_event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }))
            .unwrap();
        assert_eq!(cursor.get_cursor(), (0, 0));
    }
}
