//! Window, toolbar, and panel descriptions.

use crate::runtime::{ServiceError, UpdateContext, WeakDispatch};
use crate::services::PlatformServices;
use crate::{Component, Dispatch, Element, MenuBar, ToolbarDisplay, ToolbarItem};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;

/// Stable top-level window identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WindowId(String);

impl WindowId {
    /// Creates an identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Logical size in platform-independent points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    /// Horizontal extent.
    pub width: f64,
    /// Vertical extent.
    pub height: f64,
}

impl Size {
    /// Creates a positive size.
    pub fn new(width: f64, height: f64) -> Self {
        Self {
            width: width.max(1.0),
            height: height.max(1.0),
        }
    }
}

/// Native panel interaction policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PanelBehavior {
    /// Keeps the panel above normal windows of the same application.
    pub floating: bool,
    /// Hides the panel when the application becomes inactive.
    pub hides_when_inactive: bool,
    /// Allows text fields and other controls to become key.
    pub accepts_keyboard: bool,
}

/// Top-level window semantic kind.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WindowKind {
    /// Main document or application window.
    Main,
    /// Settings window using platform preference conventions.
    Preferences,
    /// Auxiliary native panel.
    Panel(PanelBehavior),
}

/// Render invalidation handle supplied to reactive window content.
#[derive(Clone)]
pub struct RenderContext(Rc<dyn Fn()>);

impl RenderContext {
    pub(crate) fn new(handler: impl Fn() + 'static) -> Self {
        Self(Rc::new(handler))
    }

    /// Requests reconciliation from the current component state.
    pub fn request_render(&self) {
        (self.0)();
    }

    fn inert() -> Self {
        Self(Rc::new(|| {}))
    }
}

impl fmt::Debug for RenderContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RenderContext(..)")
    }
}

/// Type-erased content factory retained by a native window host.
#[derive(Clone)]
pub struct WindowContent {
    render: Rc<dyn Fn(RenderContext) -> Element>,
    services: Rc<RefCell<Rc<PlatformServices>>>,
    service_error: Rc<RefCell<Option<ServiceError>>>,
}

impl WindowContent {
    /// Creates content from a reactive view function.
    pub fn reactive(render: impl Fn(RenderContext) -> Element + 'static) -> Self {
        Self {
            render: Rc::new(render),
            services: Rc::new(RefCell::new(Rc::new(PlatformServices::default()))),
            service_error: Rc::new(RefCell::new(None)),
        }
    }

    /// Retains a component and connects its messages to window reconciliation.
    ///
    /// Message delivery is queue-based: a message emitted while an update
    /// runs — including a synchronously answered dialog outcome — is applied
    /// after the current update returns, so the component is never
    /// re-entered. Each update receives an [`UpdateContext`] carrying the
    /// services the mounting host injected.
    pub fn component<C>(component: C) -> Self
    where
        C: Component + 'static,
        C::Message: 'static,
    {
        let component = Rc::new(RefCell::new(component));
        let content = Self::reactive(|_| unreachable!("replaced below"));
        let services = content.services.clone();
        let service_error = content.service_error.clone();
        let messages: Rc<RefCell<VecDeque<C::Message>>> = Rc::new(RefCell::new(VecDeque::new()));
        let draining = Rc::new(Cell::new(false));
        let render = move |context: RenderContext| {
            let target = component.clone();
            let render_context = context.clone();
            let services = services.clone();
            let service_error = service_error.clone();
            let messages = messages.clone();
            let draining = draining.clone();
            // A dialog outcome re-enters this same dispatch loop later; the
            // slot holds a weak self-reference so the handler and its own
            // dispatch never form a retain cycle.
            let dispatch_slot: Rc<RefCell<Option<WeakDispatch<C::Message>>>> =
                Rc::new(RefCell::new(None));
            let handler_slot = dispatch_slot.clone();
            let dispatch = Dispatch::from_handler(move |message| {
                messages.borrow_mut().push_back(message);
                if draining.replace(true) {
                    return;
                }
                loop {
                    let next = messages.borrow_mut().pop_front();
                    let Some(next) = next else {
                        break;
                    };
                    let redispatch = handler_slot
                        .borrow()
                        .as_ref()
                        .and_then(WeakDispatch::upgrade)
                        .expect("the dispatch loop outlives its own invocation");
                    let error_slot = service_error.clone();
                    let update_context = UpdateContext::for_runtime(
                        redispatch,
                        services.borrow().clone(),
                        Rc::new(move |error| {
                            *error_slot.borrow_mut() = Some(error);
                        }),
                    );
                    target.borrow_mut().update(next, &update_context);
                }
                draining.set(false);
                render_context.request_render();
            });
            *dispatch_slot.borrow_mut() = Some(dispatch.downgrade());
            component.borrow().view(dispatch)
        };
        Self {
            render: Rc::new(render),
            services: content.services,
            service_error: content.service_error,
        }
    }

