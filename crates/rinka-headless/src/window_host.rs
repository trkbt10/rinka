//! Deterministic window-set model for runtime window lifecycle tests.
//!
//! [`HeadlessWindowHost`] plays the role a platform application delegate
//! plays natively: it mounts each opened [`rinka_core::WindowSpec`] in its
//! own [`WindowRuntime`] over a fresh [`HeadlessBackend`], injects itself as
//! the [`WindowService`] of every window it mounts, models focus and
//! geometry, applies the reconciled window title after every render, and
//! implements the close-interception protocol documented in
//! `rinka_core::window_service` — including the pending-close token. Every
//! observable transition is recorded, in order, as a [`WindowOperation`], so
//! consumer tests assert the complete lifecycle deterministically.

use crate::HeadlessBackend;
use rinka_core::{
    EventBindings, PlatformServices, Props, RenderError, Renderer, Size, WindowBindings,
    WindowError, WindowEvent, WindowId, WindowKind, WindowPosition, WindowRuntime, WindowService,
    WindowSpec,
};
use std::cell::RefCell;
use std::fmt;
use std::rc::{Rc, Weak};

/// One recorded window-set transition.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowOperation {
    /// A window was realized.
    Opened(WindowId),
    /// A window became focused.
    Focused(WindowId),
    /// A window stopped being focused.
    Resigned(WindowId),
    /// A window's reconciled title changed.
    TitleChanged(WindowId, String),
    /// A window's content area was resized.
    Resized(WindowId, Size),
    /// A window was moved.
    Moved(WindowId, WindowPosition),
    /// A user-gesture close was deferred and its token retained.
    CloseRequested(WindowId),
    /// A pending close was answered with a veto.
    CloseVetoed(WindowId),
    /// A pending close was answered with a confirmation.
    CloseConfirmed(WindowId),
    /// A window was torn down.
    Closed(WindowId),
    /// The last open window closed; the platform policy would apply here.
    AllWindowsClosed,
}

/// Result of simulating one user-gesture close.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseRequestOutcome {
    /// No close-request handler was declared; the window closed natively.
    ClosedImmediately,
    /// The close was deferred: a token is pending and the close-request
    /// message was dispatched. (The component may already have answered
    /// synchronously by the time this returns.)
    Deferred,
    /// A token was already pending; the gesture was absorbed without a
    /// second message.
    AlreadyPending,
}

struct HostedWindow {
    id: WindowId,
    kind: WindowKind,
    title: String,
    content_size: Size,
    position: WindowPosition,
    runtime: WindowRuntime<HeadlessBackend>,
    bindings: WindowBindings,
}

struct HostInner {
    self_weak: RefCell<Weak<HostInner>>,
    build_services: RefCell<Rc<dyn Fn() -> PlatformServices>>,
    windows: RefCell<Vec<HostedWindow>>,
    focused: RefCell<Option<WindowId>>,
    pending_closes: RefCell<Vec<WindowId>>,
    operations: RefCell<Vec<WindowOperation>>,
}

/// Deterministic runtime window host for headless consumer tests.
///
/// Clones share the same window set. The host injects itself as the
/// [`WindowService`] of every window it mounts, so a component opened by the
/// host can itself open, close, focus, and answer close requests through
/// [`rinka_core::UpdateContext::windows`].
#[derive(Clone)]
pub struct HeadlessWindowHost {
    inner: Rc<HostInner>,
}

impl Default for HeadlessWindowHost {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessWindowHost {
    /// Creates a host whose windows receive the default (typed-rejection)
    /// service registry plus this host's window service.
    pub fn new() -> Self {
        let inner = Rc::new(HostInner {
            self_weak: RefCell::new(Weak::new()),
            build_services: RefCell::new(Rc::new(PlatformServices::default)),
            windows: RefCell::new(Vec::new()),
            focused: RefCell::new(None),
            pending_closes: RefCell::new(Vec::new()),
            operations: RefCell::new(Vec::new()),
        });
        *inner.self_weak.borrow_mut() = Rc::downgrade(&inner);
        Self { inner }
    }

    /// Replaces the registry template cloned into every window this host
    /// mounts; the host appends its own window service to whatever the
    /// template builds.
    pub fn with_services(self, build: impl Fn() -> PlatformServices + 'static) -> Self {
        *self.inner.build_services.borrow_mut() = Rc::new(build);
        self
    }

    /// Returns the injectable window service backed by this host.
    pub fn service(&self) -> impl WindowService + 'static {
        WeakWindowService {
            inner: Rc::downgrade(&self.inner),
        }
    }

