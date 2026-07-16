//! Platform service registry supplied by a mounting host.

use crate::clipboard::Clipboard;
use std::fmt;

/// Window-independent platform capabilities registered at mount.
///
/// A platform host constructs one registry and passes it to
/// [`crate::AppRuntime::mount`] or [`crate::WindowRuntime::mount`]; the
/// runtime hands it to [`crate::Component::update`] through
/// [`crate::UpdateContext`]. A capability the host does not provide is
/// registered as its typed-diagnostic implementation (for the clipboard,
/// [`Clipboard::unsupported`]) — never a silent no-op — so update logic
/// observes an honest error instead of fake success.
#[derive(Clone)]
pub struct PlatformServices {
    clipboard: Clipboard,
}

impl PlatformServices {
    /// Creates a registry from a host's service implementations.
    pub fn new(clipboard: Clipboard) -> Self {
        Self { clipboard }
    }

    /// Returns the clipboard service handle.
    pub fn clipboard(&self) -> &Clipboard {
        &self.clipboard
    }
}

impl Default for PlatformServices {
    /// Registry used where no platform host mounted the content, such as
    /// detached snapshots; every capability is the typed rejection.
    fn default() -> Self {
        Self {
            clipboard: Clipboard::unsupported("no platform service host"),
        }
    }
}

impl fmt::Debug for PlatformServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlatformServices")
            .field("clipboard", &self.clipboard)
            .finish()
    }
}
