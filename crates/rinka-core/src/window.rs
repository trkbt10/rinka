//! Window, toolbar, and panel descriptions.

use crate::{
    Component, Dispatch, Element, PlatformServices, ToolbarDisplay, ToolbarItem, UpdateContext,
};
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
pub struct RenderContext {
    render: Rc<dyn Fn()>,
    services: PlatformServices,
}

impl RenderContext {
    pub(crate) fn new(handler: impl Fn() + 'static, services: PlatformServices) -> Self {
        Self {
            render: Rc::new(handler),
            services,
        }
    }

    /// Requests reconciliation from the current component state.
    pub fn request_render(&self) {
        (self.render)();
    }

    /// Returns the platform services registered by the mounting host.
    pub fn services(&self) -> &PlatformServices {
        &self.services
    }

    fn inert() -> Self {
        Self {
            render: Rc::new(|| {}),
            services: PlatformServices::default(),
        }
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
}

impl WindowContent {
    /// Creates content from a reactive view function.
    pub fn reactive(render: impl Fn(RenderContext) -> Element + 'static) -> Self {
        Self {
            render: Rc::new(render),
        }
    }

    /// Retains a component and connects its messages to window reconciliation.
    ///
    /// Delivery follows [`crate::AppRuntime`]'s queued discipline: a message
    /// emitted while an update is running — including a synchronously
    /// delivered clipboard read completion — is queued and applied in order
    /// after the current update returns, so `update` never re-enters itself.
    pub fn component<C>(component: C) -> Self
    where
        C: Component + 'static,
        C::Message: 'static,
    {
        let driver = Rc::new(ComponentDriver {
            component: RefCell::new(component),
            queue: RefCell::new(VecDeque::new()),
            processing: Cell::new(false),
        });
        Self::reactive(move |context| {
            let dispatch = ComponentDriver::dispatch(&driver, &context);
            driver.component.borrow().view(dispatch)
        })
    }

    /// Produces a read-only snapshot for extraction and structural review.
    pub fn snapshot(&self) -> Element {
        self.render(RenderContext::inert())
    }

    pub(crate) fn render(&self, context: RenderContext) -> Element {
        (self.render)(context)
    }
}

/// Component state with its queued message delivery for window content.
struct ComponentDriver<C: Component> {
    component: RefCell<C>,
    queue: RefCell<VecDeque<C::Message>>,
    processing: Cell<bool>,
}

impl<C: Component + 'static> ComponentDriver<C> {
    /// Builds a sender that queues into this driver and drains it.
    fn dispatch(driver: &Rc<Self>, context: &RenderContext) -> Dispatch<C::Message> {
        let driver = driver.clone();
        let context = context.clone();
        Dispatch::from_handler(move |message| {
            driver.queue.borrow_mut().push_back(message);
            Self::drain(&driver, &context);
        })
    }

    /// Applies queued messages in order, then requests one reconciliation.
    ///
    /// The `processing` guard makes a nested emit — from inside `update` or
    /// from a synchronous service completion — enqueue instead of re-enter.
    fn drain(driver: &Rc<Self>, context: &RenderContext) {
        if driver.processing.replace(true) {
            return;
        }
        let mut delivered = false;
        loop {
            let message = driver.queue.borrow_mut().pop_front();
            let Some(message) = message else {
                break;
            };
            let update_context =
                UpdateContext::new(Self::dispatch(driver, context), context.services().clone());
            driver
                .component
                .borrow_mut()
                .update(message, &update_context);
            delivered = true;
        }
        driver.processing.set(false);
        if delivered {
            context.request_render();
        }
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

/// Application identity and initial window set.
#[derive(Clone, Debug)]
pub struct ApplicationSpec {
    /// Reverse-DNS application identifier.
    pub id: String,
    /// Human-readable application name.
    pub name: String,
    /// Initial windows and panels.
    pub windows: Vec<WindowSpec>,
}
