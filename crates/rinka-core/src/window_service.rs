//! Runtime window lifecycle: the host-implemented window service, per-window
//! lifecycle events, and the reconciled window declaration slot.
//!
//! The window set is runtime state. A component opens, closes, and focuses
//! top-level windows through [`crate::UpdateContext::windows`], exactly like
//! dialogs: the host injects a [`WindowService`] at mount, every failure is
//! the typed [`WindowError`] recorded as [`crate::RenderError::Window`] on
//! the mounting runtime, and every lifecycle fact returns as an ordinary
//! component message.
//!
//! Window identity is the stable [`WindowId`]: a window opened at runtime
//! keeps its native realization across reconciles, and re-opening an
//! already-open identity is a typed error, never a silent focus or a second
//! native window.
//!
//! Two window properties differ deliberately in mutation style:
//!
//! - **Title is reconciled.** The content root declares
//!   [`crate::Element::window_title`]; the host applies the freshest
//!   declaration after every reconciliation, so the title is a pure function
//!   of component state like every other declared property.
//! - **Size and position are imperative.** The user mutates both
//!   continuously through native resize and move gestures, and there is no
//!   revision protocol (as controlled text has) to arbitrate ownership; a
//!   reconciled value would snap the window back under the user's pointer on
//!   every render. The service's one-shot setters keep the native shell the
//!   owner and the component an occasional requester.
//!
//! ## The close-interception protocol
//!
//! A user-gesture close (close button, Cmd+W, `performClose:`) of a window
//! whose content root declares [`crate::Element::on_close_request`] is
//! deferred, never performed directly:
//!
//! 1. The host refuses the native close and retains one **pending-close
//!    token** for the window's identity. A second gesture while the token is
//!    pending is absorbed — the open request stands, no second message.
//! 2. The declared close-request handler dispatches a component message.
//! 3. The component answers — synchronously or after any number of
//!    intervening updates (typically a confirmation sheet through
//!    [`crate::UpdateContext::dialogs`]) — with
//!    [`Windows::confirm_close`] (the token is consumed and the native
//!    window closes) or [`Windows::veto_close`] (the token is consumed and
//!    nothing else happens).
//! 4. Answering without a pending token is the typed
//!    [`WindowError::NoPendingClose`].
//!
//! A window whose root declares no close-request handler closes natively,
//! with no message and no token. [`Windows::close`] is unconditional: a
//! programmatic close is a component decision, so it never consults the
//! close-request handler and clears any pending token for that window.

use crate::window::{Size, WindowId, WindowSpec};
use std::cell::RefCell;
use std::error::Error;
use std::fmt;
use std::rc::Rc;

/// Top-level window position in the platform's screen coordinates.
///
/// The value is platform-native and documented per host (macOS reports the
/// frame origin in its bottom-left-origin global screen space); it exists to
/// carry moves and one-shot placement requests, not to abstract screen
/// geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowPosition {
    /// Horizontal screen coordinate.
    pub x: f64,
    /// Vertical screen coordinate.
    pub y: f64,
}

impl WindowPosition {
    /// Creates a position.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// One per-window lifecycle fact delivered as a component message.
///
/// Events reach the component through the content root's
/// [`crate::Element::on_window_event`] declaration. The close request is
/// deliberately not part of this enum: it carries a protocol obligation
/// (answer with confirm or veto) and is declared separately through
/// [`crate::Element::on_close_request`], so subscribing to notifications can
/// never accidentally intercept closing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowEvent {
    /// The window became the focused (key) window.
    Focused,
    /// The window stopped being the focused (key) window.
    Resigned,
    /// The window's content area was resized.
    Resized(Size),
    /// The window was moved; the position is the platform-native frame
    /// origin in screen coordinates.
    Moved(WindowPosition),
}

/// Runtime window lifecycle failure.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowError {
    /// An open request re-used an identity that is already open.
    ///
    /// This is a typed error, not a focus: silently focusing would hide a
    /// consumer identity bug, and a consumer that wants open-or-focus
    /// composes it explicitly from its own window state plus
    /// [`Windows::focus`].
    AlreadyOpen {
        /// Identity that is already realized.
        id: WindowId,
    },
    /// The addressed window identity is not open.
    NotOpen {
        /// Unknown identity.
        id: WindowId,
    },
    /// A close answer arrived without a pending close request.
    NoPendingClose {
        /// Identity whose close was answered.
        id: WindowId,
    },
    /// The mounting host has not installed a window service.
    NoHost,
    /// The host rejected the request with a platform diagnostic.
    Host {
        /// Human-readable host failure.
        reason: String,
    },
}

