//! Declarative application menu bar built from the shared menu vocabulary.
//!
//! One application-global bar (File/Edit/View/Window/Help) is declared either
//! on [`crate::ApplicationSpec`] — the app-level description installed at
//! startup — or, for state-driven bars, redeclared by a window's content root
//! through [`crate::Element::menu_bar`] and reconciled like every other
//! declaration. App-defined entries reuse the shared [`MenuItem`] and
//! [`Submenu`] vocabulary; platform-owned commands are typed
//! [`StandardItem`] roles so consumers never fake native behavior.
//!
//! Routing contract: the *focused window's* declaration is the effective bar.
//! [`MenuBarRouter`] resolves the key window's stable [`MenuBarBindings`]
//! first, falls back to the first registered window that declares a bar (the
//! main window), and finally to the application-level bar. An activation is
//! dispatched through the handler found in the effective bar, so a menu
//! command reaches the focused window's component through its own queued
//! message delivery, and switching focus redirects delivery.

use crate::chord::KeyChord;
use crate::event::ActivateHandler;
use crate::menu::{MenuEntry, MenuItem, Submenu, activation_handler_in, validate_identities_in};
use crate::window::WindowId;
use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::rc::Rc;

/// Platform-owned menu command realized by the native host.
///
/// Standard roles pass through to the platform's own dispatch — on macOS a
/// nil-target selector down the responder chain — so native text editing and
/// window management work with zero consumer code. `About` and `Quit` live in
/// the platform's application menu on macOS regardless of where they are
/// declared; the declared position is used by hosts whose conventions keep
/// them in File or Help.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StandardItem {
    /// Shows the native about panel.
    About,
    /// Terminates the application through the native quit path.
    Quit,
    /// Closes the key window.
    CloseWindow,
    /// Miniaturizes the key window.
    Minimize,
    /// Undoes the focused editor's last change.
    Undo,
    /// Redoes the focused editor's last undone change.
    Redo,
    /// Cuts the focused selection to the clipboard.
    Cut,
    /// Copies the focused selection to the clipboard.
    Copy,
    /// Pastes the clipboard into the focused editor.
    Paste,
    /// Selects the focused editor's complete content.
    SelectAll,
}

impl StandardItem {
    /// Stable identity used in diagnostics and activation logs.
    pub const fn id(self) -> &'static str {
        match self {
            Self::About => "standard-about",
            Self::Quit => "standard-quit",
            Self::CloseWindow => "standard-close-window",
            Self::Minimize => "standard-minimize",
            Self::Undo => "standard-undo",
            Self::Redo => "standard-redo",
            Self::Cut => "standard-cut",
            Self::Copy => "standard-copy",
            Self::Paste => "standard-paste",
            Self::SelectAll => "standard-select-all",
        }
    }

    /// Platform-conventional key chord the native item binds and displays.
    ///
    /// `About` has no conventional chord. The chords participate in the
    /// bar-wide duplicate-chord validation so an app-defined item cannot
    /// silently shadow a standard editing command.
    pub fn canonical_chord(self) -> Option<KeyChord> {
        let text = match self {
            Self::About => return None,
            Self::Quit => "Primary+Q",
            Self::CloseWindow => "Primary+W",
            Self::Minimize => "Primary+M",
            Self::Undo => "Primary+Z",
            Self::Redo => "Primary+Shift+Z",
            Self::Cut => "Primary+X",
            Self::Copy => "Primary+C",
            Self::Paste => "Primary+V",
            Self::SelectAll => "Primary+A",
        };
        Some(text.parse().expect("standard chords are canonical"))
    }
}

/// Entry in one top-level menu of the application menu bar.
///
/// App-defined entries reuse the shared menu vocabulary — [`MenuItem`] with
/// its display [`MenuItem::chord`], separators, and nested [`Submenu`] — and
/// platform-owned commands are typed [`StandardItem`] roles.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuBarEntry {
    /// App-defined command dispatching through the focused window's handlers.
    Item(MenuItem),
    /// Platform-owned command realized natively.
    Standard(StandardItem),
    /// Native separator.
    Separator,
    /// Nested app-defined submenu.
    Submenu(Submenu),
}