    /// Produces a read-only snapshot for extraction and structural review.
    pub fn snapshot(&self) -> Element {
        self.render(RenderContext::inert())
    }

    pub(crate) fn render(&self, context: RenderContext) -> Element {
        (self.render)(context)
    }

    /// Installs the platform services injected by the mounting host.
    pub(crate) fn install_services(&self, services: PlatformServices) {
        *self.services.borrow_mut() = Rc::new(services);
    }

    /// Takes the most recent service failure raised by a drained update.
    pub(crate) fn take_service_error(&self) -> Option<ServiceError> {
        self.service_error.borrow_mut().take()
    }
}

impl From<Element> for WindowContent {
    fn from(element: Element) -> Self {
        Self::reactive(move |_| element.clone())
    }
}

impl fmt::Debug for WindowContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WindowContent(..)")
    }
}

/// Complete top-level native window description.
#[derive(Clone, Debug)]
pub struct WindowSpec {
    /// Stable identity.
    pub id: WindowId,
    /// Visible title.
    pub title: String,
    /// Native semantic kind.
    pub kind: WindowKind,
    /// Initial content size.
    pub initial_size: Size,
    /// Minimum content size.
    pub minimum_size: Size,
    /// Native toolbar items.
    pub toolbar: Vec<ToolbarItem>,
    /// Native toolbar label presentation.
    pub toolbar_display: ToolbarDisplay,
    /// Declarative content root.
    pub content: WindowContent,
}

/// Application behavior when its last open window closes.
///
/// Each platform host maps the declaration onto its native lifecycle:
/// macOS answers `applicationShouldTerminateAfterLastWindowClosed:` from it,
/// hosts whose process lifetime is bound to their single window (GTK, WinUI)
/// treat [`Self::Exit`] and [`Self::PlatformDefault`] identically and reject
/// [`Self::StayRunning`] with a typed diagnostic until they can host an
/// empty window set.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LastWindowClosedPolicy {
    /// The platform's own convention: macOS keeps the application running
    /// (the AppKit default), GTK and WinUI exit with their window.
    #[default]
    PlatformDefault,
    /// The application exits when its last window closes.
    Exit,
    /// The application keeps running with no open windows, where the
    /// platform can represent that state.
    StayRunning,
}

/// Application identity and initial window set.
#[derive(Clone, Debug)]
pub struct ApplicationSpec {
    /// Reverse-DNS application identifier.
    pub id: String,
    /// Human-readable application name.
    pub name: String,
    /// Application-level menu bar declaration.
    ///
    /// This is the bar installed at startup and whenever no window declares
    /// its own. Because component state lives in windows, a bar whose items
    /// dispatch component messages or reflect component state is redeclared
    /// by a window content root through [`crate::Element::menu_bar`]; the
    /// focused window's declaration then supersedes this one. An empty bar
    /// declares no application-level menus.
    pub menu_bar: MenuBar,
    /// Initial windows and panels.
    ///
    /// This is the launch set only; the runtime window set changes through
    /// [`crate::Windows`], reached from a component update via
    /// [`crate::UpdateContext::windows`].
    pub windows: Vec<WindowSpec>,
    /// Behavior when the last open window closes.
    pub last_window_closed: LastWindowClosedPolicy,
}