impl fmt::Display for WindowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyOpen { id } => {
                write!(formatter, "window '{}' is already open", id.as_str())
            }
            Self::NotOpen { id } => write!(formatter, "window '{}' is not open", id.as_str()),
            Self::NoPendingClose { id } => write!(
                formatter,
                "window '{}' has no pending close request",
                id.as_str()
            ),
            Self::NoHost => {
                formatter.write_str("the window host has not installed a window service")
            }
            Self::Host { reason } => write!(formatter, "window host failure: {reason}"),
        }
    }
}

impl Error for WindowError {}

/// Host-implemented runtime window lifecycle.
///
/// The host injects an implementation through
/// [`crate::PlatformServices::with_window_service`]. Every method reports
/// failure as the typed [`WindowError`]; the [`Windows`] accessor forwards
/// that error to the mounting runtime as [`crate::RenderError::Window`].
pub trait WindowService {
    /// Realizes one new native window from its complete description.
    ///
    /// The description is the same [`WindowSpec`] the launch window set
    /// uses: the content is a [`crate::WindowContent`] (a retained component
    /// or a reactive closure) mounted in its own window runtime, and the
    /// identity must not already be open.
    fn open(&self, window: WindowSpec) -> Result<(), WindowError>;

    /// Closes one window unconditionally, without consulting its
    /// close-request handler, clearing any pending close token.
    fn close(&self, id: &WindowId) -> Result<(), WindowError>;

    /// Makes one window the focused (key) window.
    fn focus(&self, id: &WindowId) -> Result<(), WindowError>;

    /// Requests one window's content area extent (one-shot; see the module
    /// documentation for why geometry is imperative).
    fn set_content_size(&self, id: &WindowId, size: Size) -> Result<(), WindowError>;

    /// Requests one window's frame origin in platform screen coordinates
    /// (one-shot; see the module documentation for why geometry is
    /// imperative).
    fn set_position(&self, id: &WindowId, position: WindowPosition) -> Result<(), WindowError>;

    /// Consumes the pending close token and performs the deferred close.
    fn confirm_close(&self, id: &WindowId) -> Result<(), WindowError>;

    /// Consumes the pending close token and keeps the window open.
    fn veto_close(&self, id: &WindowId) -> Result<(), WindowError>;
}

/// Sink recording a window request the runtime could not perform.
pub(crate) type WindowErrorReport = Rc<dyn Fn(WindowError)>;

/// Typed runtime window lifecycle reached through [`crate::UpdateContext`].
///
/// Every failure — a host without an injected [`WindowService`] included —
/// is recorded as the typed [`crate::RenderError::Window`] on the mounting
/// runtime, never silently dropped.
pub struct Windows<'a> {
    service: Option<&'a Rc<dyn WindowService>>,
    report: &'a WindowErrorReport,
}

impl<'a> Windows<'a> {
    pub(crate) fn new(
        service: Option<&'a Rc<dyn WindowService>>,
        report: &'a WindowErrorReport,
    ) -> Self {
        Self { service, report }
    }

    fn call(&self, request: impl FnOnce(&Rc<dyn WindowService>) -> Result<(), WindowError>) {
        let Some(service) = self.service else {
            (self.report)(WindowError::NoHost);
            return;
        };
        if let Err(error) = request(service) {
            (self.report)(error);
        }
    }

    /// Opens one new native window from its complete description.
    pub fn open(&self, window: WindowSpec) {
        self.call(move |service| service.open(window));
    }

    /// Closes one window unconditionally from a component decision.
    pub fn close(&self, id: &WindowId) {
        self.call(|service| service.close(id));
    }

    /// Makes one window the focused (key) window.
    pub fn focus(&self, id: &WindowId) {
        self.call(|service| service.focus(id));
    }

    /// Requests one window's content area extent.
    pub fn set_content_size(&self, id: &WindowId, size: Size) {
        self.call(|service| service.set_content_size(id, size));
    }

    /// Requests one window's frame origin in platform screen coordinates.
    pub fn set_position(&self, id: &WindowId, position: WindowPosition) {
        self.call(|service| service.set_position(id, position));
    }

    /// Answers a pending close request by performing the close.
    pub fn confirm_close(&self, id: &WindowId) {
        self.call(|service| service.confirm_close(id));
    }