impl MenuBarEntry {
    /// Creates an app-defined menu command.
    pub fn item(item: MenuItem) -> Self {
        Self::Item(item)
    }

    /// Creates a platform-owned standard command.
    pub const fn standard(item: StandardItem) -> Self {
        Self::Standard(item)
    }

    /// Creates a native separator.
    pub const fn separator() -> Self {
        Self::Separator
    }

    /// Creates a nested app-defined submenu.
    pub fn submenu(submenu: Submenu) -> Self {
        Self::Submenu(submenu)
    }
}

impl From<MenuEntry> for MenuBarEntry {
    fn from(entry: MenuEntry) -> Self {
        match entry {
            MenuEntry::Item(item) => Self::Item(item),
            MenuEntry::Separator => Self::Separator,
            MenuEntry::Submenu(submenu) => Self::Submenu(submenu),
        }
    }
}

/// Native behavior a platform attaches to one top-level menu.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum MenuBarMenuRole {
    /// Plain menu with only its declared entries.
    #[default]
    Custom,
    /// The window-management menu; the platform appends its native window
    /// list and window behaviors (macOS `windowsMenu`).
    Window,
    /// The help menu; the platform adds its native help affordances
    /// (macOS `helpMenu` with the search field).
    Help,
}

/// One top-level menu of the application menu bar.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuBarMenu {
    /// Stable identity, unique within the whole menu bar.
    pub id: String,
    /// Visible and accessible title.
    pub label: String,
    /// Native behavior the platform attaches to this menu.
    pub role: MenuBarMenuRole,
    /// Entries in display order.
    pub entries: Vec<MenuBarEntry>,
}

impl MenuBarMenu {
    /// Creates a plain top-level menu.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        entries: impl IntoIterator<Item = MenuBarEntry>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            role: MenuBarMenuRole::Custom,
            entries: entries.into_iter().collect(),
        }
    }

    /// Attaches a native menu role.
    pub const fn role(mut self, role: MenuBarMenuRole) -> Self {
        self.role = role;
        self
    }
}

/// Declarative application menu bar.
///
/// Item and submenu identities share one namespace across the whole bar;
/// validation rejects duplicates before any native realization, and a chord
/// may be bound by at most one item — standard items' canonical chords
/// included.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuBar {
    /// Top-level menus in display order.
    pub menus: Vec<MenuBarMenu>,
}

impl MenuBar {
    /// Creates a menu bar from top-level menus in display order.
    pub fn new(menus: impl IntoIterator<Item = MenuBarMenu>) -> Self {
        Self {
            menus: menus.into_iter().collect(),
        }
    }

    /// Returns whether the bar declares no menus.
    pub fn is_empty(&self) -> bool {
        self.menus.is_empty()
    }

    /// Finds an app-defined item by identity regardless of enabled state.
    pub fn find_item(&self, id: &str) -> Option<&MenuItem> {
        self.menus
            .iter()
            .flat_map(|menu| &menu.entries)
            .find_map(|entry| match entry {
                MenuBarEntry::Item(item) if item.id == id => Some(item),
                MenuBarEntry::Item(_) | MenuBarEntry::Standard(_) | MenuBarEntry::Separator => None,
                MenuBarEntry::Submenu(submenu) => find_item_in_entries(&submenu.entries, id),
            })
    }

    /// Resolves the activation handler for one app-defined item identity.
    ///
    /// Returns `None` for an unknown item, a disabled item, or an item inside
    /// a disabled submenu: a command the native menu would refuse must not
    /// dispatch through the semantic model either.
    pub fn activation_handler(&self, id: &str) -> Option<ActivateHandler> {
        for menu in &self.menus {
            for entry in &menu.entries {
                match entry {
                    MenuBarEntry::Item(item) if item.id == id => {
                        return item.enabled.then(|| item.on_activate.clone());
                    }
                    MenuBarEntry::Item(_) | MenuBarEntry::Standard(_) | MenuBarEntry::Separator => {
                    }
                    MenuBarEntry::Submenu(submenu) => {
                        if let Some(handler) =
                            activation_handler_in(&submenu.entries, id, submenu.enabled)
                        {
                            return Some(handler);
                        }
                    }
                }
            }
        }
        None
    }

