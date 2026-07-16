//! Stable event slots shared by the reconciler and native adapters.

use crate::menu::ContextMenu;
use crate::{PointerEvent, TableSort, TextChange, TextSelection};
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
/// Callback used by multi-line text areas for native edit deltas.
pub type TextChangeHandler = Rc<dyn Fn(TextChange)>;
/// Callback used by multi-line text areas for native selection changes.
pub type SelectionChangeHandler = Rc<dyn Fn(TextSelection)>;

/// Event handlers associated with one declarative element.
#[derive(Clone, Default)]
pub struct EventHandlers {
    pub(crate) activate: Option<ActivateHandler>,
    pub(crate) input: Option<InputHandler>,
    pub(crate) toggle: Option<ToggleHandler>,
    pub(crate) sort: Option<SortHandler>,
    pub(crate) pointer: Option<PointerHandler>,
    /// The context-menu model rides with the handlers because its items carry
    /// activation closures that must stay current across renders.
    pub(crate) context_menu: Option<ContextMenu>,
    pub(crate) text_change: Option<TextChangeHandler>,
    pub(crate) selection_change: Option<SelectionChangeHandler>,
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
            .field("context_menu", &self.context_menu.is_some())
            .field("text_change", &self.text_change.is_some())
            .field("selection_change", &self.selection_change.is_some())
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
    context_menu: Option<ContextMenu>,
    text_change: Option<TextChangeHandler>,
    selection_change: Option<SelectionChangeHandler>,
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
            context_menu: handlers.context_menu.clone(),
            text_change: handlers.text_change.clone(),
            selection_change: handlers.selection_change.clone(),
        })))
    }

    /// Creates an activation-only binding for window or toolbar hosts.
    pub fn activate(handler: ActivateHandler) -> Self {
        Self(Rc::new(RefCell::new(EventSlots {
            activate: Some(handler),
            ..EventSlots::default()
        })))
    }

    /// Creates an input-only binding for window or toolbar hosts.
    pub fn input(handler: InputHandler) -> Self {
        Self(Rc::new(RefCell::new(EventSlots {
            input: Some(handler),
            ..EventSlots::default()
        })))
    }

    pub(crate) fn replace(&self, handlers: &EventHandlers) {
        let mut slots = self.0.borrow_mut();
        slots.activate.clone_from(&handlers.activate);
        slots.input.clone_from(&handlers.input);
        slots.toggle.clone_from(&handlers.toggle);
        slots.sort.clone_from(&handlers.sort);
        slots.pointer.clone_from(&handlers.pointer);
        slots.context_menu.clone_from(&handlers.context_menu);
        slots.text_change.clone_from(&handlers.text_change);
        slots
            .selection_change
            .clone_from(&handlers.selection_change);
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

    /// Dispatches the activation of one context-menu item through the current
    /// model and returns whether a handler ran.
    ///
    /// An unknown item, a disabled item, and an item inside a disabled
    /// submenu do not dispatch: a command the native menu refuses must not
    /// fire through the semantic model either.
    pub fn emit_context_menu_activation(&self, item_id: &str) -> bool {
        let handler = self
            .0
            .borrow()
            .context_menu
            .as_ref()
            .and_then(|menu| menu.activation_handler(item_id));
        if let Some(handler) = handler {
            handler();
            true
        } else {
            false
        }
    }

    /// Emits a native text-edit delta through the current handler.
    pub fn emit_text_change(&self, value: TextChange) {
        let handler = self.0.borrow().text_change.clone();
        if let Some(handler) = handler {
            handler(value);
        }
    }

    /// Emits a native selection change through the current handler.
    pub fn emit_selection_change(&self, value: TextSelection) {
        let handler = self.0.borrow().selection_change.clone();
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
            .field("context_menu", &slots.context_menu.is_some())
            .field("text_change", &slots.text_change.is_some())
            .field("selection_change", &slots.selection_change.is_some())
            .finish()
    }
}
