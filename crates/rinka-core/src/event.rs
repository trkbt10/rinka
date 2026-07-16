//! Stable event slots shared by the reconciler and native adapters.

use crate::dock::{DockEvent, DockTabMenus};
use crate::drag::{DragPayload, DropTarget, FileDrop, FilePromise, PayloadDrop};
use crate::menu::ContextMenu;
use crate::{ImeEvent, KeyEvent, PointerEvent, TableSort, TextChange, TextSelection};
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
/// Callback used by elements accepting operating-system file drops.
pub type FileDropHandler = Rc<dyn Fn(FileDrop)>;
/// Callback used by elements accepting typed intra-application payloads.
pub type PayloadDropHandler = Rc<dyn Fn(PayloadDrop)>;
/// Callback used by input-accepting canvases for raw key-down events.
pub type KeyHandler = Rc<dyn Fn(KeyEvent)>;
/// Callback used by input-accepting canvases for IME composition events.
pub type ImeHandler = Rc<dyn Fn(ImeEvent)>;
/// Callback used by input-accepting canvases for focus changes; `true`
/// reports focus gained, `false` focus lost.
pub type FocusHandler = Rc<dyn Fn(bool)>;
/// Callback used by docks for semantic tab and split operation requests.
pub type DockHandler = Rc<dyn Fn(DockEvent)>;

/// Event handlers associated with one declarative element.
#[derive(Clone, Default)]
pub struct EventHandlers {
    pub(crate) activate: Option<ActivateHandler>,
    pub(crate) input: Option<InputHandler>,
    pub(crate) toggle: Option<ToggleHandler>,
    pub(crate) sort: Option<SortHandler>,
    pub(crate) pointer: Option<PointerHandler>,
    pub(crate) key: Option<KeyHandler>,
    pub(crate) ime: Option<ImeHandler>,
    pub(crate) focus: Option<FocusHandler>,
    /// The context-menu model rides with the handlers because its items carry
    /// activation closures that must stay current across renders.
    pub(crate) context_menu: Option<ContextMenu>,
    pub(crate) text_change: Option<TextChangeHandler>,
    pub(crate) selection_change: Option<SelectionChangeHandler>,
    pub(crate) file_drop: Option<FileDropHandler>,
    pub(crate) payload_drop: Option<PayloadDropHandler>,
    /// The file-promise model rides with the handlers because its write
    /// callback must stay current across renders, like menu activations.
    pub(crate) file_promise: Option<FilePromise>,
    pub(crate) drag_payload: Option<DragPayload>,
    pub(crate) drop_target: Option<DropTarget>,
    pub(crate) dock: Option<DockHandler>,
    /// The per-tab menu models ride with the handlers because their items
    /// carry activation closures that must stay current across renders.
    pub(crate) dock_tab_menus: Option<DockTabMenus>,
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
            .field("key", &self.key.is_some())
            .field("ime", &self.ime.is_some())
            .field("focus", &self.focus.is_some())
            .field("context_menu", &self.context_menu.is_some())
            .field("text_change", &self.text_change.is_some())
            .field("selection_change", &self.selection_change.is_some())
            .field("file_drop", &self.file_drop.is_some())
            .field("payload_drop", &self.payload_drop.is_some())
            .field("file_promise", &self.file_promise.is_some())
            .field("drag_payload", &self.drag_payload.is_some())
            .field("drop_target", &self.drop_target.is_some())
            .field("dock", &self.dock.is_some())
            .field("dock_tab_menus", &self.dock_tab_menus.is_some())
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
    key: Option<KeyHandler>,
    ime: Option<ImeHandler>,
    focus: Option<FocusHandler>,
    context_menu: Option<ContextMenu>,
    text_change: Option<TextChangeHandler>,
    selection_change: Option<SelectionChangeHandler>,
    file_drop: Option<FileDropHandler>,
    payload_drop: Option<PayloadDropHandler>,
    file_promise: Option<FilePromise>,
    drag_payload: Option<DragPayload>,
    drop_target: Option<DropTarget>,
    dock: Option<DockHandler>,
    dock_tab_menus: Option<DockTabMenus>,
}

impl EventSlots {
    fn from_handlers(handlers: &EventHandlers) -> Self {
        Self {
            activate: handlers.activate.clone(),
            input: handlers.input.clone(),
            toggle: handlers.toggle.clone(),
            sort: handlers.sort.clone(),
            pointer: handlers.pointer.clone(),
            context_menu: handlers.context_menu.clone(),
            text_change: handlers.text_change.clone(),
            selection_change: handlers.selection_change.clone(),
            key: handlers.key.clone(),
            ime: handlers.ime.clone(),
            focus: handlers.focus.clone(),
            file_drop: handlers.file_drop.clone(),
            payload_drop: handlers.payload_drop.clone(),
            file_promise: handlers.file_promise.clone(),
            drag_payload: handlers.drag_payload.clone(),
            drop_target: handlers.drop_target.clone(),
            dock: handlers.dock.clone(),
            dock_tab_menus: handlers.dock_tab_menus.clone(),
        }
    }
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
        Self(Rc::new(RefCell::new(EventSlots::from_handlers(handlers))))
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
        slots.key.clone_from(&handlers.key);
        slots.ime.clone_from(&handlers.ime);
        slots.focus.clone_from(&handlers.focus);
        slots.context_menu.clone_from(&handlers.context_menu);
        slots.text_change.clone_from(&handlers.text_change);
        slots
            .selection_change
            .clone_from(&handlers.selection_change);
        slots.file_drop.clone_from(&handlers.file_drop);
        slots.payload_drop.clone_from(&handlers.payload_drop);
        slots.file_promise.clone_from(&handlers.file_promise);
        slots.drag_payload.clone_from(&handlers.drag_payload);
        slots.drop_target.clone_from(&handlers.drop_target);
        slots.dock.clone_from(&handlers.dock);
        slots.dock_tab_menus.clone_from(&handlers.dock_tab_menus);
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

