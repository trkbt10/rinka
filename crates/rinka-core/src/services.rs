//! Host-injected platform capability registry.

use crate::clipboard::Clipboard;
use crate::dialog::DialogService;
use crate::window_service::WindowService;
use std::fmt;
use std::rc::Rc;

/// Window-independent platform capabilities registered at mount.
///
/// A platform host constructs one registry and passes it to
/// [`crate::AppRuntime::mount`] or [`crate::WindowRuntime::mount`]; the
/// runtime hands it to [`crate::Component::update`] through
/// [`crate::UpdateContext`]. A capability the host does not provide is
/// registered as its typed-diagnostic implementation (for the clipboard,
/// [`Clipboard::unsupported`]) or left absent (for dialogs, surfacing as a
/// typed [`crate::DialogError::NoPresenter`]; for windows, as a typed
/// [`crate::WindowError::NoHost`]) — never a silent no-op — so update logic
/// observes an honest error instead of fake success.
#[derive(Clone)]
pub struct PlatformServices {
    clipboard: Clipboard,
    dialogs: Option<Rc<dyn DialogService>>,
    windows: Option<Rc<dyn WindowService>>,
}

impl PlatformServices {
    /// Creates a registry from a host's service implementations.
    pub fn new(clipboard: Clipboard) -> Self {
        Self {
            clipboard,
            dialogs: None,
            windows: None,
        }
    }

    /// Injects the host's window-modal dialog presenter.
    pub fn with_dialog_service(mut self, service: impl DialogService + 'static) -> Self {
        self.dialogs = Some(Rc::new(service));
        self
    }

    /// Injects the host's runtime window lifecycle service.
    pub fn with_window_service(mut self, service: impl WindowService + 'static) -> Self {
        self.windows = Some(Rc::new(service));
        self
    }

    /// Returns the clipboard service handle.
    pub fn clipboard(&self) -> &Clipboard {
        &self.clipboard
    }

    pub(crate) fn dialog_service(&self) -> Option<&Rc<dyn DialogService>> {
        self.dialogs.as_ref()
    }

    pub(crate) fn window_service(&self) -> Option<&Rc<dyn WindowService>> {
        self.windows.as_ref()
    }
}

impl Default for PlatformServices {
    /// Registry used where no platform host mounted the content, such as
    /// detached snapshots; every capability is the typed rejection.
    fn default() -> Self {
        Self {
            clipboard: Clipboard::unsupported("no platform service host"),
            dialogs: None,
            windows: None,
        }
    }
}

impl fmt::Debug for PlatformServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlatformServices")
            .field("clipboard", &self.clipboard)
            .field("dialogs", &self.dialogs.is_some())
            .field("windows", &self.windows.is_some())
            .finish()
    }
}
