//! Component state and queued message delivery.

use crate::dialog::{DialogErrorReport, Dialogs};
use crate::{
    Clipboard, Element, ElementKind, NativeBackend, PlatformServices, RenderContext, RenderError,
    Renderer, TreeError, WindowContent,
};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt;
use std::rc::{Rc, Weak};

/// Stateful declarative application unit.
pub trait Component {
    /// Message accepted by the state transition function.
    type Message: 'static;

    /// Applies one message; platform capabilities are reached through the
    /// update context's injected services.
    fn update(&mut self, message: Self::Message, context: &UpdateContext<Self::Message>);

    /// Describes the current native UI tree.
    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element;
}

/// Per-update capability context handed to [`Component::update`].
///
/// The context carries the component's own [`Dispatch`] and the host's
/// injected [`PlatformServices`]. Message delivery is queue-based: a message
/// emitted while an update runs — including a synchronously answered dialog
/// outcome — is applied after the current update returns, never re-entering
/// the component.
pub struct UpdateContext<M> {
    dispatch: Dispatch<M>,
    services: Rc<PlatformServices>,
    report_dialog_error: DialogErrorReport,
}

impl<M: 'static> UpdateContext<M> {
    /// Creates a context over a recording dispatch and fake services.
    ///
    /// Runtimes build one per message through [`Self::for_runtime`]; tests
    /// build their own with this constructor. A dialog error raised without
    /// a runtime is discarded, matching the runtime-less snapshot path.
    pub fn new(dispatch: Dispatch<M>, services: PlatformServices) -> Self {
        Self::for_runtime(dispatch, Rc::new(services), Rc::new(|_| {}))
    }

    pub(crate) fn for_runtime(
        dispatch: Dispatch<M>,
        services: Rc<PlatformServices>,
        report_dialog_error: DialogErrorReport,
    ) -> Self {
        Self {
            dispatch,
            services,
            report_dialog_error,
        }
    }

    /// Message sender for follow-up messages from this update.
    pub fn dispatch(&self) -> &Dispatch<M> {
        &self.dispatch
    }

    /// Returns the mounting host's service registry.
    pub fn services(&self) -> &PlatformServices {
        &self.services
    }

    /// Returns the platform clipboard.
    pub fn clipboard(&self) -> &Clipboard {
        self.services.clipboard()
    }

    /// Window-modal dialog presentation through the host's injected service.
    pub fn dialogs(&self) -> Dialogs<'_, M> {
        Dialogs::new(
            self.services.dialog_service(),
            &self.dispatch,
            &self.report_dialog_error,
        )
    }
}

impl<M> fmt::Debug for UpdateContext<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateContext")
            .field("services", &self.services)
            .finish_non_exhaustive()
    }
}

/// Cloneable message sender captured by event closures.
pub struct Dispatch<M>(Rc<dyn Fn(M)>);

impl<M> Dispatch<M> {
    /// Creates a sender from a raw handler.
    ///
    /// The runtimes build queue-backed senders themselves; this constructor
    /// exists for platform hosts and for tests that record emitted messages
    /// while calling [`Component::update`] directly.
    pub fn from_handler(handler: impl Fn(M) + 'static) -> Self {
        Self(Rc::new(handler))
    }

    pub(crate) fn downgrade(&self) -> WeakDispatch<M> {
        WeakDispatch(Rc::downgrade(&self.0))
    }

    /// Emits a message.
    pub fn emit(&self, message: M) {
        (self.0)(message);
    }
}

impl<M> Clone for Dispatch<M> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

/// Non-owning message sender used to break dispatch reference cycles.
pub(crate) struct WeakDispatch<M>(Weak<dyn Fn(M)>);

impl<M> WeakDispatch<M> {
    pub(crate) fn upgrade(&self) -> Option<Dispatch<M>> {
        self.0.upgrade().map(Dispatch)
    }
}

impl<M> fmt::Debug for Dispatch<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Dispatch(..)")
    }
}

struct RuntimeInner<B: NativeBackend, C: Component> {
    renderer: RefCell<Renderer<B>>,
    component: RefCell<C>,
    queue: RefCell<VecDeque<C::Message>>,
    processing: Cell<bool>,
    services: Rc<PlatformServices>,
    last_error: RefCell<Option<RenderError<B::Error>>>,
}

