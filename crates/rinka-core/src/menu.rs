//! Declarative native menu vocabulary shared by toolbar and context menus.
//!
//! One menu model serves every native menu surface: toolbar dropdowns realize
//! it through their platform menu item, and element context menus attach it
//! through [`crate::Element::context_menu`]. Comparable declarative state and
//! activation handlers travel together, but equality intentionally ignores
//! handlers: reconciliation refreshes handlers on every render through stable
//! event bindings, so only the comparable state decides whether a native menu
//! must be patched.

use crate::chord::KeyChord;
use crate::{ActivateHandler, Symbol};
use std::fmt;
use std::rc::Rc;

/// Semantic role of one activatable menu item.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum MenuItemRole {
    /// Normal command.
    #[default]
    Standard,
    /// Command with destructive consequences, such as Delete.
    Destructive,
}

/// One activatable command inside a native menu.
#[derive(Clone)]
pub struct MenuItem {
    /// Stable identity, unique within the whole containing menu.
    pub id: String,
    /// Visible and accessible label.
    pub label: String,
    /// Optional platform symbol shown where the native menu supports one.
    pub symbol: Option<Symbol>,
    /// Hover help and accessible description.
    pub help: String,
    /// Whether the command is currently enabled.
    pub enabled: bool,
    /// Whether the item shows the platform checkmark.
    pub checked: bool,
    /// Semantic role translated to the platform treatment where one exists.
    pub role: MenuItemRole,
    /// Key chord shown as the native key equivalent where the platform menu
    /// supports one. Display only: app-wide delivery is owned by the
    /// window's accelerator table.
    pub chord: Option<KeyChord>,
    /// Activation handler refreshed by reconciliation on every render.
    pub on_activate: ActivateHandler,
}

impl MenuItem {
    /// Creates an enabled, unchecked, standard menu item.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        handler: impl Fn() + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            symbol: None,
            help: String::new(),
            enabled: true,
            checked: false,
            role: MenuItemRole::Standard,
            chord: None,
            on_activate: Rc::new(handler),
        }
    }

    /// Adds a platform symbol.
    pub fn symbol(mut self, symbol: Symbol) -> Self {
        self.symbol = Some(symbol);
        self
    }

    /// Adds hover help and an accessible description.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }

    /// Changes availability while preserving the item.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets the platform checkmark state.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Marks the command as destructive.
    pub fn destructive(mut self) -> Self {
        self.role = MenuItemRole::Destructive;
        self
    }

    /// Declares the key chord a hosting menu displays for this item.
    pub fn chord(mut self, chord: KeyChord) -> Self {
        self.chord = Some(chord);
        self
    }
}

impl PartialEq for MenuItem {
    /// Compares declarative state and intentionally ignores the activation
    /// handler; handlers are refreshed on every render regardless of equality.
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.label == other.label
            && self.symbol == other.symbol
            && self.help == other.help
            && self.enabled == other.enabled
            && self.checked == other.checked
            && self.role == other.role
            && self.chord == other.chord
    }
}

impl fmt::Debug for MenuItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MenuItem")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("symbol", &self.symbol)
            .field("help", &self.help)
            .field("enabled", &self.enabled)
            .field("checked", &self.checked)
            .field("role", &self.role)
            .field("chord", &self.chord.map(|chord| chord.to_string()))
            .finish_non_exhaustive()
    }
}

/// Nested native submenu.
#[derive(Clone, Debug, PartialEq)]
pub struct Submenu {
    /// Stable identity, unique within the whole containing menu.
    pub id: String,
    /// Visible and accessible label.
    pub label: String,
    /// Whether the submenu can be opened; a disabled submenu also prevents
    /// activation of every entry inside it.
    pub enabled: bool,
    /// Entries in display order.
    pub entries: Vec<MenuEntry>,
}

impl Submenu {
    /// Creates an enabled submenu.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        entries: impl IntoIterator<Item = MenuEntry>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: true,
            entries: entries.into_iter().collect(),
        }
    }

    /// Changes availability while preserving the submenu.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Entry in a native menu.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuEntry {
    /// Activatable command.
    Item(MenuItem),
    /// Native menu separator.
    Separator,
    /// Nested native submenu.
    Submenu(Submenu),
}

impl MenuEntry {
    /// Creates an activatable menu command.
    pub fn item(item: MenuItem) -> Self {
        Self::Item(item)
    }

    /// Creates a native separator.
    pub const fn separator() -> Self {
        Self::Separator
    }

    /// Creates a nested submenu.
    pub fn submenu(submenu: Submenu) -> Self {
        Self::Submenu(submenu)
    }
}

#[cfg(test)]
mod tests {
    use super::{MenuEntry, MenuItem, MenuItemRole, Submenu};

    #[test]
    fn equality_compares_declarative_state_and_ignores_handlers() {
        let first = MenuItem::new("delete", "Delete", || {}).destructive();
        let second = MenuItem::new("delete", "Delete", || panic!("never invoked")).destructive();
        assert_eq!(first, second);

        let disabled = MenuItem::new("delete", "Delete", || {})
            .destructive()
            .enabled(false);
        assert_ne!(first, disabled);
    }

    #[test]
    fn builder_methods_preserve_identity_and_set_state() {
        let item = MenuItem::new("toggle", "Toggle", || {})
            .checked(true)
            .help("Toggles the state")
            .enabled(false);
        assert_eq!(item.id, "toggle");
        assert!(item.checked);
        assert!(!item.enabled);
        assert_eq!(item.role, MenuItemRole::Standard);

        let submenu = Submenu::new(
            "more",
            "More",
            [MenuEntry::item(item), MenuEntry::separator()],
        )
        .enabled(false);
        assert_eq!(submenu.entries.len(), 2);
        assert!(!submenu.enabled);
    }
}
