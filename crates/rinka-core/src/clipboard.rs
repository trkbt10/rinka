//! Platform-neutral clipboard service contract.
//!
//! The clipboard is a window-independent platform *service*, not an element
//! mutation, so it lives beside [`crate::NativeBackend`] rather than inside
//! it: each platform host implements [`ClipboardService`] and registers the
//! handle in its [`crate::PlatformServices`] at mount. Components reach it
//! through the [`crate::UpdateContext`] passed to
//! [`crate::Component::update`].
//!
//! Reading is completion-based because GTK 4 can only read the clipboard
//! asynchronously. A synchronous platform (macOS, the headless fake) invokes
//! the completion before `read_text` returns; the component runtimes queue
//! messages emitted during a running update, so a synchronously delivered
//! completion becomes an ordinary follow-up message on every platform. The
//! delivery model is therefore uniform: the outcome always arrives at
//! `update` as a dispatched message.

use std::error::Error;
use std::fmt;
use std::rc::Rc;

/// Clipboard content representation vocabulary.
///
/// Plain text is the only implemented flavor. [`ClipboardFlavor::Files`] and
/// [`ClipboardFlavor::RichText`] are declared extension points reserved for a
/// future contract revision; no adapter implements them and the service
/// exposes no operation over them yet.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClipboardFlavor {
    /// Plain Unicode text.
    Text,
    /// File references (reserved extension point, unimplemented).
    Files,
    /// Styled text (reserved extension point, unimplemented).
    RichText,
}

impl fmt::Display for ClipboardFlavor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text => formatter.write_str("text"),
            Self::Files => formatter.write_str("files"),
            Self::RichText => formatter.write_str("rich text"),
        }
    }
}

/// Typed clipboard diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClipboardError {
    /// The mounting host provides no clipboard service.
    Unsupported {
        /// Host or adapter that rejected the capability.
        platform: &'static str,
    },
    /// The platform clipboard operation failed.
    Platform {
        /// Explanation captured at the failure site.
        reason: String,
    },
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { platform } => {
                write!(formatter, "{platform} does not provide a clipboard service")
            }
            Self::Platform { reason } => formatter.write_str(reason),
        }
    }
}

impl Error for ClipboardError {}

/// Platform clipboard operations over plain text.
///
/// Implementations must deliver a read outcome exactly once, and must reject
/// an unavailable capability with a typed [`ClipboardError`] — never a
/// silent no-op. The flavor vocabulary reserved for richer content is
/// [`ClipboardFlavor`].
pub trait ClipboardService {
    /// Replaces the clipboard contents with plain text.
    fn write_text(&self, text: &str) -> Result<(), ClipboardError>;

    /// Reads the clipboard's plain text.
    ///
    /// Invokes `deliver` exactly once with `Ok(None)` when the clipboard
    /// holds no text. A synchronous platform may invoke it before this call
    /// returns; an asynchronous platform invokes it later on the UI thread.
    fn read_text(&self, deliver: Box<dyn FnOnce(Result<Option<String>, ClipboardError>)>);
}

/// Cloneable clipboard handle registered in [`crate::PlatformServices`].
#[derive(Clone)]
pub struct Clipboard(Rc<dyn ClipboardService>);

impl Clipboard {
    /// Wraps one platform service implementation.
    pub fn new(service: impl ClipboardService + 'static) -> Self {
        Self(Rc::new(service))
    }

    /// Creates the typed-diagnostic handle for a host without a clipboard.
    ///
    /// Every operation returns [`ClipboardError::Unsupported`] naming the
    /// rejecting platform, mirroring the unsupported-element rule: a missing
    /// capability is an honest error, never a fake success.
    pub fn unsupported(platform: &'static str) -> Self {
        Self(Rc::new(UnsupportedClipboard { platform }))
    }

    /// Replaces the clipboard contents with plain text.
    pub fn write_text(&self, text: &str) -> Result<(), ClipboardError> {
        self.0.write_text(text)
    }

    /// Reads the clipboard's plain text, delivering the outcome exactly once.
    pub fn read_text(
        &self,
        deliver: impl FnOnce(Result<Option<String>, ClipboardError>) + 'static,
    ) {
        self.0.read_text(Box::new(deliver));
    }
}

impl fmt::Debug for Clipboard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Clipboard(..)")
    }
}

/// Typed rejection standing in for a clipboard a host does not provide.
struct UnsupportedClipboard {
    platform: &'static str,
}

impl ClipboardService for UnsupportedClipboard {
    fn write_text(&self, _text: &str) -> Result<(), ClipboardError> {
        Err(ClipboardError::Unsupported {
            platform: self.platform,
        })
    }

    fn read_text(&self, deliver: Box<dyn FnOnce(Result<Option<String>, ClipboardError>)>) {
        deliver(Err(ClipboardError::Unsupported {
            platform: self.platform,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::{Clipboard, ClipboardError};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn an_unsupported_clipboard_rejects_writes_with_the_platform_name() {
        let clipboard = Clipboard::unsupported("Test Host");
        assert_eq!(
            clipboard.write_text("payload"),
            Err(ClipboardError::Unsupported {
                platform: "Test Host",
            })
        );
    }

    #[test]
    fn an_unsupported_clipboard_delivers_the_typed_read_rejection_once() {
        let clipboard = Clipboard::unsupported("Test Host");
        let delivered = Rc::new(RefCell::new(Vec::new()));
        let sink = delivered.clone();
        clipboard.read_text(move |result| sink.borrow_mut().push(result));
        assert_eq!(
            *delivered.borrow(),
            vec![Err(ClipboardError::Unsupported {
                platform: "Test Host",
            })]
        );
    }

    #[test]
    fn diagnostics_name_the_rejecting_platform_and_failure_reason() {
        assert_eq!(
            ClipboardError::Unsupported {
                platform: "Windows WinUI 3",
            }
            .to_string(),
            "Windows WinUI 3 does not provide a clipboard service"
        );
        assert_eq!(
            ClipboardError::Platform {
                reason: "pasteboard rejected the write".to_owned(),
            }
            .to_string(),
            "pasteboard rejected the write"
        );
    }
}
