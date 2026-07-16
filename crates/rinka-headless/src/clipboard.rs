//! Deterministic in-memory clipboard for consumer tests.

use rinka_core::{Clipboard, ClipboardError, ClipboardService};
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Fake clipboard whose state is shared by every clone.
///
/// Reads deliver synchronously, exercising the same completion path a real
/// synchronous platform (macOS) takes; the runtimes' queued delivery turns
/// the completion into an ordinary follow-up message.
#[derive(Clone, Default)]
pub struct FakeClipboard {
    text: Rc<RefCell<Option<String>>>,
}

impl FakeClipboard {
    /// Creates an empty clipboard.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current clipboard text for assertions.
    pub fn text(&self) -> Option<String> {
        self.text.borrow().clone()
    }

    /// Wraps this fake in the core service handle.
    pub fn handle(&self) -> Clipboard {
        Clipboard::new(self.clone())
    }
}

impl ClipboardService for FakeClipboard {
    fn write_text(&self, text: &str) -> Result<(), ClipboardError> {
        *self.text.borrow_mut() = Some(text.to_owned());
        Ok(())
    }

    fn read_text(&self, deliver: Box<dyn FnOnce(Result<Option<String>, ClipboardError>)>) {
        deliver(Ok(self.text.borrow().clone()));
    }
}

impl fmt::Debug for FakeClipboard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FakeClipboard")
            .field("has_text", &self.text.borrow().is_some())
            .finish()
    }
}
