//! Declarative accelerator tables and platform-neutral chord routing.
//!
//! A window's content root declares its accelerator table like every other
//! element description. Reconciliation replaces the entries held by one
//! stable [`AcceleratorBindings`] value per renderer — the same stable-slot
//! discipline as [`crate::EventBindings`] — so a platform host connects its
//! native key source once and never reconnects it when entries change.

use crate::chord::KeyChord;
use crate::event::ActivateHandler;
use crate::window::WindowId;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Reach of one accelerator entry.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum AcceleratorScope {
    /// Fires only while the declaring window is the key window.
    #[default]
    Window,
    /// Fires regardless of which application window is key.
    Application,
}

impl fmt::Display for AcceleratorScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window => formatter.write_str("window"),
            Self::Application => formatter.write_str("application"),
        }
    }
}

/// One declarative chord-to-message binding.
///
/// The action is a message-dispatching closure exactly like a button handler;
/// it is replaced on every render while the native connection stays stable.
#[derive(Clone)]
pub struct Accelerator {
    id: String,
    chord: KeyChord,
    scope: AcceleratorScope,
    enabled: bool,
    global: bool,
    action: ActivateHandler,
}

impl Accelerator {
    /// Creates an enabled, window-scoped entry that defers to focused text input.
    pub fn new(id: impl Into<String>, chord: KeyChord, action: impl Fn() + 'static) -> Self {
        Self {
            id: id.into(),
            chord,
            scope: AcceleratorScope::Window,
            enabled: true,
            global: false,
            action: Rc::new(action),
        }
    }

    /// Changes the entry's reach.
    pub fn scope(mut self, scope: AcceleratorScope) -> Self {
        self.scope = scope;
        self
    }

    /// Changes availability while preserving the entry.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Declares that the chord fires even while a native text input has focus.
    ///
    /// Entries default to deferring to typing: a focused text field receives
    /// the key event and the accelerator is withheld.
    pub fn global(mut self, global: bool) -> Self {
        self.global = global;
        self
    }

    /// Returns the stable entry identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the declared chord.
    pub const fn chord(&self) -> KeyChord {
        self.chord
    }

    /// Returns the declared reach.
    pub const fn declared_scope(&self) -> AcceleratorScope {
        self.scope
    }

    /// Returns whether the entry currently fires.
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Returns whether the entry fires over focused text input.
    pub const fn is_global(&self) -> bool {
        self.global
    }

    /// Returns the comparable, handler-free description of this entry.
    pub fn description(&self) -> AcceleratorDescription {
        AcceleratorDescription {
            id: self.id.clone(),
            chord: self.chord,
            scope: self.scope,
            enabled: self.enabled,
            global: self.global,
        }
    }
}

impl fmt::Debug for Accelerator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Accelerator")
            .field("id", &self.id)
            .field("chord", &self.chord.to_string())
            .field("scope", &self.scope)
            .field("enabled", &self.enabled)
            .field("global", &self.global)
            .finish_non_exhaustive()
    }
}

/// Comparable snapshot of one accelerator entry without its handler.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AcceleratorDescription {
    /// Stable entry identity.
    pub id: String,
    /// Declared chord.
    pub chord: KeyChord,
    /// Declared reach.
    pub scope: AcceleratorScope,
    /// Whether the entry currently fires.
    pub enabled: bool,
    /// Whether the entry fires over focused text input.
    pub global: bool,
}

/// Stable accelerator table connected once by a platform host.
///
/// The reconciler replaces the stored entries after every successful render;
/// the platform's native key source keeps consulting the same value.
#[derive(Clone, Default)]
pub struct AcceleratorBindings(Rc<RefCell<Vec<Accelerator>>>);

impl AcceleratorBindings {
    pub(crate) fn replace(&self, entries: &[Accelerator]) {
        let mut slots = self.0.borrow_mut();
        slots.clear();
        slots.extend(entries.iter().cloned());
    }