/// Mounted component and native renderer.
pub struct AppRuntime<B: NativeBackend, C: Component> {
    inner: Rc<RuntimeInner<B, C>>,
}

impl<B: NativeBackend + 'static, C: Component + 'static> AppRuntime<B, C> {
    /// Mounts a component with the host's injected platform services and
    /// performs the first render.
    pub fn mount(
        renderer: Renderer<B>,
        component: C,
        services: PlatformServices,
    ) -> Result<Self, RenderError<B::Error>> {
        let inner = Rc::new(RuntimeInner {
            renderer: RefCell::new(renderer),
            component: RefCell::new(component),
            queue: RefCell::new(VecDeque::new()),
            processing: Cell::new(false),
            services: Rc::new(services),
            last_error: RefCell::new(None),
        });
        let runtime = Self { inner };
        runtime.render_current()?;
        Ok(runtime)
    }

    fn dispatch(weak: Weak<RuntimeInner<B, C>>) -> Dispatch<C::Message> {
        Dispatch(Rc::new(move |message| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            inner.queue.borrow_mut().push_back(message);
            Self::drain(&inner);
        }))
    }

    fn update_context(inner: &Rc<RuntimeInner<B, C>>) -> UpdateContext<C::Message> {
        let error_slot = Rc::downgrade(inner);
        UpdateContext::for_runtime(
            Self::dispatch(Rc::downgrade(inner)),
            inner.services.clone(),
            Rc::new(move |error| {
                if let Some(inner) = error_slot.upgrade() {
                    *inner.last_error.borrow_mut() = Some(RenderError::Dialog(error));
                }
            }),
        )
    }

    fn drain(inner: &Rc<RuntimeInner<B, C>>) {
        if inner.processing.replace(true) {
            return;
        }
        loop {
            let message = inner.queue.borrow_mut().pop_front();
            let Some(message) = message else {
                break;
            };
            let context = Self::update_context(inner);
            inner.component.borrow_mut().update(message, &context);
            let dispatch = Self::dispatch(Rc::downgrade(inner));
            let next = inner.component.borrow().view(dispatch);
            if let Err(error) = inner.renderer.borrow_mut().render(next) {
                inner.queue.borrow_mut().clear();
                *inner.last_error.borrow_mut() = Some(error);
                break;
            }
        }
        inner.processing.set(false);
    }

    fn render_current(&self) -> Result<(), RenderError<B::Error>> {
        let dispatch = Self::dispatch(Rc::downgrade(&self.inner));
        let next = self.inner.component.borrow().view(dispatch);
        self.inner.renderer.borrow_mut().render(next).map(|_| ())
    }

    /// Reads component state.
    pub fn with_component<R>(&self, read: impl FnOnce(&C) -> R) -> R {
        read(&self.inner.component.borrow())
    }

    /// Reads the native renderer.
    pub fn with_renderer<R>(&self, read: impl FnOnce(&Renderer<B>) -> R) -> R {
        read(&self.inner.renderer.borrow())
    }

    /// Mutates the native renderer for platform integration.
    pub fn with_renderer_mut<R>(&self, write: impl FnOnce(&mut Renderer<B>) -> R) -> R {
        write(&mut self.inner.renderer.borrow_mut())
    }

    /// Takes the most recent asynchronous render error.
    pub fn take_error(&self) -> Option<RenderError<B::Error>> {
        self.inner.last_error.borrow_mut().take()
    }
}

impl<B: NativeBackend, C: Component> Clone for AppRuntime<B, C> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<B: NativeBackend, C: Component> fmt::Debug for AppRuntime<B, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppRuntime")
            .field("queued", &self.inner.queue.borrow().len())
            .field("processing", &self.inner.processing.get())
            .field("has_error", &self.inner.last_error.borrow().is_some())
            .finish()
    }
}

struct WindowRuntimeInner<B: NativeBackend> {
    renderer: RefCell<Renderer<B>>,
    content: WindowContent,
    root_kind: Cell<Option<ElementKind>>,
    rendering: Cell<bool>,
    pending: Cell<bool>,
    reconciled: RefCell<Option<Rc<dyn Fn()>>>,
    last_error: RefCell<Option<RenderError<B::Error>>>,
}

