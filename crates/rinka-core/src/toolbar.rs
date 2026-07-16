//! Declarative native toolbar contracts.

use crate::chord::KeyChord;
use crate::{ActivateHandler, InputHandler, Symbol};
use std::fmt;
use std::rc::Rc;

/// Toolbar item placement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ToolbarPlacement {
    /// Leading navigation region.
    Leading,
    /// Centered or principal region.
    Center,
    /// Trailing action region.
    Trailing,
}

/// Window-level preference for native toolbar item labels.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ToolbarDisplay {
    /// Let the platform and the user's toolbar preferences choose.
    #[default]
    Automatic,
    /// Show symbols and labels.
    IconAndLabel,
    /// Show symbols while retaining labels for accessibility and menus.
    IconOnly,
    /// Show labels without symbols where the platform supports it.
    LabelOnly,
}

/// Presentation preference for a native toolbar group.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ToolbarGroupDisplay {
    /// Let the platform choose from the available toolbar width.
    #[default]
    Automatic,
    /// Keep the group's individual controls visible when supported.
    Expanded,
    /// Present the group through its compact native representation.
    Collapsed,
}

/// One action inside a native toolbar group or menu.
#[derive(Clone)]
pub struct ToolbarAction {
    /// Stable identity within the containing item.
    pub id: String,
    /// Visible and accessible label.
    pub label: String,
    /// Platform symbol name.
    pub symbol: Symbol,
    /// Hover help and accessible description.
    pub help: String,
    /// Whether the action is currently enabled.
    pub enabled: bool,
    /// Key chord shown as the native key equivalent when the action is
    /// hosted inside a menu. Display data only: app-wide delivery of the
    /// chord is declared through the window's accelerator table.
    pub chord: Option<KeyChord>,
    /// Activation handler connected once by the native host.
    pub on_activate: ActivateHandler,
}

impl ToolbarAction {
    /// Creates an enabled toolbar action.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        symbol: Symbol,
        help: impl Into<String>,
        handler: impl Fn() + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            symbol,
            help: help.into(),
            enabled: true,
            chord: None,
            on_activate: Rc::new(handler),
        }
    }

    /// Changes availability while preserving the action.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Declares the key chord a hosting menu displays for this action.
    pub fn chord(mut self, chord: KeyChord) -> Self {
        self.chord = Some(chord);
        self
    }
}

impl fmt::Debug for ToolbarAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolbarAction")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("symbol", &self.symbol)
            .field("help", &self.help)
            .field("enabled", &self.enabled)
            .field("chord", &self.chord.map(|chord| chord.to_string()))
            .finish_non_exhaustive()
    }
}

/// One mutually-exclusive choice in a native toolbar selection group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolbarChoice {
    /// Stable identity reported to the selection handler.
    pub id: String,
    /// Visible and accessible label.
    pub label: String,
    /// Platform symbol name.
    pub symbol: Symbol,
    /// Whether the choice is currently enabled.
    pub enabled: bool,
}

impl ToolbarChoice {
    /// Creates an enabled toolbar choice.
    pub fn new(id: impl Into<String>, label: impl Into<String>, symbol: Symbol) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            symbol,
            enabled: true,
        }
    }

    /// Changes availability while preserving identity.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Entry in a toolbar-owned native menu.
#[derive(Clone)]
pub enum ToolbarMenuEntry {
    /// Activatable command.
    Action(ToolbarAction),
    /// Native menu separator.
    Separator,
}

impl ToolbarMenuEntry {
    /// Creates an activatable menu command.
    pub fn action(action: ToolbarAction) -> Self {
        Self::Action(action)
    }

    /// Creates a native separator.
    pub const fn separator() -> Self {
        Self::Separator
    }
}

impl fmt::Debug for ToolbarMenuEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Action(action) => formatter.debug_tuple("Action").field(action).finish(),
            Self::Separator => formatter.write_str("Separator"),
        }
    }
}

/// Native representation used by a declarative toolbar item.
#[derive(Clone)]
pub enum ToolbarItemKind {
    /// One standard toolbar action.
    Action {
        /// Platform symbol name.
        symbol: Symbol,
        /// Activation handler.
        on_activate: ActivateHandler,
    },
    /// Attached actions that move and overflow as one native group.
    ActionGroup {
        /// Actions in display order.
        actions: Vec<ToolbarAction>,
    },
    /// Single-selection native segmented group.
    SelectionGroup {
        /// Choices in display order.
        choices: Vec<ToolbarChoice>,
        /// Controlled selected identity.
        selected_id: String,
        /// Handler receiving the selected identity.
        on_select: InputHandler,
    },
    /// Action menu presented by a native menu toolbar item.
    Menu {
        /// Platform symbol name.
        symbol: Symbol,
        /// Menu entries in display order.
        entries: Vec<ToolbarMenuEntry>,
    },
    /// Native toolbar search field.
    Search {
        /// Controlled query text.
        value: String,
        /// Empty-field prompt.
        placeholder: String,
        /// Screen-reader label.
        accessibility_label: String,
        /// Handler receiving edited query text.
        on_input: InputHandler,
    },
}