    /// Returns comparable descriptions of the current entries.
    pub fn descriptions(&self) -> Vec<AcceleratorDescription> {
        self.0
            .borrow()
            .iter()
            .map(Accelerator::description)
            .collect()
    }

    /// Returns whether any entry is currently declared.
    pub fn has_entries(&self) -> bool {
        !self.0.borrow().is_empty()
    }

    /// Finds the first enabled entry in the requested scope whose chord
    /// matches, preferring entries applicable over focused text input.
    fn find(
        &self,
        chord: KeyChord,
        scope: AcceleratorScope,
        text_input_focused: bool,
    ) -> Option<TableMatch> {
        let slots = self.0.borrow();
        let mut withheld = None;
        for entry in slots
            .iter()
            .filter(|entry| entry.scope == scope && entry.enabled && entry.chord == chord)
        {
            if entry.global || !text_input_focused {
                return Some(TableMatch::Applicable {
                    id: entry.id.clone(),
                    action: entry.action.clone(),
                });
            }
            if withheld.is_none() {
                withheld = Some(TableMatch::Withheld {
                    id: entry.id.clone(),
                });
            }
        }
        withheld
    }
}

impl fmt::Debug for AcceleratorBindings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcceleratorBindings")
            .field("entries", &self.descriptions())
            .finish()
    }
}

enum TableMatch {
    Applicable { id: String, action: ActivateHandler },
    Withheld { id: String },
}

/// Focus facts a platform host supplies with each key event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyRoutingContext {
    /// Identity of the key window, if any application window is key.
    pub key_window: Option<WindowId>,
    /// Whether a native text input currently has keyboard focus.
    pub text_input_focused: bool,
}

/// Result of routing one chord through the registered tables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AcceleratorOutcome {
    /// An entry matched and its message was dispatched; the platform must
    /// consume the native event.
    Dispatched {
        /// Window whose table owned the fired entry.
        window: WindowId,
        /// Fired entry identity.
        accelerator: String,
    },
    /// An entry matched but was withheld because a text input has focus and
    /// the entry is not global; the platform must let typing proceed.
    WithheldForTextInput {
        /// Window whose table owned the withheld entry.
        window: WindowId,
        /// Withheld entry identity.
        accelerator: String,
    },
    /// No enabled entry matched; the platform must let the event fall through.
    Unmatched,
}

/// Application-wide chord router over per-window accelerator tables.
///
/// Precedence is: window-scoped entries of the key window first, then
/// application-scoped entries of every registered window in registration
/// order. Disabled entries never match and never withhold. A matched entry
/// that defers to focused text input stops its own table's window scope from
/// firing but does not shadow a global application-scoped entry.
#[derive(Debug, Default)]
pub struct AcceleratorRouter {
    windows: Vec<(WindowId, AcceleratorBindings)>,
}

impl AcceleratorRouter {
    /// Creates an empty router.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one window's stable table, replacing any previous
    /// registration under the same identity while keeping its precedence.
    pub fn register_window(&mut self, id: WindowId, bindings: AcceleratorBindings) {
        if let Some(slot) = self
            .windows
            .iter_mut()
            .find(|(existing, _)| *existing == id)
        {
            slot.1 = bindings;
        } else {
            self.windows.push((id, bindings));
        }
    }

    /// Removes one window's table.
    pub fn unregister_window(&mut self, id: &WindowId) {
        self.windows.retain(|(existing, _)| existing != id);
    }

    /// Routes one chord, invoking the matched entry's action before returning.
    ///
    /// The action runs after every internal borrow is released, so it may
    /// re-render and replace table entries. An action that must register or
    /// unregister windows on this very router (opening or closing a window)
    /// needs the caller to hold no outer borrow while it runs; such hosts
    /// use [`Self::resolve`] and invoke the returned action themselves.
    pub fn route(&self, chord: KeyChord, context: &KeyRoutingContext) -> AcceleratorOutcome {
        let (outcome, action) = self.resolve(chord, context);
        if let Some(action) = action {
            action();
        }
        outcome
    }