    /// Answers a pending close request by keeping the window open.
    pub fn veto_close(&self, id: &WindowId) {
        self.call(|service| service.veto_close(id));
    }
}

impl fmt::Debug for Windows<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Windows")
            .field("service", &self.service.is_some())
            .finish_non_exhaustive()
    }
}

/// Callback delivering one per-window lifecycle event.
pub type WindowEventHandler = Rc<dyn Fn(WindowEvent)>;
/// Callback delivering one interceptable close request.
pub type CloseRequestHandler = Rc<dyn Fn()>;

/// The window declaration a content root carries: the reconciled title and
/// the lifecycle subscriptions.
#[derive(Clone, Default)]
pub(crate) struct WindowDeclaration {
    pub(crate) title: Option<String>,
    pub(crate) event: Option<WindowEventHandler>,
    pub(crate) close_request: Option<CloseRequestHandler>,
}

impl WindowDeclaration {
    pub(crate) fn is_declared(&self) -> bool {
        self.title.is_some() || self.event.is_some() || self.close_request.is_some()
    }
}

impl fmt::Debug for WindowDeclaration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowDeclaration")
            .field("title", &self.title)
            .field("event", &self.event.is_some())
            .field("close_request", &self.close_request.is_some())
            .finish()
    }
}

/// Stable window declaration slot connected once by a platform host.
///
/// The reconciler replaces the stored declaration after every successful
/// render — the same stable-slot discipline as
/// [`crate::AcceleratorBindings`] and [`crate::MenuBarBindings`] — so the
/// host reads the freshest declared title and dispatches lifecycle events
/// through the freshest handlers without reconnecting anything native.
#[derive(Clone, Default)]
pub struct WindowBindings(Rc<RefCell<WindowDeclaration>>);

impl WindowBindings {
    pub(crate) fn replace(&self, declaration: WindowDeclaration) {
        *self.0.borrow_mut() = declaration;
    }

    /// Returns the currently declared window title.
    pub fn declared_title(&self) -> Option<String> {
        self.0.borrow().title.clone()
    }

    /// Returns whether the content root currently subscribes to lifecycle
    /// events.
    pub fn declares_window_events(&self) -> bool {
        self.0.borrow().event.is_some()
    }

    /// Returns whether the content root currently intercepts close requests.
    pub fn declares_close_request(&self) -> bool {
        self.0.borrow().close_request.is_some()
    }

    /// Delivers one lifecycle event through the current subscription.
    ///
    /// The handler is cloned out before it runs, so it may re-render and
    /// replace this very declaration. Returns whether a handler was
    /// declared.
    pub fn dispatch_event(&self, event: WindowEvent) -> bool {
        let handler = self.0.borrow().event.clone();
        match handler {
            Some(handler) => {
                handler(event);
                true
            }
            None => false,
        }
    }

    /// Delivers one close request through the current declaration.
    ///
    /// The handler is cloned out before it runs, so it may re-render and
    /// replace this very declaration. Returns whether a handler was
    /// declared.
    pub fn dispatch_close_request(&self) -> bool {
        let handler = self.0.borrow().close_request.clone();
        match handler {
            Some(handler) => {
                handler();
                true
            }
            None => false,
        }
    }
}

impl fmt::Debug for WindowBindings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let declaration = self.0.borrow();
        formatter
            .debug_struct("WindowBindings")
            .field("title", &declaration.title)
            .field("event", &declaration.event.is_some())
            .field("close_request", &declaration.close_request.is_some())
            .finish()
    }
}

/// Recording window service for tests without a platform host.
///
/// Every call is recorded in order as a [`WindowServiceCall`]; results are
/// scripted per identity so typed-error paths are exercisable. Clones share
/// the same recording.
#[derive(Clone, Default)]
pub struct RecordingWindowService {
    calls: Rc<RefCell<Vec<WindowServiceCall>>>,
    rejections: Rc<RefCell<Vec<(WindowId, WindowError)>>>,
}

/// One recorded window service request.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowServiceCall {
    /// An open request with the identity it described.
    Open(WindowId),
    /// An unconditional close request.
    Close(WindowId),
    /// A focus request.
    Focus(WindowId),
    /// A content-size request.
    SetContentSize(WindowId, Size),
    /// A position request.
    SetPosition(WindowId, WindowPosition),
    /// A close confirmation.
    ConfirmClose(WindowId),
    /// A close veto.
    VetoClose(WindowId),
}

impl RecordingWindowService {
    /// Creates a service with no recorded calls.
    pub fn new() -> Self {
        Self::default()
    }