impl fmt::Debug for ToolbarItemKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Action { symbol, .. } => formatter
                .debug_struct("Action")
                .field("symbol", symbol)
                .finish_non_exhaustive(),
            Self::ActionGroup { actions } => formatter
                .debug_struct("ActionGroup")
                .field("actions", actions)
                .finish(),
            Self::SelectionGroup {
                choices,
                selected_id,
                ..
            } => formatter
                .debug_struct("SelectionGroup")
                .field("choices", choices)
                .field("selected_id", selected_id)
                .finish_non_exhaustive(),
            Self::Menu {
                symbol, entries, ..
            } => formatter
                .debug_struct("Menu")
                .field("symbol", symbol)
                .field("entries", entries)
                .finish(),
            Self::Search {
                value,
                placeholder,
                accessibility_label,
                ..
            } => formatter
                .debug_struct("Search")
                .field("value", value)
                .field("placeholder", placeholder)
                .field("accessibility_label", accessibility_label)
                .finish_non_exhaustive(),
        }
    }
}

/// Declarative native toolbar item.
#[derive(Clone, Debug)]
pub struct ToolbarItem {
    /// Stable item identity.
    pub id: String,
    /// Visible or menu label.
    pub label: String,
    /// Hover help and accessible description.
    pub help: String,
    /// Toolbar region.
    pub placement: ToolbarPlacement,
    /// Whether the item is currently enabled.
    pub enabled: bool,
    /// Native representation preference for grouped items.
    pub group_display: ToolbarGroupDisplay,
    /// Native semantic representation.
    pub kind: ToolbarItemKind,
}

impl ToolbarItem {
    /// Creates an enabled native toolbar action.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        symbol: Symbol,
        help: impl Into<String>,
        placement: ToolbarPlacement,
        handler: impl Fn() + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            help: help.into(),
            placement,
            enabled: true,
            group_display: ToolbarGroupDisplay::Automatic,
            kind: ToolbarItemKind::Action {
                symbol,
                on_activate: Rc::new(handler),
            },
        }
    }

    /// Creates attached native actions that move and overflow together.
    pub fn action_group(
        id: impl Into<String>,
        label: impl Into<String>,
        help: impl Into<String>,
        placement: ToolbarPlacement,
        actions: impl IntoIterator<Item = ToolbarAction>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            help: help.into(),
            placement,
            enabled: true,
            group_display: ToolbarGroupDisplay::Automatic,
            kind: ToolbarItemKind::ActionGroup {
                actions: actions.into_iter().collect(),
            },
        }
    }

    /// Creates a native single-selection segmented group.
    #[allow(clippy::too_many_arguments)]
    pub fn selection_group(
        id: impl Into<String>,
        label: impl Into<String>,
        help: impl Into<String>,
        placement: ToolbarPlacement,
        choices: impl IntoIterator<Item = ToolbarChoice>,
        selected_id: impl Into<String>,
        handler: impl Fn(String) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            help: help.into(),
            placement,
            enabled: true,
            group_display: ToolbarGroupDisplay::Automatic,
            kind: ToolbarItemKind::SelectionGroup {
                choices: choices.into_iter().collect(),
                selected_id: selected_id.into(),
                on_select: Rc::new(handler),
            },
        }
    }

    /// Creates a native toolbar-owned action menu.
    pub fn menu(
        id: impl Into<String>,
        label: impl Into<String>,
        symbol: Symbol,
        help: impl Into<String>,
        placement: ToolbarPlacement,
        entries: impl IntoIterator<Item = ToolbarMenuEntry>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            help: help.into(),
            placement,
            enabled: true,
            group_display: ToolbarGroupDisplay::Automatic,
            kind: ToolbarItemKind::Menu {
                symbol,
                entries: entries.into_iter().collect(),
            },
        }
    }

    /// Creates a native toolbar search item.
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        id: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
        placeholder: impl Into<String>,
        accessibility_label: impl Into<String>,
        help: impl Into<String>,
        placement: ToolbarPlacement,
        handler: impl Fn(String) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            help: help.into(),
            placement,
            enabled: true,
            group_display: ToolbarGroupDisplay::Automatic,
            kind: ToolbarItemKind::Search {
                value: value.into(),
                placeholder: placeholder.into(),
                accessibility_label: accessibility_label.into(),
                on_input: Rc::new(handler),
            },
        }
    }

    /// Changes availability while preserving the native semantic kind.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Chooses the native representation used for a grouped toolbar item.
    pub fn group_display(mut self, display: ToolbarGroupDisplay) -> Self {
        self.group_display = display;
        self
    }
}