    /// Resolves one chord to its outcome and the matched entry's action,
    /// without invoking it.
    ///
    /// Splitting resolution from invocation lets a host release every borrow
    /// of this router before the action runs: an action may open or close
    /// windows, which registers and unregisters tables here.
    pub fn resolve(
        &self,
        chord: KeyChord,
        context: &KeyRoutingContext,
    ) -> (AcceleratorOutcome, Option<ActivateHandler>) {
        let mut withheld = None;
        let key_window_table = context.key_window.as_ref().and_then(|key_window| {
            self.windows
                .iter()
                .find(|(id, _)| id == key_window)
                .map(|(id, bindings)| (id.clone(), bindings))
        });
        if let Some((window, bindings)) = key_window_table {
            match bindings.find(chord, AcceleratorScope::Window, context.text_input_focused) {
                Some(TableMatch::Applicable { id, action }) => {
                    return (
                        AcceleratorOutcome::Dispatched {
                            window,
                            accelerator: id,
                        },
                        Some(action),
                    );
                }
                Some(TableMatch::Withheld { id }) => {
                    withheld = Some(AcceleratorOutcome::WithheldForTextInput {
                        window,
                        accelerator: id,
                    });
                }
                None => {}
            }
        }
        for (window, bindings) in &self.windows {
            match bindings.find(
                chord,
                AcceleratorScope::Application,
                context.text_input_focused,
            ) {
                Some(TableMatch::Applicable { id, action }) => {
                    return (
                        AcceleratorOutcome::Dispatched {
                            window: window.clone(),
                            accelerator: id,
                        },
                        Some(action),
                    );
                }
                Some(TableMatch::Withheld { id }) if withheld.is_none() => {
                    withheld = Some(AcceleratorOutcome::WithheldForTextInput {
                        window: window.clone(),
                        accelerator: id,
                    });
                }
                Some(TableMatch::Withheld { .. }) | None => {}
            }
        }
        (withheld.unwrap_or(AcceleratorOutcome::Unmatched), None)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Accelerator, AcceleratorBindings, AcceleratorOutcome, AcceleratorRouter, AcceleratorScope,
        KeyRoutingContext,
    };
    use crate::window::WindowId;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn chord(text: &str) -> crate::KeyChord {
        text.parse().expect("test chord")
    }

    fn recording_entry(id: &str, chord_text: &str, log: &Rc<RefCell<Vec<String>>>) -> Accelerator {
        let log = log.clone();
        let entry_id = id.to_owned();
        Accelerator::new(id, chord(chord_text), move || {
            log.borrow_mut().push(entry_id.clone());
        })
    }

    fn table(entries: Vec<Accelerator>) -> AcceleratorBindings {
        let bindings = AcceleratorBindings::default();
        bindings.replace(&entries);
        bindings
    }

    fn context(key_window: Option<&str>, text_input_focused: bool) -> KeyRoutingContext {
        KeyRoutingContext {
            key_window: key_window.map(WindowId::new),
            text_input_focused,
        }
    }

    #[test]
    fn the_key_windows_window_scope_precedes_application_scope() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut router = AcceleratorRouter::new();
        router.register_window(
            WindowId::new("editor"),
            table(vec![recording_entry("editor-save", "Primary+S", &log)]),
        );
        router.register_window(
            WindowId::new("library"),
            table(vec![
                recording_entry("library-save", "Primary+S", &log)
                    .scope(AcceleratorScope::Application),
            ]),
        );

