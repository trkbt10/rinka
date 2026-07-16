//! Live retained projection for adapters with their own reconciler.

use crate::{
    Element, MountedNode, NativeBackend, PropertyPatch, RenderError, Renderer, WindowContent,
    WindowRuntime,
};
use std::convert::Infallible;

/// Stable identity assigned to one node in a live projected tree.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectedHandle(u64);

impl ProjectedHandle {
    /// Returns the process-local identity value.
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug)]
struct ProjectionBackend {
    next: u64,
}

impl ProjectionBackend {
    const ROOT: ProjectedHandle = ProjectedHandle(0);

    const fn new() -> Self {
        Self { next: 1 }
    }
}

impl NativeBackend for ProjectionBackend {
    type Handle = ProjectedHandle;
    type Error = Infallible;

    fn root(&self) -> Self::Handle {
        Self::ROOT
    }

    fn validate(&self, _element: &Element) -> Result<(), Self::Error> {
        Ok(())
    }

    fn create(
        &mut self,
        _element: &Element,
        _events: crate::EventBindings,
    ) -> Result<Self::Handle, Self::Error> {
        let handle = ProjectedHandle(self.next);
        self.next += 1;
        Ok(handle)
    }

    fn apply(&mut self, _handle: &Self::Handle, _patch: &PropertyPatch) -> Result<(), Self::Error> {
        Ok(())
    }

    fn insert_child(
        &mut self,
        _parent: &Self::Handle,
        _child: &Self::Handle,
        _index: usize,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn remove_child(
        &mut self,
        _parent: &Self::Handle,
        _child: &Self::Handle,
        _index: usize,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn move_child(
        &mut self,
        _parent: &Self::Handle,
        _child: &Self::Handle,
        _from: usize,
        _to: usize,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Retains common component state and keyed identities for a platform bridge.
///
/// This projection is for native UI systems that already reconcile their own
/// object tree. It keeps Rinka's validation, component invalidation, keys,
/// and stable event slots authoritative while the platform maps the mounted
/// nodes into its toolkit-specific element descriptions.
pub struct WindowProjection {
    runtime: WindowRuntime<ProjectionBackend>,
}

impl WindowProjection {
    /// Mounts reactive window content and validates its initial tree.
    pub fn mount(content: WindowContent) -> Result<Self, RenderError<Infallible>> {
        let runtime = WindowRuntime::mount(Renderer::new(ProjectionBackend::new()), content)?;
        Ok(Self { runtime })
    }

    /// Reads the current retained root, if mounting produced one.
    pub fn with_root<R>(&self, read: impl FnOnce(&MountedNode<ProjectedHandle>) -> R) -> Option<R> {
        self.runtime
            .with_renderer(|renderer| renderer.mounted().map(read))
    }

    /// Schedules a platform pass after a common component update reconciles.
    pub fn set_reconciled_handler(&self, handler: impl Fn() + 'static) {
        self.runtime.set_reconciled_handler(handler);
    }

    /// Takes the latest asynchronous content or tree error.
    pub fn take_error(&self) -> Option<RenderError<Infallible>> {
        self.runtime.take_error()
    }
}

impl std::fmt::Debug for WindowProjection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowProjection")
            .field("runtime", &self.runtime)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::WindowProjection;
    use crate::{Component, Dispatch, Element, Props, WindowContent, button, column, label};
    use std::cell::Cell;
    use std::rc::Rc;

    struct Counter {
        value: u32,
    }

    impl Component for Counter {
        type Message = ();

        fn update(&mut self, (): Self::Message) {
            self.value += 1;
        }

        fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
            column([
                label(format!("count={}", self.value)).with_key("count"),
                button("Increment", "Increment", move || dispatch.emit(())).with_key("increment"),
            ])
            .with_key("counter")
        }
    }

    #[test]
    fn common_event_reconciles_and_preserves_projected_identity() {
        let projection = WindowProjection::mount(WindowContent::component(Counter { value: 0 }))
            .expect("initial projection");
        let reconciled = Rc::new(Cell::new(0));
        let observed = reconciled.clone();
        projection.set_reconciled_handler(move || observed.set(observed.get() + 1));

        let (root_before, label_before, button_before, events) = projection
            .with_root(|root| {
                (
                    root.handle().value(),
                    root.children()[0].handle().value(),
                    root.children()[1].handle().value(),
                    root.children()[1].events().clone(),
                )
            })
            .expect("projected root");

        events.emit_activate();

        projection
            .with_root(|root| {
                assert_eq!(root.handle().value(), root_before);
                assert_eq!(root.children()[0].handle().value(), label_before);
                assert_eq!(root.children()[1].handle().value(), button_before);
                assert!(matches!(
                    root.children()[0].element().props(),
                    Props::Label { text, .. } if text == "count=1"
                ));
            })
            .expect("updated projected root");
        assert_eq!(reconciled.get(), 1);

        events.emit_activate();
        projection
            .with_root(|root| {
                assert!(matches!(
                    root.children()[0].element().props(),
                    Props::Label { text, .. } if text == "count=2"
                ));
            })
            .expect("second projected update");
        assert_eq!(reconciled.get(), 2);
        assert!(projection.take_error().is_none());
    }
}