    /// Opens one window, exactly as a component's
    /// [`rinka_core::Windows::open`] would.
    pub fn open(&self, window: WindowSpec) -> Result<(), WindowError> {
        self.inner.open(window)
    }

    /// Simulates the user's native close gesture (close button, Cmd+W) on
    /// one window, honoring the close-interception protocol.
    pub fn request_close(&self, id: &WindowId) -> Result<CloseRequestOutcome, WindowError> {
        self.inner.request_close(id)
    }

    /// Simulates a native focus change, exactly as the platform's key-window
    /// transition would deliver it.
    pub fn focus(&self, id: &WindowId) -> Result<(), WindowError> {
        self.inner.focus(id)
    }

    /// Simulates a native content resize, delivering the lifecycle event.
    pub fn set_content_size(&self, id: &WindowId, size: Size) -> Result<(), WindowError> {
        self.inner.set_content_size(id, size)
    }

    /// Simulates a native window move, delivering the lifecycle event.
    pub fn set_position(&self, id: &WindowId, position: WindowPosition) -> Result<(), WindowError> {
        self.inner.set_position(id, position)
    }

    /// Returns the open window identities in realization order.
    pub fn open_ids(&self) -> Vec<WindowId> {
        self.inner
            .windows
            .borrow()
            .iter()
            .map(|window| window.id.clone())
            .collect()
    }

    /// Returns whether this identity is open.
    pub fn is_open(&self, id: &WindowId) -> bool {
        self.inner
            .windows
            .borrow()
            .iter()
            .any(|window| window.id == *id)
    }

    /// Returns the focused window identity.
    pub fn focused(&self) -> Option<WindowId> {
        self.inner.focused.borrow().clone()
    }

    /// Returns one window's current (reconciled) title.
    pub fn title_of(&self, id: &WindowId) -> Option<String> {
        self.inner
            .windows
            .borrow()
            .iter()
            .find(|window| window.id == *id)
            .map(|window| window.title.clone())
    }

    /// Returns one window's semantic kind.
    pub fn kind_of(&self, id: &WindowId) -> Option<WindowKind> {
        self.inner
            .windows
            .borrow()
            .iter()
            .find(|window| window.id == *id)
            .map(|window| window.kind)
    }

    /// Returns one window's modeled content size.
    pub fn content_size_of(&self, id: &WindowId) -> Option<Size> {
        self.inner
            .windows
            .borrow()
            .iter()
            .find(|window| window.id == *id)
            .map(|window| window.content_size)
    }

    /// Returns one window's modeled position.
    pub fn position_of(&self, id: &WindowId) -> Option<WindowPosition> {
        self.inner
            .windows
            .borrow()
            .iter()
            .find(|window| window.id == *id)
            .map(|window| window.position)
    }

    /// Returns the identities whose close is pending an answer.
    pub fn pending_close_ids(&self) -> Vec<WindowId> {
        self.inner.pending_closes.borrow().clone()
    }

    /// Returns every recorded transition in order.
    pub fn operations(&self) -> Vec<WindowOperation> {
        self.inner.operations.borrow().clone()
    }

    /// Returns the stable event target mounted under `key` in one window.
    ///
    /// The binding is cloned out, so events may be emitted after this call
    /// without holding any host borrow.
    pub fn events_of(&self, id: &WindowId, key: &str) -> Option<EventBindings> {
        self.inner
            .windows
            .borrow()
            .iter()
            .find(|window| window.id == *id)?
            .runtime
            .with_renderer(|renderer| {
                let backend = renderer.backend();
                backend
                    .find_by_key(key)
                    .and_then(|handle| backend.events_of(handle))
            })
    }

    /// Reads the mounted label text declared under `key` in one window.
    pub fn label_text(&self, id: &WindowId, key: &str) -> Option<String> {
        self.inner
            .windows
            .borrow()
            .iter()
            .find(|window| window.id == *id)?
            .runtime
            .with_renderer(|renderer| {
                let backend = renderer.backend();
                let handle = backend.find_by_key(key)?;
                match backend.props_of(handle)? {
                    Props::Label { text, .. } => Some(text.clone()),
                    _ => None,
                }
            })
    }

    /// Takes one window runtime's most recent typed error.
    pub fn take_error(&self, id: &WindowId) -> Option<RenderError<crate::HeadlessError>> {
        self.inner
            .windows
            .borrow()
            .iter()
            .find(|window| window.id == *id)?
            .runtime
            .take_error()
    }
}

impl fmt::Debug for HeadlessWindowHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HeadlessWindowHost")
            .field("open", &self.open_ids())
            .field("focused", &self.focused())
            .field("pending_closes", &self.pending_close_ids())
            .finish()
    }
}