    /// Returns whether an app-defined item is currently activatable,
    /// folding the enabled state of every enclosing submenu.
    ///
    /// `None` means the identity is unknown to this bar.
    pub fn item_enabled(&self, id: &str) -> Option<bool> {
        for menu in &self.menus {
            for entry in &menu.entries {
                match entry {
                    MenuBarEntry::Item(item) if item.id == id => return Some(item.enabled),
                    MenuBarEntry::Item(_) | MenuBarEntry::Standard(_) | MenuBarEntry::Separator => {
                    }
                    MenuBarEntry::Submenu(submenu) => {
                        if let Some(enabled) =
                            item_enabled_in_entries(&submenu.entries, id, submenu.enabled)
                        {
                            return Some(enabled);
                        }
                    }
                }
            }
        }
        None
    }

    /// Returns whether any item in the bar binds this chord.
    ///
    /// A claimed chord is menu-owned: the platform must let its native menu
    /// key-equivalent dispatch handle the event (which natively fires over
    /// focused text input), and same-chord window accelerator entries are
    /// shadowed. The claim holds for disabled items too — a disabled menu
    /// command refuses its chord natively instead of falling through to a
    /// second dispatcher.
    pub fn claims_chord(&self, chord: KeyChord) -> bool {
        self.menus.iter().any(|menu| {
            menu.entries.iter().any(|entry| match entry {
                MenuBarEntry::Item(item) => item.chord == Some(chord),
                MenuBarEntry::Standard(standard) => standard.canonical_chord() == Some(chord),
                MenuBarEntry::Separator => false,
                MenuBarEntry::Submenu(submenu) => entries_claim_chord(&submenu.entries, chord),
            })
        })
    }

    /// Checks identity uniqueness and chord uniqueness across the whole bar.
    ///
    /// Hosts validate the application-level declaration at startup;
    /// window-root declarations are checked by tree validation before any
    /// native mutation.
    pub fn validate(&self) -> Result<(), String> {
        let mut identities = HashSet::new();
        let mut chords = HashSet::new();
        for menu in &self.menus {
            if menu.id.is_empty() {
                return Err("menu bar menu identity must not be empty".to_owned());
            }
            if !identities.insert(menu.id.clone()) {
                return Err(format!("menu bar identity '{}' is duplicated", menu.id));
            }
            validate_entries(&menu.entries, &mut identities, &mut chords)?;
        }
        Ok(())
    }

    /// Returns whether two bars share one native structure, meaning a
    /// retained native menu tree can be updated in place instead of rebuilt.
    pub fn structure_matches(&self, other: &Self) -> bool {
        self.menus.len() == other.menus.len()
            && self.menus.iter().zip(&other.menus).all(|(left, right)| {
                left.id == right.id
                    && left.role == right.role
                    && bar_entries_structure_matches(&left.entries, &right.entries)
            })
    }

    /// Plans how a host updates its retained native realization from
    /// `previous` to `next`.
    pub fn plan_update(previous: &Self, next: &Self) -> MenuBarUpdate {
        if previous == next {
            MenuBarUpdate::Unchanged
        } else if previous.structure_matches(next) {
            MenuBarUpdate::RefreshInPlace
        } else {
            MenuBarUpdate::Rebuild
        }
    }
}

/// Native mutation a menu bar model change requires.
///
/// This is the recorded reconciliation plan: comparable state changes on an
/// unchanged structure refresh the retained native items in place, structural
/// changes rebuild the native tree, and handler-only changes require no
/// native mutation at all because activation resolves through the live
/// bindings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuBarUpdate {
    /// The comparable state is identical; no native mutation.
    Unchanged,
    /// Same structure: update titles, checkmarks, enabled state, and key
    /// equivalents on the retained native items.
    RefreshInPlace,
    /// Structure changed: rebuild the native menu tree.
    Rebuild,
}

