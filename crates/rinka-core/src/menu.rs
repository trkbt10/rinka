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

/// Declarative context menu attached to one element.
///
/// The platform opens it through its contextual interaction (secondary click,
/// ctrl-click, keyboard menu key, or the accessibility show-menu action) and
/// anchors it at the interaction point. Item and submenu identities share one
/// namespace across the whole menu; reconciliation validates their uniqueness
/// before any native mutation.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextMenu {
    /// Entries in display order.
    pub entries: Vec<MenuEntry>,
}

impl ContextMenu {
    /// Creates a context menu from entries in display order.
    pub fn new(entries: impl IntoIterator<Item = MenuEntry>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    /// Finds an item by identity regardless of its enabled state.
    pub fn find_item(&self, id: &str) -> Option<&MenuItem> {
        find_item_in(&self.entries, id)
    }

    /// Resolves the activation handler for one item identity.
    ///
    /// Returns `None` for an unknown item, a disabled item, or an item inside
    /// a disabled submenu: a command the native menu would refuse must not
    /// dispatch through the semantic model either.
    pub fn activation_handler(&self, id: &str) -> Option<ActivateHandler> {
        activation_handler_in(&self.entries, id, true)
    }

    /// Checks that every item and submenu identity is non-empty and unique
    /// within the whole menu.
    pub(crate) fn validate_identities(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        validate_identities_in(&self.entries, &mut seen)
    }
}

fn find_item_in<'entries>(entries: &'entries [MenuEntry], id: &str) -> Option<&'entries MenuItem> {
    for entry in entries {
        match entry {
            MenuEntry::Item(item) if item.id == id => return Some(item),
            MenuEntry::Item(_) | MenuEntry::Separator => {}
            MenuEntry::Submenu(submenu) => {
                if let Some(found) = find_item_in(&submenu.entries, id) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Resolves an enabled item's activation handler inside shared menu entries,
/// folding ancestor enabled state; shared with the menu bar model.
pub(crate) fn activation_handler_in(
    entries: &[MenuEntry],
    id: &str,
    ancestors_enabled: bool,
) -> Option<ActivateHandler> {
    for entry in entries {
        match entry {
            MenuEntry::Item(item) if item.id == id => {
                return (ancestors_enabled && item.enabled).then(|| item.on_activate.clone());
            }
            MenuEntry::Item(_) | MenuEntry::Separator => {}
            MenuEntry::Submenu(submenu) => {
                if let Some(handler) = activation_handler_in(
                    &submenu.entries,
                    id,
                    ancestors_enabled && submenu.enabled,
                ) {
                    return Some(handler);
                }
            }
        }
    }
    None
}

/// Checks identity uniqueness inside shared menu entries, collecting every
/// identity into `seen`; shared with the menu bar model.
pub(crate) fn validate_identities_in<'entries>(
    entries: &'entries [MenuEntry],
    seen: &mut std::collections::HashSet<&'entries str>,
) -> Result<(), String> {
    for entry in entries {
        match entry {
            MenuEntry::Item(item) => require_unique_identity(&item.id, seen)?,
            MenuEntry::Separator => {}
            MenuEntry::Submenu(submenu) => {
                require_unique_identity(&submenu.id, seen)?;
                validate_identities_in(&submenu.entries, seen)?;
            }
        }
    }
    Ok(())
}

fn require_unique_identity<'entries>(
    id: &'entries str,
    seen: &mut std::collections::HashSet<&'entries str>,
) -> Result<(), String> {
    if id.is_empty() {
        return Err("menu entry identity must not be empty".to_owned());
    }
    if !seen.insert(id) {
        return Err(format!("menu entry identity '{id}' is duplicated"));
    }
    Ok(())
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