    /// Scripts the typed rejection every future call for `id` returns.
    pub fn reject(&self, id: WindowId, error: WindowError) {
        self.rejections.borrow_mut().push((id, error));
    }

    /// Returns the recorded calls in order.
    pub fn calls(&self) -> Vec<WindowServiceCall> {
        self.calls.borrow().clone()
    }

    fn record(&self, id: &WindowId, call: WindowServiceCall) -> Result<(), WindowError> {
        self.calls.borrow_mut().push(call);
        let rejection = self
            .rejections
            .borrow()
            .iter()
            .find(|(rejected, _)| rejected == id)
            .map(|(_, error)| error.clone());
        match rejection {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl fmt::Debug for RecordingWindowService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordingWindowService")
            .field("calls", &self.calls.borrow().len())
            .finish()
    }
}

impl WindowService for RecordingWindowService {
    fn open(&self, window: WindowSpec) -> Result<(), WindowError> {
        let id = window.id.clone();
        self.record(&id, WindowServiceCall::Open(id.clone()))
    }

    fn close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.record(id, WindowServiceCall::Close(id.clone()))
    }

    fn focus(&self, id: &WindowId) -> Result<(), WindowError> {
        self.record(id, WindowServiceCall::Focus(id.clone()))
    }

    fn set_content_size(&self, id: &WindowId, size: Size) -> Result<(), WindowError> {
        self.record(id, WindowServiceCall::SetContentSize(id.clone(), size))
    }

    fn set_position(&self, id: &WindowId, position: WindowPosition) -> Result<(), WindowError> {
        self.record(id, WindowServiceCall::SetPosition(id.clone(), position))
    }

    fn confirm_close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.record(id, WindowServiceCall::ConfirmClose(id.clone()))
    }

    fn veto_close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.record(id, WindowServiceCall::VetoClose(id.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RecordingWindowService, WindowBindings, WindowDeclaration, WindowError, WindowEvent,
        WindowService, WindowServiceCall,
    };
    use crate::window::{Size, WindowId};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn bindings_replace_the_declaration_in_place() {
        let bindings = WindowBindings::default();
        assert_eq!(bindings.declared_title(), None);
        assert!(!bindings.declares_window_events());
        assert!(!bindings.declares_close_request());
        assert!(!bindings.dispatch_event(WindowEvent::Focused));
        assert!(!bindings.dispatch_close_request());

        let observed = Rc::new(RefCell::new(Vec::new()));
        let sink = observed.clone();
        bindings.replace(WindowDeclaration {
            title: Some("Session 2".to_owned()),
            event: Some(Rc::new(move |event| sink.borrow_mut().push(event))),
            close_request: None,
        });
        assert_eq!(bindings.declared_title().as_deref(), Some("Session 2"));
        assert!(bindings.dispatch_event(WindowEvent::Resized(Size::new(640.0, 480.0))));
        assert_eq!(
            *observed.borrow(),
            vec![WindowEvent::Resized(Size::new(640.0, 480.0))]
        );

        bindings.replace(WindowDeclaration::default());
        assert!(!bindings.dispatch_event(WindowEvent::Focused));
        assert_eq!(
            *observed.borrow_mut(),
            vec![WindowEvent::Resized(Size::new(640.0, 480.0))]
        );
    }

    #[test]
    fn a_dispatched_handler_may_replace_its_own_declaration() {
        let bindings = WindowBindings::default();
        let reentrant = bindings.clone();
        bindings.replace(WindowDeclaration {
            title: None,
            event: None,
            close_request: Some(Rc::new(move || {
                reentrant.replace(WindowDeclaration::default());
            })),
        });
        assert!(bindings.dispatch_close_request());
        assert!(!bindings.declares_close_request());
    }

    #[test]
    fn the_recording_service_scripts_typed_rejections_per_identity() {
        let service = RecordingWindowService::new();
        let rejected = WindowId::new("stuck");
        service.reject(
            rejected.clone(),
            WindowError::NotOpen {
                id: rejected.clone(),
            },
        );

        assert_eq!(service.close(&WindowId::new("fine")), Ok(()));
        assert_eq!(
            service.focus(&rejected),
            Err(WindowError::NotOpen {
                id: rejected.clone()
            })
        );
        assert_eq!(
            service.calls(),
            vec![
                WindowServiceCall::Close(WindowId::new("fine")),
                WindowServiceCall::Focus(rejected),
            ]
        );
    }
}