fn bar_entries_structure_matches(current: &[MenuBarEntry], next: &[MenuBarEntry]) -> bool {
    current.len() == next.len()
        && current.iter().zip(next).all(|pair| match pair {
            (MenuBarEntry::Separator, MenuBarEntry::Separator) => true,
            (MenuBarEntry::Item(current), MenuBarEntry::Item(next)) => current.id == next.id,
            (MenuBarEntry::Standard(current), MenuBarEntry::Standard(next)) => current == next,
            (MenuBarEntry::Submenu(current), MenuBarEntry::Submenu(next)) => {
                current.id == next.id && entries_structure_matches(&current.entries, &next.entries)
            }
            _ => false,
        })
}

fn entries_structure_matches(current: &[MenuEntry], next: &[MenuEntry]) -> bool {
    current.len() == next.len()
        && current.iter().zip(next).all(|pair| match pair {
            (MenuEntry::Separator, MenuEntry::Separator) => true,
            (MenuEntry::Item(current), MenuEntry::Item(next)) => current.id == next.id,
            (MenuEntry::Submenu(current), MenuEntry::Submenu(next)) => {
                current.id == next.id && entries_structure_matches(&current.entries, &next.entries)
            }
            _ => false,
        })
}

fn find_item_in_entries<'entries>(
    entries: &'entries [MenuEntry],
    id: &str,
) -> Option<&'entries MenuItem> {
    for entry in entries {
        match entry {
            MenuEntry::Item(item) if item.id == id => return Some(item),
            MenuEntry::Item(_) | MenuEntry::Separator => {}
            MenuEntry::Submenu(submenu) => {
                if let Some(found) = find_item_in_entries(&submenu.entries, id) {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn item_enabled_in_entries(
    entries: &[MenuEntry],
    id: &str,
    ancestors_enabled: bool,
) -> Option<bool> {
    for entry in entries {
        match entry {
            MenuEntry::Item(item) if item.id == id => {
                return Some(ancestors_enabled && item.enabled);
            }
            MenuEntry::Item(_) | MenuEntry::Separator => {}
            MenuEntry::Submenu(submenu) => {
                if let Some(enabled) = item_enabled_in_entries(
                    &submenu.entries,
                    id,
                    ancestors_enabled && submenu.enabled,
                ) {
                    return Some(enabled);
                }
            }
        }
    }
    None
}

fn entries_claim_chord(entries: &[MenuEntry], chord: KeyChord) -> bool {
    entries.iter().any(|entry| match entry {
        MenuEntry::Item(item) => item.chord == Some(chord),
        MenuEntry::Separator => false,
        MenuEntry::Submenu(submenu) => entries_claim_chord(&submenu.entries, chord),
    })
}

fn validate_entries(
    entries: &[MenuBarEntry],
    identities: &mut HashSet<String>,
    chords: &mut HashSet<KeyChord>,
) -> Result<(), String> {
    for entry in entries {
        match entry {
            MenuBarEntry::Item(item) => {
                require_unique_bar_identity(&item.id, identities)?;
                if let Some(chord) = item.chord {
                    require_unique_bar_chord(chord, chords)?;
                }
            }
            MenuBarEntry::Standard(standard) => {
                require_unique_bar_identity(standard.id(), identities)?;
                if let Some(chord) = standard.canonical_chord() {
                    require_unique_bar_chord(chord, chords)?;
                }
            }
            MenuBarEntry::Separator => {}
            MenuBarEntry::Submenu(submenu) => {
                require_unique_bar_identity(&submenu.id, identities)?;
                let mut nested = HashSet::new();
                validate_identities_in(&submenu.entries, &mut nested)?;
                for id in nested {
                    require_unique_bar_identity(id, identities)?;
                }
                validate_entry_chords(&submenu.entries, chords)?;
            }
        }
    }
    Ok(())
}

fn validate_entry_chords(
    entries: &[MenuEntry],
    chords: &mut HashSet<KeyChord>,
) -> Result<(), String> {
    for entry in entries {
        match entry {
            MenuEntry::Item(item) => {
                if let Some(chord) = item.chord {
                    require_unique_bar_chord(chord, chords)?;
                }
            }
            MenuEntry::Separator => {}
            MenuEntry::Submenu(submenu) => validate_entry_chords(&submenu.entries, chords)?,
        }
    }
    Ok(())
}

fn require_unique_bar_identity(id: &str, identities: &mut HashSet<String>) -> Result<(), String> {
    if id.is_empty() {
        return Err("menu bar entry identity must not be empty".to_owned());
    }
    if !identities.insert(id.to_owned()) {
        return Err(format!("menu bar identity '{id}' is duplicated"));
    }
    Ok(())
}

fn require_unique_bar_chord(chord: KeyChord, chords: &mut HashSet<KeyChord>) -> Result<(), String> {
    if !chords.insert(chord) {
        return Err(format!("menu bar chord '{chord}' is bound more than once"));
    }
    Ok(())
}

/// Stable menu bar slot connected once by a platform host.
///
/// The reconciler replaces the stored model after every successful render —
/// the same stable-slot discipline as [`crate::AcceleratorBindings`] — so a
/// host resolves the current declaration and its current handlers at
/// activation time without ever reconnecting anything native.
#[derive(Clone, Default)]
pub struct MenuBarBindings(Rc<RefCell<Option<MenuBar>>>);

impl MenuBarBindings {
    pub(crate) fn replace(&self, model: Option<MenuBar>) {
        *self.0.borrow_mut() = model;
    }

    /// Returns whether a menu bar is currently declared.
    pub fn has_model(&self) -> bool {
        self.0.borrow().is_some()
    }

    /// Returns the current declared model.
    pub fn model(&self) -> Option<MenuBar> {
        self.0.borrow().clone()
    }
}

impl fmt::Debug for MenuBarBindings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MenuBarBindings")
            .field("declared", &self.has_model())
            .finish()
    }
}

/// Outcome of routing one menu bar activation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MenuBarActivation {
    /// The item's handler ran. `owner` names the window whose declaration
    /// dispatched — the focused window when it declares a bar, otherwise the
    /// fallback declarer — and is `None` when the application-level bar
    /// handled the activation.
    Dispatched {
        /// Window whose declaration owned the fired handler.
        owner: Option<WindowId>,
    },
    /// The item exists but is disabled, directly or through a disabled
    /// enclosing submenu; nothing dispatched.
    Refused,
    /// No effective bar declares this identity; nothing dispatched.
    Unknown,
}

/// Application-wide resolver from focus facts to the effective menu bar.
///
/// Resolution order: the key window's declared bar, then the first registered
/// window that declares one (the main window), then the application-level
/// bar. The effective bar supplies both the native realization a host
/// installs and the handlers an activation dispatches through, so the focused
/// window's component receives menu messages through its own queued delivery
/// and switching focus redirects delivery.
#[derive(Debug)]
pub struct MenuBarRouter {
    windows: Vec<(WindowId, MenuBarBindings)>,
    application: MenuBar,
}

impl MenuBarRouter {
    /// Creates a router over the application-level declaration.
    pub fn new(application: MenuBar) -> Self {
        Self {
            windows: Vec::new(),
            application,
        }
    }

    /// Registers one window's stable menu bar slot, replacing any previous
    /// registration under the same identity while keeping its precedence.
    pub fn register_window(&mut self, id: WindowId, bindings: MenuBarBindings) {
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

    /// Removes one window's slot.
    pub fn unregister_window(&mut self, id: &WindowId) {
        self.windows.retain(|(existing, _)| existing != id);
    }

    fn with_effective<R>(
        &self,
        key_window: Option<&WindowId>,
        read: impl FnOnce(Option<&WindowId>, &MenuBar) -> R,
    ) -> Option<R> {
        if let Some(key_window) = key_window
            && let Some((id, bindings)) = self
                .windows
                .iter()
                .find(|(id, bindings)| id == key_window && bindings.has_model())
        {
            let model = bindings.0.borrow();
            let model = model.as_ref().expect("has_model guarded the slot");
            return Some(read(Some(id), model));
        }
        if let Some((id, bindings)) = self
            .windows
            .iter()
            .find(|(_, bindings)| bindings.has_model())
        {
            let model = bindings.0.borrow();
            let model = model.as_ref().expect("has_model guarded the slot");
            return Some(read(Some(id), model));
        }
        if self.application.is_empty() {
            None
        } else {
            Some(read(None, &self.application))
        }
    }

    /// Returns the declaration a host must install for the current focus:
    /// the owning window (or `None` for the application-level bar) and a
    /// snapshot of the model.
    pub fn effective_model(
        &self,
        key_window: Option<&WindowId>,
    ) -> Option<(Option<WindowId>, MenuBar)> {
        self.with_effective(key_window, |owner, model| (owner.cloned(), model.clone()))
    }

    /// Returns whether the effective bar binds this chord (menu-owned; the
    /// platform must let native menu dispatch handle the event).
    pub fn claims_chord(&self, key_window: Option<&WindowId>, chord: KeyChord) -> bool {
        self.with_effective(key_window, |_, model| model.claims_chord(chord))
            .unwrap_or(false)
    }

    /// Returns whether an app-defined item on the effective bar is currently
    /// activatable; unknown identities are not.
    pub fn item_enabled(&self, key_window: Option<&WindowId>, id: &str) -> bool {
        self.with_effective(key_window, |_, model| {
            model.item_enabled(id).unwrap_or(false)
        })
        .unwrap_or(false)
    }

    /// Activates one app-defined item on the effective bar, running its
    /// current handler before returning.
    ///
    /// The handler is resolved from the live bindings, so a stale native menu
    /// dispatches through the freshest declaration, and a command the model
    /// currently refuses is never dispatched.
    pub fn activate(&self, key_window: Option<&WindowId>, item_id: &str) -> MenuBarActivation {
        let resolved = self.with_effective(key_window, |owner, model| {
            if model.find_item(item_id).is_none() {
                return (None, MenuBarActivation::Unknown);
            }
            match model.activation_handler(item_id) {
                Some(handler) => (
                    Some(handler),
                    MenuBarActivation::Dispatched {
                        owner: owner.cloned(),
                    },
                ),
                None => (None, MenuBarActivation::Refused),
            }
        });
        let Some((handler, outcome)) = resolved else {
            return MenuBarActivation::Unknown;
        };
        // The handler runs after every internal borrow is released, so it may
        // re-render and replace the declared model.
        if let Some(handler) = handler {
            handler();
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MenuBar, MenuBarActivation, MenuBarBindings, MenuBarEntry, MenuBarMenu, MenuBarMenuRole,
        MenuBarRouter, MenuBarUpdate, StandardItem,
    };
    use crate::menu::{MenuEntry, MenuItem, Submenu};
    use crate::window::WindowId;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn chord(text: &str) -> crate::KeyChord {
        text.parse().expect("test chord")
    }

    fn recording_item(id: &str, label: &str, log: &Rc<RefCell<Vec<String>>>) -> MenuItem {
        let log = log.clone();
        let recorded = id.to_owned();
        MenuItem::new(id, label, move || log.borrow_mut().push(recorded.clone()))
    }

    fn file_menu(log: &Rc<RefCell<Vec<String>>>) -> MenuBarMenu {
        MenuBarMenu::new(
            "file",
            "File",
            [
                MenuBarEntry::item(recording_item("new-folder", "New Folder", log)),
                MenuBarEntry::separator(),
                MenuBarEntry::standard(StandardItem::CloseWindow),
            ],
        )
    }

    #[test]
    fn equality_compares_declarative_state_and_ignores_handlers() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let first = MenuBar::new([file_menu(&log)]);
        let second = MenuBar::new([MenuBarMenu::new(
            "file",
            "File",
            [
                MenuBarEntry::item(MenuItem::new("new-folder", "New Folder", || {
                    panic!("never invoked")
                })),
                MenuBarEntry::separator(),
                MenuBarEntry::standard(StandardItem::CloseWindow),
            ],
        )]);
        assert_eq!(first, second);
        assert_eq!(
            MenuBar::plan_update(&first, &second),
            MenuBarUpdate::Unchanged
        );
    }

    #[test]
    fn plan_update_refreshes_state_changes_and_rebuilds_structure_changes() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let base = MenuBar::new([file_menu(&log)]);

        let relabeled = MenuBar::new([MenuBarMenu::new(
            "file",
            "File",
            [
                MenuBarEntry::item(recording_item("new-folder", "New Folder Here", &log)),
                MenuBarEntry::separator(),
                MenuBarEntry::standard(StandardItem::CloseWindow),
            ],
        )]);
        assert_eq!(
            MenuBar::plan_update(&base, &relabeled),
            MenuBarUpdate::RefreshInPlace
        );

        let disabled = MenuBar::new([MenuBarMenu::new(
            "file",
            "File",
            [
                MenuBarEntry::item(recording_item("new-folder", "New Folder", &log).enabled(false)),
                MenuBarEntry::separator(),
                MenuBarEntry::standard(StandardItem::CloseWindow),
            ],
        )]);
        assert_eq!(
            MenuBar::plan_update(&base, &disabled),
            MenuBarUpdate::RefreshInPlace
        );

        let grown = MenuBar::new([MenuBarMenu::new(
            "file",
            "File",
            [
                MenuBarEntry::item(recording_item("new-folder", "New Folder", &log)),
                MenuBarEntry::item(recording_item("open", "Open", &log)),
                MenuBarEntry::separator(),
                MenuBarEntry::standard(StandardItem::CloseWindow),
            ],
        )]);
        assert_eq!(MenuBar::plan_update(&base, &grown), MenuBarUpdate::Rebuild);

        let role_changed = MenuBar::new([file_menu(&log).role(MenuBarMenuRole::Window)]);
        assert_eq!(
            MenuBar::plan_update(&base, &role_changed),
            MenuBarUpdate::Rebuild
        );
    }

    #[test]
    fn activation_respects_item_and_submenu_enabled_state() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let bar = MenuBar::new([MenuBarMenu::new(
            "view",
            "View",
            [
                MenuBarEntry::item(recording_item("live", "Live", &log)),
                MenuBarEntry::item(recording_item("dead", "Dead", &log).enabled(false)),
                MenuBarEntry::submenu(
                    Submenu::new(
                        "nested",
                        "Nested",
                        [MenuEntry::item(recording_item("inner", "Inner", &log))],
                    )
                    .enabled(false),
                ),
            ],
        )]);

        assert!(bar.activation_handler("live").is_some());
        assert!(bar.activation_handler("dead").is_none());
        assert!(bar.activation_handler("inner").is_none());
        assert_eq!(bar.item_enabled("live"), Some(true));
        assert_eq!(bar.item_enabled("dead"), Some(false));
        assert_eq!(bar.item_enabled("inner"), Some(false));
        assert_eq!(bar.item_enabled("unknown"), None);
    }

    #[test]
    fn chords_are_claimed_for_app_defined_and_standard_items() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let bar = MenuBar::new([
            MenuBarMenu::new(
                "file",
                "File",
                [MenuBarEntry::item(
                    recording_item("new-folder", "New Folder", &log).chord(chord("Primary+N")),
                )],
            ),
            MenuBarMenu::new("edit", "Edit", [MenuBarEntry::standard(StandardItem::Copy)]),
        ]);
        assert!(bar.claims_chord(chord("Primary+N")));
        assert!(bar.claims_chord(chord("Primary+C")));
        assert!(!bar.claims_chord(chord("Primary+Shift+H")));
    }

    #[test]
    fn validation_rejects_duplicate_identities_and_chords() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let duplicate_id = MenuBar::new([
            MenuBarMenu::new(
                "file",
                "File",
                [MenuBarEntry::item(recording_item("open", "Open", &log))],
            ),
            MenuBarMenu::new(
                "edit",
                "Edit",
                [MenuBarEntry::item(recording_item(
                    "open",
                    "Open Again",
                    &log,
                ))],
            ),
        ]);
        assert!(duplicate_id.validate().is_err());

        let duplicate_chord = MenuBar::new([MenuBarMenu::new(
            "edit",
            "Edit",
            [
                MenuBarEntry::standard(StandardItem::Copy),
                MenuBarEntry::item(
                    recording_item("shadow-copy", "Shadow Copy", &log).chord(chord("Primary+C")),
                ),
            ],
        )]);
        assert!(duplicate_chord.validate().is_err());

        let valid = MenuBar::new([file_menu(&log)]);
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn the_router_prefers_the_key_window_and_falls_back_in_registration_order() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let main_bindings = MenuBarBindings::default();
        main_bindings.replace(Some(MenuBar::new([MenuBarMenu::new(
            "file",
            "File",
            [MenuBarEntry::item(recording_item("command", "Main", &log))],
        )])));
        let secondary_bindings = MenuBarBindings::default();
        secondary_bindings.replace(Some(MenuBar::new([MenuBarMenu::new(
            "file",
            "File",
            [MenuBarEntry::item(recording_item(
                "command",
                "Secondary",
                &log,
            ))],
        )])));
        let panel_bindings = MenuBarBindings::default();

        let mut router = MenuBarRouter::new(MenuBar::default());
        router.register_window(WindowId::new("main"), main_bindings);
        router.register_window(WindowId::new("secondary"), secondary_bindings);
        router.register_window(WindowId::new("panel"), panel_bindings);

        let (owner, model) = router
            .effective_model(Some(&WindowId::new("secondary")))
            .expect("secondary declares a bar");
        assert_eq!(owner, Some(WindowId::new("secondary")));
        assert_eq!(model.menus[0].label, "File");

        // A focused window without a declaration delegates to the first
        // declaring window.
        let (owner, _) = router
            .effective_model(Some(&WindowId::new("panel")))
            .expect("fallback declares a bar");
        assert_eq!(owner, Some(WindowId::new("main")));

        assert_eq!(
            router.activate(Some(&WindowId::new("secondary")), "command"),
            MenuBarActivation::Dispatched {
                owner: Some(WindowId::new("secondary")),
            }
        );
        assert_eq!(
            router.activate(Some(&WindowId::new("panel")), "command"),
            MenuBarActivation::Dispatched {
                owner: Some(WindowId::new("main")),
            }
        );
        assert_eq!(
            *log.borrow(),
            vec!["command".to_owned(), "command".to_owned()]
        );
    }

    #[test]
    fn the_application_bar_serves_hosts_without_window_declarations() {
        let fired = Rc::new(RefCell::new(0_u32));
        let counter = fired.clone();
        let mut router = MenuBarRouter::new(MenuBar::new([MenuBarMenu::new(
            "help",
            "Help",
            [MenuBarEntry::item(MenuItem::new(
                "app-help",
                "Help",
                move || {
                    *counter.borrow_mut() += 1;
                },
            ))],
        )]));
        router.register_window(WindowId::new("main"), MenuBarBindings::default());

        let (owner, _) = router.effective_model(None).expect("application bar");
        assert_eq!(owner, None);
        assert_eq!(
            router.activate(Some(&WindowId::new("main")), "app-help"),
            MenuBarActivation::Dispatched { owner: None }
        );
        assert_eq!(*fired.borrow(), 1);
        assert_eq!(router.activate(None, "unknown"), MenuBarActivation::Unknown);
    }

    #[test]
    fn a_disabled_item_is_refused_and_never_dispatches() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let bindings = MenuBarBindings::default();
        bindings.replace(Some(MenuBar::new([MenuBarMenu::new(
            "file",
            "File",
            [MenuBarEntry::item(
                recording_item("locked", "Locked", &log).enabled(false),
            )],
        )])));
        let mut router = MenuBarRouter::new(MenuBar::default());
        router.register_window(WindowId::new("main"), bindings);

        assert_eq!(
            router.activate(Some(&WindowId::new("main")), "locked"),
            MenuBarActivation::Refused
        );
        assert!(log.borrow().is_empty());
        assert!(!router.item_enabled(Some(&WindowId::new("main")), "locked"));
    }
}
