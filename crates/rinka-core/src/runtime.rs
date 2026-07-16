//! Component state and queued message delivery.

use crate::{
    Element, ElementKind, NativeBackend, RenderContext, RenderError, Renderer, TreeError,
    WindowContent,
};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt;
use std::rc::{Rc, Weak};

/// Stateful declarative application unit.
pub trait Component {
    /// Message accepted by the state transition function.
    type Message: 'static;

    /// Applies one message.
    fn update(&mut self, message: Self::Message);

    /// Describes the current native UI tree.
    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element;
}

/// Cloneable message sender captured by event closures.
pub struct Dispatch<M>(Rc<dyn Fn(M)>);

impl<M> Dispatch<M> {
    pub(crate) fn from_handler(handler: impl Fn(M) + 'static) -> Self {
        Self(Rc::new(handler))
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
    last_error: RefCell<Option<RenderError<B::Error>>>,
}

/// Mounted component and native renderer.
pub struct AppRuntime<B: NativeBackend, C: Component> {
    inner: Rc<RuntimeInner<B, C>>,
}

impl<B: NativeBackend + 'static, C: Component + 'static> AppRuntime<B, C> {
    /// Mounts a component and performs the first render.
    pub fn mount(renderer: Renderer<B>, component: C) -> Result<Self, RenderError<B::Error>> {
        let inner = Rc::new(RuntimeInner {
            renderer: RefCell::new(renderer),
            component: RefCell::new(component),
            queue: RefCell::new(VecDeque::new()),
            processing: Cell::new(false),
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

    fn drain(inner: &Rc<RuntimeInner<B, C>>) {
        if inner.processing.replace(true) {
            return;
        }
        loop {
            let message = inner.queue.borrow_mut().pop_front();
            let Some(message) = message else {
                break;
            };
            inner.component.borrow_mut().update(message);
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
    /// Mounts window content and performs the first reconciliation.
    pub fn mount(
        renderer: Renderer<B>,
        content: WindowContent,
    ) -> Result<Self, RenderError<B::Error>> {
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
            match Self::reconcile(inner, next) {
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

    pub(crate) fn set_reconciled_handler(&self, handler: impl Fn() + 'static) {
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