    /// Emits a raw key-down event through the current handler.
    pub fn emit_key(&self, value: KeyEvent) {
        let handler = self.0.borrow().key.clone();
        if let Some(handler) = handler {
            handler(value);
        }
    }

    /// Emits an IME composition event through the current handler.
    pub fn emit_ime(&self, value: ImeEvent) {
        let handler = self.0.borrow().ime.clone();
        if let Some(handler) = handler {
            handler(value);
        }
    }

    /// Emits a focus change through the current handler; `true` reports
    /// focus gained.
    pub fn emit_focus(&self, focused: bool) {
        let handler = self.0.borrow().focus.clone();
        if let Some(handler) = handler {
            handler(focused);
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
    /// Delivers dropped operating-system files through the current handler
    /// and returns whether the drop was consumed.
    ///
    /// An element whose current drop-target model does not accept files
    /// refuses the drop, matching the native adapters' refusal during the
    /// drag session's validation phase.
    pub fn emit_file_drop(&self, value: FileDrop) -> bool {
        let handler = {
            let slots = self.0.borrow();
            if !slots
                .drop_target
                .as_ref()
                .is_some_and(DropTarget::accepts_files)
            {
                return false;
            }
            slots.file_drop.clone()
        };
        if let Some(handler) = handler {
            handler(value);
            true
        } else {
            false
        }
    }

    /// Delivers a typed intra-application payload through the current
    /// handler and returns whether the drop was consumed.
    ///
    /// A payload whose type the current drop-target model does not accept is
    /// refused: a drop the native session's validation phase would reject
    /// must not fire through the semantic model either.
    pub fn emit_payload_drop(&self, value: PayloadDrop) -> bool {
        let handler = {
            let slots = self.0.borrow();
            if !slots
                .drop_target
                .as_ref()
                .is_some_and(|target| target.accepts_payload_type(value.payload.payload_type()))
            {
                return false;
            }
            slots.payload_drop.clone()
        };
        if let Some(handler) = handler {
            handler(value);
            true
        } else {
            false
        }
    }

    /// Emits a semantic dock operation request through the current handler
    /// and returns whether a handler consumed it.
    pub fn emit_dock(&self, event: DockEvent) -> bool {
        let handler = self.0.borrow().dock.clone();
        match handler {
            Some(handler) => {
                handler(event);
                true
            }
            None => false,
        }
    }

    /// Dispatches the activation of one dock tab's menu item through the
    /// current per-tab menu models and returns whether a handler ran.
    ///
    /// An unknown tab, an unknown item, a disabled item, and an item inside
    /// a disabled submenu do not dispatch, matching the element context-menu
    /// contract.
    pub fn emit_dock_tab_menu_activation(&self, tab_id: &str, item_id: &str) -> bool {
        let handler = self
            .0
            .borrow()
            .dock_tab_menus
            .as_ref()
            .and_then(|menus| menus.menu_for(tab_id))
            .and_then(|menu| menu.activation_handler(item_id));
        if let Some(handler) = handler {
            handler();
            true
        } else {
            false
        }
    }

    /// Returns the current per-tab dock menu models.
    pub fn dock_tab_menus(&self) -> Option<DockTabMenus> {
        self.0.borrow().dock_tab_menus.clone()
    }

    /// Returns the current file-promise drag-source model.
    ///
    /// Adapters read this at drag-session start and at promise
    /// materialization time, so the write callback is always the one from
    /// the latest render.
    pub fn file_promise(&self) -> Option<FilePromise> {
        self.0.borrow().file_promise.clone()
    }

    /// Returns the current typed-payload drag-source model.
    pub fn drag_payload(&self) -> Option<DragPayload> {
        self.0.borrow().drag_payload.clone()
    }

    /// Returns the current drop-target model.
    pub fn drop_target(&self) -> Option<DropTarget> {
        self.0.borrow().drop_target.clone()
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
            .field("key", &slots.key.is_some())
            .field("ime", &slots.ime.is_some())
            .field("focus", &slots.focus.is_some())
            .field("context_menu", &slots.context_menu.is_some())
            .field("text_change", &slots.text_change.is_some())
            .field("selection_change", &slots.selection_change.is_some())
            .field("file_drop", &slots.file_drop.is_some())
            .field("payload_drop", &slots.payload_drop.is_some())
            .field("file_promise", &slots.file_promise.is_some())
            .field("drag_payload", &slots.drag_payload.is_some())
            .field("drop_target", &slots.drop_target.is_some())
            .field("dock", &slots.dock.is_some())
            .field("dock_tab_menus", &slots.dock_tab_menus.is_some())
            .finish()
    }
}
