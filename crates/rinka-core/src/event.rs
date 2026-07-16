//! Stable event slots shared by the reconciler and native adapters.

use crate::{PointerEvent, TableSort};
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Callback used by buttons, rows, and other activation controls.
pub type ActivateHandler = Rc<dyn Fn()>;
/// Callback used by editable text controls.
pub type InputHandler = Rc<dyn Fn(String)>;
/// Callback used by binary controls.
pub type ToggleHandler = Rc<dyn Fn(bool)>;
/// Callback used by native sortable table headers.
pub type SortHandler = Rc<dyn Fn(TableSort)>;
/// Callback used by owned-drawing canvas surfaces.
pub type PointerHandler = Rc<dyn Fn(PointerEvent)>;

/// Event handlers associated with one declarative element.
#[derive(Clone, Default)]
pub struct EventHandlers {
    pub(crate) activate: Option<ActivateHandler>,
    pub(crate) input: Option<InputHandler>,
    pub(crate) toggle: Option<ToggleHandler>,
    pub(crate) sort: Option<SortHandler>,
    pub(crate) pointer: Option<PointerHandler>,
}

impl fmt::Debug for EventHandlers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventHandlers")
            .field("activate", &self.activate.is_some())
            .field("input", &self.input.is_some())
            .field("toggle", &self.toggle.is_some())
            .field("sort", &self.sort.is_some())
            .field("pointer", &self.pointer.is_some())
            .finish()
    }
}

#[derive(Default)]
struct EventSlots {
    activate: Option<ActivateHandler>,
    input: Option<InputHandler>,
    toggle: Option<ToggleHandler>,
    sort: Option<SortHandler>,
    pointer: Option<PointerHandler>,
}

/// Stable native signal target whose handlers can be replaced after a render.
///
/// A platform adapter connects a native signal to this value once. The
/// reconciler updates the stored closures without reconnecting the signal.
#[derive(Clone, Default)]
pub struct EventBindings(Rc<RefCell<EventSlots>>);

impl EventBindings {
    /// Creates bindings from one element's handlers.
    pub fn new(handlers: &EventHandlers) -> Self {
        Self(Rc::new(RefCell::new(EventSlots {
            activate: handlers.activate.clone(),
            input: handlers.input.clone(),
            toggle: handlers.toggle.clone(),
            sort: handlers.sort.clone(),
            pointer: handlers.pointer.clone(),
        })))
    }

    /// Creates an activation-only binding for window or toolbar hosts.
    pub fn activate(handler: ActivateHandler) -> Self {
        Self(Rc::new(RefCell::new(EventSlots {
            activate: Some(handler),
            input: None,
            toggle: None,
            sort: None,
            pointer: None,
        })))
    }

    /// Creates an input-only binding for window or toolbar hosts.
    pub fn input(handler: InputHandler) -> Self {
        Self(Rc::new(RefCell::new(EventSlots {
            activate: None,
            input: Some(handler),
            toggle: None,
            sort: None,
            pointer: None,
        })))
    }

    pub(crate) fn replace(&self, handlers: &EventHandlers) {
        let mut slots = self.0.borrow_mut();
        slots.activate.clone_from(&handlers.activate);
        slots.input.clone_from(&handlers.input);
        slots.toggle.clone_from(&handlers.toggle);
        slots.sort.clone_from(&handlers.sort);
        slots.pointer.clone_from(&handlers.pointer);
    }

    /// Emits an activation event through the current handler.
    pub fn emit_activate(&self) {
        let handler = self.0.borrow().activate.clone();
        if let Some(handler) = handler {
            handler();
        }
    }

    /// Emits an edited value through the current handler.
    pub fn emit_input(&self, value: impl Into<String>) {
        let handler = self.0.borrow().input.clone();
        if let Some(handler) = handler {
            handler(value.into());
        }
    }

    /// Emits a binary state through the current handler.
    pub fn emit_toggle(&self, value: bool) {
        let handler = self.0.borrow().toggle.clone();
        if let Some(handler) = handler {
            handler(value);
        }
    }

    /// Emits a native table sort change through the current handler.
    pub fn emit_sort(&self, value: TableSort) {
        let handler = self.0.borrow().sort.clone();
        if let Some(handler) = handler {
            handler(value);
        }
    }

    /// Emits an element-local pointer event through the current handler.
    pub fn emit_pointer(&self, value: PointerEvent) {
        let handler = self.0.borrow().pointer.clone();
        if let Some(handler) = handler {
            handler(value);
        }
    }
}

impl fmt::Debug for EventBindings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let slots = self.0.borrow();
        formatter
            .debug_struct("EventBindings")
            .field("activate", &slots.activate.is_some())
            .field("input", &slots.input.is_some())
            .field("toggle", &slots.toggle.is_some())
            .field("sort", &slots.sort.is_some())
            .field("pointer", &slots.pointer.is_some())
            .finish()
    }
}