/// Type-erased reactive content mounted in one native window.
pub struct WindowRuntime<B: NativeBackend> {
    inner: Rc<WindowRuntimeInner<B>>,
}

impl<B: NativeBackend + 'static> WindowRuntime<B> {
    /// Mounts window content with the host's injected platform services and
    /// performs the first reconciliation.
    pub fn mount(
        renderer: Renderer<B>,
        content: WindowContent,
        services: PlatformServices,
    ) -> Result<Self, RenderError<B::Error>> {
        content.install_services(services);
        let runtime = Self {
            inner: Rc::new(WindowRuntimeInner {
                renderer: RefCell::new(renderer),
                content,
                root_kind: Cell::new(None),
                rendering: Cell::new(false),
                pending: Cell::new(false),
                reconciled: RefCell::new(None),
                last_error: RefCell::new(None),
            }),
        };
        runtime.render_now()?;
        Ok(runtime)
    }

    fn context(inner: &Rc<WindowRuntimeInner<B>>) -> RenderContext {
        let weak = Rc::downgrade(inner);
        RenderContext::new(move || {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            inner.pending.set(true);
            Self::drain(&inner);
        })
    }

    fn drain(inner: &Rc<WindowRuntimeInner<B>>) {
        if inner.rendering.replace(true) {
            return;
        }
        while inner.pending.replace(false) {
            let context = Self::context(inner);
            let next = inner.content.render(context);
            let result = Self::reconcile(inner, next);
            // Dialog presentation failures recorded by the drained updates
            // become the same typed error channel reconciliation uses.
            if let Some(error) = inner.content.take_dialog_error() {
                *inner.last_error.borrow_mut() = Some(RenderError::Dialog(error));
            }
            match result {
                Ok(()) => {
                    let reconciled = inner.reconciled.borrow().clone();
                    if let Some(reconciled) = reconciled {
                        reconciled();
                    }
                }
                Err(error) => {
                    inner.pending.set(false);
                    *inner.last_error.borrow_mut() = Some(error);
                    break;
                }
            }
        }
        inner.rendering.set(false);
    }

    fn render_now(&self) -> Result<(), RenderError<B::Error>> {
        let context = Self::context(&self.inner);
        let next = self.inner.content.render(context);
        Self::reconcile(&self.inner, next)
    }

    fn reconcile(
        inner: &Rc<WindowRuntimeInner<B>>,
        next: Element,
    ) -> Result<(), RenderError<B::Error>> {
        let next_kind = next.kind();
        if let Some(previous) = inner.root_kind.get()
            && previous != next_kind
        {
            return Err(RenderError::Tree(TreeError::WindowRootKindChanged {
                previous,
                next: next_kind,
            }));
        }
        inner.renderer.borrow_mut().render(next)?;
        inner.root_kind.set(Some(next_kind));
        Ok(())
    }

    /// Reads the native renderer.
    pub fn with_renderer<R>(&self, read: impl FnOnce(&Renderer<B>) -> R) -> R {
        read(&self.inner.renderer.borrow())
    }

    /// Mutates the native renderer for platform integration.
    pub fn with_renderer_mut<R>(&self, write: impl FnOnce(&mut Renderer<B>) -> R) -> R {
        write(&mut self.inner.renderer.borrow_mut())
    }

    /// Schedules a platform pass after content reconciles from a component
    /// update.
    ///
    /// Platform hosts use this to refresh application-level chrome derived
    /// from the window's declaration — the application menu bar — after every
    /// reconciliation, without polling.
    pub fn set_reconciled_handler(&self, handler: impl Fn() + 'static) {
        *self.inner.reconciled.borrow_mut() = Some(Rc::new(handler));
    }

    /// Takes the most recent asynchronous reconciliation error.
    pub fn take_error(&self) -> Option<RenderError<B::Error>> {
        self.inner.last_error.borrow_mut().take()
    }
}

impl<B: NativeBackend> Clone for WindowRuntime<B> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<B: NativeBackend> fmt::Debug for WindowRuntime<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowRuntime")
            .field("root_kind", &self.inner.root_kind.get())
            .field("rendering", &self.inner.rendering.get())
            .field("pending", &self.inner.pending.get())
            .field("has_error", &self.inner.last_error.borrow().is_some())
            .finish()
    }
}