/// Non-owning service handle injected into mounted windows, so the window
/// set never retains itself through its own components' service registries.
struct WeakWindowService {
    inner: Weak<HostInner>,
}

impl WeakWindowService {
    fn with_host<R>(
        &self,
        request: impl FnOnce(&Rc<HostInner>) -> Result<R, WindowError>,
    ) -> Result<R, WindowError> {
        match self.inner.upgrade() {
            Some(inner) => request(&inner),
            None => Err(WindowError::Host {
                reason: "the headless window host was dropped".to_owned(),
            }),
        }
    }
}

impl WindowService for WeakWindowService {
    fn open(&self, window: WindowSpec) -> Result<(), WindowError> {
        self.with_host(|inner| inner.open(window))
    }

    fn close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.with_host(|inner| inner.close(id))
    }

    fn focus(&self, id: &WindowId) -> Result<(), WindowError> {
        self.with_host(|inner| inner.focus(id))
    }

    fn set_content_size(&self, id: &WindowId, size: Size) -> Result<(), WindowError> {
        self.with_host(|inner| inner.set_content_size(id, size))
    }

    fn set_position(&self, id: &WindowId, position: WindowPosition) -> Result<(), WindowError> {
        self.with_host(|inner| inner.set_position(id, position))
    }

    fn confirm_close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.with_host(|inner| inner.confirm_close(id))
    }

    fn veto_close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.with_host(|inner| inner.veto_close(id))
    }
}

impl HostInner {
    fn record(&self, operation: WindowOperation) {
        self.operations.borrow_mut().push(operation);
    }

    fn bindings_of(&self, id: &WindowId) -> Option<WindowBindings> {
        self.windows
            .borrow()
            .iter()
            .find(|window| window.id == *id)
            .map(|window| window.bindings.clone())
    }

    fn open(&self, window: WindowSpec) -> Result<(), WindowError> {
        if self
            .windows
            .borrow()
            .iter()
            .any(|hosted| hosted.id == window.id)
        {
            return Err(WindowError::AlreadyOpen { id: window.id });
        }
        let services =
            (self.build_services.borrow().clone())().with_window_service(WeakWindowService {
                inner: self.self_weak.borrow().clone(),
            });
        let runtime = Renderer::new(HeadlessBackend::new());
        let runtime =
            WindowRuntime::mount(runtime, window.content.clone(), services).map_err(|error| {
                WindowError::Host {
                    reason: error.to_string(),
                }
            })?;
        let bindings = runtime.with_renderer(|renderer| renderer.window_bindings().clone());
        // The launch title yields to a declared title, exactly like the
        // native hosts, and the reconciled handler keeps following it.
        let title = bindings
            .declared_title()
            .unwrap_or_else(|| window.title.clone());
        let id = window.id.clone();
        {
            let weak = self.self_weak.borrow().clone();
            let handler_id = id.clone();
            let handler_bindings = bindings.clone();
            runtime.set_reconciled_handler(move || {
                if let Some(inner) = weak.upgrade() {
                    inner.refresh_title(&handler_id, &handler_bindings);
                }
            });
        }
        self.windows.borrow_mut().push(HostedWindow {
            id: id.clone(),
            kind: window.kind,
            title,
            content_size: window.initial_size,
            position: WindowPosition::new(0.0, 0.0),
            runtime,
            bindings,
        });
        self.record(WindowOperation::Opened(id.clone()));
        // A newly opened window becomes the focused window, as
        // makeKeyAndOrderFront does natively.
        self.focus_internal(&id);
        Ok(())
    }

    fn refresh_title(&self, id: &WindowId, bindings: &WindowBindings) {
        // A withdrawn declaration keeps the last native title, matching the
        // native hosts: retitling is a positive act, never an implicit reset.
        let Some(declared) = bindings.declared_title() else {
            return;
        };
        let changed = {
            let mut windows = self.windows.borrow_mut();
            let Some(window) = windows.iter_mut().find(|window| window.id == *id) else {
                return;
            };
            if window.title == declared {
                false
            } else {
                window.title = declared.clone();
                true
            }
        };
        if changed {
            self.record(WindowOperation::TitleChanged(id.clone(), declared));
        }
    }

    /// Moves focus to `id`, dispatching Resigned and Focused in the native
    /// order. Every host borrow is released before any handler runs.
    fn focus_internal(&self, id: &WindowId) {
        let previous = self.focused.borrow().clone();
        if previous.as_ref() == Some(id) {
            return;
        }
        *self.focused.borrow_mut() = Some(id.clone());
        if let Some(previous) = previous {
            self.record(WindowOperation::Resigned(previous.clone()));
            if let Some(bindings) = self.bindings_of(&previous) {
                bindings.dispatch_event(WindowEvent::Resigned);
            }
        }
        self.record(WindowOperation::Focused(id.clone()));
        if let Some(bindings) = self.bindings_of(id) {
            bindings.dispatch_event(WindowEvent::Focused);
        }
    }