        let outcome = router.route(chord("Primary+S"), &context(Some("editor"), false));
        assert_eq!(
            outcome,
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("editor"),
                accelerator: "editor-save".to_owned(),
            }
        );
        assert_eq!(*log.borrow(), vec!["editor-save".to_owned()]);
    }

    #[test]
    fn a_window_scoped_entry_does_not_fire_from_another_key_window() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut router = AcceleratorRouter::new();
        router.register_window(
            WindowId::new("editor"),
            table(vec![recording_entry("editor-save", "Primary+S", &log)]),
        );
        router.register_window(WindowId::new("library"), table(Vec::new()));

        let outcome = router.route(chord("Primary+S"), &context(Some("library"), false));
        assert_eq!(outcome, AcceleratorOutcome::Unmatched);
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn text_input_focus_withholds_unless_global() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut router = AcceleratorRouter::new();
        router.register_window(
            WindowId::new("editor"),
            table(vec![
                recording_entry("deferential", "Primary+D", &log),
                recording_entry("global", "Primary+G", &log).global(true),
            ]),
        );

        let focused = context(Some("editor"), true);
        assert_eq!(
            router.route(chord("Primary+D"), &focused),
            AcceleratorOutcome::WithheldForTextInput {
                window: WindowId::new("editor"),
                accelerator: "deferential".to_owned(),
            }
        );
        assert!(log.borrow().is_empty());
        assert_eq!(
            router.route(chord("Primary+G"), &focused),
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("editor"),
                accelerator: "global".to_owned(),
            }
        );
        assert_eq!(*log.borrow(), vec!["global".to_owned()]);
    }

    #[test]
    fn a_withheld_window_entry_does_not_shadow_a_global_application_entry() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut router = AcceleratorRouter::new();
        router.register_window(
            WindowId::new("editor"),
            table(vec![recording_entry("editor-find", "Primary+F", &log)]),
        );
        router.register_window(
            WindowId::new("library"),
            table(vec![
                recording_entry("library-find", "Primary+F", &log)
                    .scope(AcceleratorScope::Application)
                    .global(true),
            ]),
        );

        let outcome = router.route(chord("Primary+F"), &context(Some("editor"), true));
        assert_eq!(
            outcome,
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("library"),
                accelerator: "library-find".to_owned(),
            }
        );
        assert_eq!(*log.borrow(), vec!["library-find".to_owned()]);
    }

    #[test]
    fn disabled_entries_neither_fire_nor_withhold() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut router = AcceleratorRouter::new();
        router.register_window(
            WindowId::new("editor"),
            table(vec![
                recording_entry("disabled", "Primary+D", &log).enabled(false),
            ]),
        );

        assert_eq!(
            router.route(chord("Primary+D"), &context(Some("editor"), false)),
            AcceleratorOutcome::Unmatched
        );
        assert_eq!(
            router.route(chord("Primary+D"), &context(Some("editor"), true)),
            AcceleratorOutcome::Unmatched
        );
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn replacing_entries_keeps_the_stable_table_identity() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let bindings = table(vec![recording_entry("first", "Primary+1", &log)]);
        let mut router = AcceleratorRouter::new();
        router.register_window(WindowId::new("editor"), bindings.clone());

        bindings.replace(&[recording_entry("second", "Primary+2", &log)]);
        assert_eq!(
            router.route(chord("Primary+1"), &context(Some("editor"), false)),
            AcceleratorOutcome::Unmatched
        );
        assert_eq!(
            router.route(chord("Primary+2"), &context(Some("editor"), false)),
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("editor"),
                accelerator: "second".to_owned(),
            }
        );
    }

    #[test]
    fn descriptions_expose_the_comparable_entry_state() {
        let entry = Accelerator::new("toggle", chord("Primary+Shift+H"), || {})
            .scope(AcceleratorScope::Application)
            .enabled(false)
            .global(true);
        let description = entry.description();
        assert_eq!(description.id, "toggle");
        assert_eq!(description.chord.to_string(), "Primary+Shift+H");
        assert_eq!(description.scope, AcceleratorScope::Application);
        assert!(!description.enabled);
        assert!(description.global);
    }
}
