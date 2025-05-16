use crossterm::terminal::ClearType;
use crossterm::event::Event;

pub(crate) enum EventResult {
    CursorChange,
    /// quit the application
    Quit,
    /// submit prompt to the server
    Submit,
    UnhandledEvent(Event),
    UpdateView(ClearType),
    Unchanged,
}

impl EventResult {
    pub(crate) fn ct(ct: ClearType) -> Self {
        EventResult::UpdateView(ct)
    }
}