    fn focus(&self, id: &WindowId) -> Result<(), WindowError> {
        if !self.windows.borrow().iter().any(|window| window.id == *id) {
            return Err(WindowError::NotOpen { id: id.clone() });
        }
        self.focus_internal(id);
        Ok(())
    }

    fn close(&self, id: &WindowId) -> Result<(), WindowError> {
        if !self.windows.borrow().iter().any(|window| window.id == *id) {
            return Err(WindowError::NotOpen { id: id.clone() });
        }
        // A programmatic close is unconditional: any pending token is
        // cleared, never answered.
        self.pending_closes
            .borrow_mut()
            .retain(|pending| pending != id);
        self.close_internal(id);
        Ok(())
    }

    /// Tears one window down and settles focus, as the native close path
    /// does after `windowWillClose`.
    fn close_internal(&self, id: &WindowId) {
        let removed = {
            let mut windows = self.windows.borrow_mut();
            let Some(index) = windows.iter().position(|window| window.id == *id) else {
                return;
            };
            windows.remove(index)
        };
        // The runtime (and its retained tree) is dropped after every borrow
        // is released, so a component closing its own window unwinds safely.
        drop(removed);
        self.record(WindowOperation::Closed(id.clone()));
        let was_focused = self.focused.borrow().as_ref() == Some(id);
        if was_focused {
            *self.focused.borrow_mut() = None;
            let next = self.windows.borrow().last().map(|window| window.id.clone());
            if let Some(next) = next {
                self.focus_internal(&next);
            }
        }
        if self.windows.borrow().is_empty() {
            self.record(WindowOperation::AllWindowsClosed);
        }
    }

    fn request_close(&self, id: &WindowId) -> Result<CloseRequestOutcome, WindowError> {
        let Some(bindings) = self.bindings_of(id) else {
            return Err(WindowError::NotOpen { id: id.clone() });
        };
        if !bindings.declares_close_request() {
            self.close_internal(id);
            return Ok(CloseRequestOutcome::ClosedImmediately);
        }
        if self.pending_closes.borrow().contains(id) {
            return Ok(CloseRequestOutcome::AlreadyPending);
        }
        self.pending_closes.borrow_mut().push(id.clone());
        self.record(WindowOperation::CloseRequested(id.clone()));
        bindings.dispatch_close_request();
        Ok(CloseRequestOutcome::Deferred)
    }

    fn take_pending(&self, id: &WindowId) -> Result<(), WindowError> {
        if !self.windows.borrow().iter().any(|window| window.id == *id) {
            return Err(WindowError::NotOpen { id: id.clone() });
        }
        let mut pending = self.pending_closes.borrow_mut();
        let Some(index) = pending.iter().position(|pending| pending == id) else {
            return Err(WindowError::NoPendingClose { id: id.clone() });
        };
        pending.remove(index);
        Ok(())
    }

    fn confirm_close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.take_pending(id)?;
        self.record(WindowOperation::CloseConfirmed(id.clone()));
        self.close_internal(id);
        Ok(())
    }

    fn veto_close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.take_pending(id)?;
        self.record(WindowOperation::CloseVetoed(id.clone()));
        Ok(())
    }

    fn set_content_size(&self, id: &WindowId, size: Size) -> Result<(), WindowError> {
        {
            let mut windows = self.windows.borrow_mut();
            let Some(window) = windows.iter_mut().find(|window| window.id == *id) else {
                return Err(WindowError::NotOpen { id: id.clone() });
            };
            window.content_size = size;
        }
        self.record(WindowOperation::Resized(id.clone(), size));
        if let Some(bindings) = self.bindings_of(id) {
            bindings.dispatch_event(WindowEvent::Resized(size));
        }
        Ok(())
    }

    fn set_position(&self, id: &WindowId, position: WindowPosition) -> Result<(), WindowError> {
        {
            let mut windows = self.windows.borrow_mut();
            let Some(window) = windows.iter_mut().find(|window| window.id == *id) else {
                return Err(WindowError::NotOpen { id: id.clone() });
            };
            window.position = position;
        }
        self.record(WindowOperation::Moved(id.clone(), position));
        if let Some(bindings) = self.bindings_of(id) {
            bindings.dispatch_event(WindowEvent::Moved(position));
        }
        Ok(())
    }
}
