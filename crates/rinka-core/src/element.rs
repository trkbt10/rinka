//! Immutable declarative element descriptions and semantic roles.

use crate::event::{EventHandlers, InputHandler};
use crate::{ActivateHandler, ToggleHandler};
use std::fmt;
use std::rc::Rc;

/// Stable identity within one sibling set.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Key(String);

impl Key {
    /// Creates a key.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the key text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Element category understood by native adapters.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ElementKind {
    /// Static text.
    Label,
    /// Push button.
    Button,
    /// Editable text or search field.
    Input,
    /// Binary control.
    Toggle,
    /// Progress indicator.
    Progress,
    /// Visual separator.
    Separator,
    /// Flexible space.
    Spacer,
    /// Horizontal or vertical layout container.
    Stack,
    /// Scrolling container.
    Scroll,
    /// Resizable two-pane container.
    Split,
    /// Native sidebar, content, and inspector workspace.
    Workspace,
    /// Native list container.
    List,
    /// Native list row.
    ListRow,
    /// Empty, busy, or error state.
    Status,
}

/// Primary layout direction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Axis {
    /// Left-to-right in a left-to-right locale.
    Horizontal,
    /// Top-to-bottom.
    Vertical,
}

/// Cross-axis alignment intent.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Align {
    /// Leading or top edge.
    Start,
    /// Geometric center.
    Center,
    /// Trailing or bottom edge.
    End,
    /// Fill the available cross-axis space.
    Stretch,
}

/// Placement of a stack's content along its primary axis.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Justify {
    /// Place content at the leading or top edge.
    Start,
    /// Center content in the available extent.
    Center,
    /// Place content at the trailing or bottom edge.
    End,
}

/// Platform-resolved spacing density.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Spacing {
    /// Joined surfaces that share one structural boundary.
    Joined,
    /// Adjacent parts of one control.
    Compact,
    /// Default control-to-control distance.
    Related,
    /// Separation between semantic groups.
    Section,
    /// Window-content inset.
    Content,
}

/// Semantic text hierarchy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextRole {
    /// Primary window or page title.
    Title,
    /// Section heading.
    Heading,
    /// Normal content.
    Body,
    /// Supporting information.
    Secondary,
    /// Monospaced content such as a path.
    Monospace,
}

/// Semantic button treatment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ButtonRole {
    /// Normal action.
    Standard,
    /// Main affirmative action in the current context.
    Primary,
    /// Action with destructive consequences.
    Destructive,
    /// Low-emphasis toolbar action.
    Toolbar,
}

/// Platform-native control metric for a button.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ControlSize {
    /// Densest metric for compact auxiliary controls.
    Mini,
    /// Small metric for space-constrained supporting controls.
    Small,
    /// Standard desktop control metric.
    Regular,
    /// Spacious metric with stronger action emphasis.
    Large,
    /// Most prominent action metric where the platform supports it.
    ExtraLarge,
}

/// Material layer used by a button.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ButtonMaterial {
    /// Let the native toolkit choose the content-layer backing.
    Automatic,
    /// Place a top-level floating action on native glass.
    Glass,
}

/// Native input variant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InputKind {
    /// General text editing.
    Text,
    /// Platform search control.
    Search,
    /// Concealed secret input.
    Secure,
}

/// Meaning of a two-pane arrangement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SplitRole {
    /// Navigation sidebar and content.
    Navigation,
    /// Content and a secondary utility pane.
    Utility,
}

/// Native list presentation intent.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ListStyle {
    /// Navigation source list with platform selection treatment.
    Source,
    /// Primary content list or table.
    Content,
    /// Column-oriented data table.
    Table,
    /// Undecorated list embedded in another surface.
    Plain,
}

/// Semantic role of one source-list row.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ListRowRole {
    /// Selectable source or content item.
    Item,
    /// Native source-list section heading.
    Section,
}

/// Direction of a native table sort descriptor.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SortDirection {
    /// Smallest or earliest values first.
    Ascending,
    /// Largest or latest values first.
    Descending,
}

/// Sort change reported by a native table header.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TableSort {
    /// Stable column identifier.
    pub column_id: String,
    /// Selected direction.
    pub direction: SortDirection,
}

/// Declarative column in a native data table.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TableColumn {
    /// Stable identifier used to preserve the native column.
    pub id: String,
    /// Visible native header title.
    pub title: String,
    /// Whether the native header accepts sorting.
    pub sortable: bool,
    /// Controlled active sort direction for this column.
    pub sort_direction: Option<SortDirection>,
}

impl TableColumn {
    /// Creates a native table column description.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            sortable: false,
            sort_direction: None,
        }
    }

    /// Enables native sorting for this column.
    pub fn sortable(mut self, sortable: bool) -> Self {
        self.sortable = sortable;
        if !sortable {
            self.sort_direction = None;
        }
        self
    }

    /// Marks this sortable column as the active table sort.
    pub fn sorted(mut self, direction: SortDirection) -> Self {
        self.sortable = true;
        self.sort_direction = Some(direction);
        self
    }
}

/// Semantic status presentation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StatusTone {
    /// No data is available yet.
    Empty,
    /// Work is in progress.
    Busy,
    /// An operation failed.
    Error,
    /// Informational state.
    Informational,
}

/// Cross-platform meaning for a native symbolic icon.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Symbol {
    /// Navigate backward.
    Back,
    /// Navigate forward.
    Forward,
    /// Add or create.
    Add,
    /// Refresh current content.
    Refresh,
    /// Search content.
    Search,
    /// Home location.
    Home,
    /// Folder or directory.
    Folder,
    /// Regular file.
    File,
    /// Source-code file.
    Code,
    /// Image file.
    Image,
    /// Terminal or command line.
    Terminal,
    /// Application settings.
    Settings,
    /// More actions.
    More,
    /// Grid or icon view.
    Grid,
    /// List view.
    List,
    /// Column view.
    Columns,
    /// Gallery view.
    Gallery,
    /// Sort or group options.
    Sort,
    /// Share content.
    Share,
    /// Apply a tag.
    Tag,
    /// Navigate forward into an item.
    Disclosure,
    /// Warning or error.
    Warning,
}

/// Comparable, platform-neutral properties.
#[derive(Clone, Debug, PartialEq)]
pub enum Props {
    /// Label properties.
    Label {
        /// Visible text.
        text: String,
        /// Typography intent.
        role: TextRole,
        /// Whether users can select and copy the text.
        selectable: bool,
    },
    /// Button properties.
    Button {
        /// Visible title.
        label: String,
        /// Visual and behavioral role.
        role: ButtonRole,
        /// Native control metric.
        size: ControlSize,
        /// Native backing material.
        material: ButtonMaterial,
        /// Whether the action is available.
        enabled: bool,
        /// Hover help.
        tooltip: Option<String>,
        /// Screen-reader label.
        accessibility_label: String,
    },
    /// Input properties.
    Input {
        /// Controlled value.
        value: String,
        /// Empty-field prompt.
        placeholder: String,
        /// Native input variant.
        kind: InputKind,
        /// Whether editing is available.
        enabled: bool,
        /// Screen-reader label.
        accessibility_label: String,
    },
    /// Toggle properties.
    Toggle {
        /// Visible label.
        label: String,
        /// Controlled value.
        value: bool,
        /// Native control metric.
        size: ControlSize,
        /// Whether interaction is available.
        enabled: bool,
        /// Screen-reader label.
        accessibility_label: String,
    },
    /// Progress properties.
    Progress {
        /// Value in the inclusive 0.0 through 1.0 range.
        fraction: f64,
        /// Textual progress description.
        accessibility_label: String,
    },
    /// Separator properties.
    Separator {
        /// Direction of the dividing line.
        axis: Axis,
    },
    /// Spacer properties.
    Spacer {
        /// Whether horizontal space can grow.
        horizontal: bool,
        /// Whether vertical space can grow.
        vertical: bool,
    },
    /// Stack properties.
    Stack {
        /// Primary layout direction.
        axis: Axis,
        /// Native spacing density.
        spacing: Spacing,
        /// Native content inset density.
        padding: Option<Spacing>,
        /// Cross-axis alignment.
        align: Align,
        /// Primary-axis placement.
        justify: Justify,
    },
    /// Scrolling container properties.
    Scroll {
        /// Scrolling direction.
        axis: Axis,
    },
    /// Split-view properties.
    Split {
        /// Pane arrangement.
        role: SplitRole,
        /// Whether the secondary pane can be hidden or collapsed.
        collapsible: bool,
    },
    /// Three-region navigation workspace properties.
    Workspace {
        /// Whether the navigation sidebar can be hidden or collapsed.
        sidebar_collapsible: bool,
        /// Whether the utility inspector can be hidden or collapsed.
        inspector_collapsible: bool,
    },
    /// List container properties.
    List {
        /// Screen-reader description.
        accessibility_label: String,
        /// Native list treatment.
        style: ListStyle,
        /// Native columns when the list uses table presentation.
        columns: Vec<TableColumn>,
    },
    /// List row properties.
    ListRow {
        /// Primary line.
        title: String,
        /// Optional supporting line.
        subtitle: Option<String>,
        /// Values for table columns after the primary title column.
        cells: Vec<String>,
        /// Source-list item or section semantics.
        role: ListRowRole,
        /// Controlled source-list expansion state.
        expanded: bool,
        /// Platform symbol name.
        symbol: Option<Symbol>,
        /// Selection state.
        selected: bool,
        /// Whether activation navigates deeper.
        disclosure: bool,
        /// Screen-reader label.
        accessibility_label: String,
    },
    /// Status-page properties.
    Status {
        /// Primary status text.
        title: String,
        /// Supporting explanation.
        message: String,
        /// Status intent.
        tone: StatusTone,
    },
}

impl Props {
    /// Returns the category represented by these properties.
    pub fn kind(&self) -> ElementKind {
        match self {
            Self::Label { .. } => ElementKind::Label,
            Self::Button { .. } => ElementKind::Button,
            Self::Input { .. } => ElementKind::Input,
            Self::Toggle { .. } => ElementKind::Toggle,
            Self::Progress { .. } => ElementKind::Progress,
            Self::Separator { .. } => ElementKind::Separator,
            Self::Spacer { .. } => ElementKind::Spacer,
            Self::Stack { .. } => ElementKind::Stack,
            Self::Scroll { .. } => ElementKind::Scroll,
            Self::Split { .. } => ElementKind::Split,
            Self::Workspace { .. } => ElementKind::Workspace,
            Self::List { .. } => ElementKind::List,
            Self::ListRow { .. } => ElementKind::ListRow,
            Self::Status { .. } => ElementKind::Status,
        }
    }
}

/// Immutable description of a native UI subtree.
#[derive(Clone)]
pub struct Element {
    pub(crate) key: Option<Key>,
    pub(crate) props: Props,
    pub(crate) children: Vec<Self>,
    pub(crate) handlers: EventHandlers,
}

impl Element {
    fn leaf(props: Props) -> Self {
        Self {
            key: None,
            props,
            children: Vec::new(),
            handlers: EventHandlers::default(),
        }
    }

    fn parent(props: Props, children: impl IntoIterator<Item = Self>) -> Self {
        Self {
            key: None,
            props,
            children: children.into_iter().collect(),
            handlers: EventHandlers::default(),
        }
    }

    /// Returns the element category.
    pub fn kind(&self) -> ElementKind {
        self.props.kind()
    }

    /// Returns the stable sibling key.
    pub fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// Returns comparable properties.
    pub fn props(&self) -> &Props {
        &self.props
    }

    /// Returns declarative children.
    pub fn children(&self) -> &[Self] {
        &self.children
    }

    /// Assigns stable identity within the parent.
    pub fn with_key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Sets stack spacing.
    pub fn spacing(mut self, spacing: Spacing) -> Self {
        match &mut self.props {
            Props::Stack { spacing: value, .. } => *value = spacing,
            _ => panic!("spacing is valid only for a stack"),
        }
        self
    }

    /// Sets stack content inset.
    pub fn padding(mut self, padding: Spacing) -> Self {
        match &mut self.props {
            Props::Stack { padding: value, .. } => *value = Some(padding),
            _ => panic!("padding is valid only for a stack"),
        }
        self
    }

    /// Sets stack cross-axis alignment.
    pub fn align(mut self, align: Align) -> Self {
        match &mut self.props {
            Props::Stack { align: value, .. } => *value = align,
            _ => panic!("align is valid only for a stack"),
        }
        self
    }

    /// Sets stack placement along its primary axis.
    pub fn justify(mut self, justify: Justify) -> Self {
        match &mut self.props {
            Props::Stack { justify: value, .. } => *value = justify,
            _ => panic!("justify is valid only for a stack"),
        }
        self
    }

    /// Sets a button's semantic role.
    pub fn button_role(mut self, role: ButtonRole) -> Self {
        match &mut self.props {
            Props::Button { role: value, .. } => *value = role,
            _ => panic!("button_role is valid only for a button"),
        }
        self
    }

    /// Sets a button or toggle's native control metric.
    pub fn control_size(mut self, size: ControlSize) -> Self {
        match &mut self.props {
            Props::Button { size: value, .. } | Props::Toggle { size: value, .. } => *value = size,
            _ => panic!("control_size is valid only for a button or toggle"),
        }
        self
    }

    /// Sets a button's material layer.
    pub fn button_material(mut self, material: ButtonMaterial) -> Self {
        match &mut self.props {
            Props::Button {
                material: value, ..
            } => *value = material,
            _ => panic!("button_material is valid only for a button"),
        }
        self
    }

    /// Sets a list's native presentation intent.
    pub fn list_style(mut self, style: ListStyle) -> Self {
        match &mut self.props {
            Props::List { style: value, .. } => *value = style,
            _ => panic!("list_style is valid only for a list"),
        }
        self
    }

    /// Sets a table list's native columns in display order.
    pub fn table_columns(mut self, columns: impl IntoIterator<Item = TableColumn>) -> Self {
        match &mut self.props {
            Props::List { columns: value, .. } => *value = columns.into_iter().collect(),
            _ => panic!("table_columns is valid only for a list"),
        }
        self
    }

    /// Sets values for table columns after a row's primary title column.
    pub fn table_cells(mut self, cells: impl IntoIterator<Item = impl Into<String>>) -> Self {
        match &mut self.props {
            Props::ListRow { cells: value, .. } => {
                *value = cells.into_iter().map(Into::into).collect();
            }
            _ => panic!("table_cells is valid only for a list row"),
        }
        self
    }

    /// Marks a row as a native source-list section heading.
    pub fn source_section(mut self) -> Self {
        match &mut self.props {
            Props::ListRow { role, .. } => *role = ListRowRole::Section,
            _ => panic!("source_section is valid only for a list row"),
        }
        self
    }

    /// Adds nested rows to a native outline list or table row.
    pub fn list_children(mut self, rows: impl IntoIterator<Item = Element>) -> Self {
        match &self.props {
            Props::ListRow { .. } => self.children = rows.into_iter().collect(),
            _ => panic!("list_children is valid only for a list row"),
        }
        self
    }

    /// Adds nested rows to a source-list item or section.
    pub fn source_children(self, rows: impl IntoIterator<Item = Element>) -> Self {
        self.list_children(rows)
    }

    /// Sets controlled native outline expansion state.
    pub fn expanded(mut self, expanded: bool) -> Self {
        match &mut self.props {
            Props::ListRow {
                expanded: value, ..
            } => *value = expanded,
            _ => panic!("expanded is valid only for a list row"),
        }
        self
    }

    /// Handles native outline expansion changes.
    pub fn on_expansion_change(mut self, handler: impl Fn(bool) + 'static) -> Self {
        match &self.props {
            Props::ListRow { .. } => self.handlers.toggle = Some(Rc::new(handler)),
            _ => panic!("on_expansion_change is valid only for a list row"),
        }
        self
    }

    /// Handles native table sort changes.
    pub fn on_sort_change(mut self, handler: impl Fn(TableSort) + 'static) -> Self {
        match &self.props {
            Props::List { .. } => self.handlers.sort = Some(Rc::new(handler)),
            _ => panic!("on_sort_change is valid only for a list"),
        }
        self
    }

    /// Sets a label's semantic role.
    pub fn text_role(mut self, role: TextRole) -> Self {
        match &mut self.props {
            Props::Label { role: value, .. } => *value = role,
            _ => panic!("text_role is valid only for a label"),
        }
        self
    }

    /// Changes the enabled state of an interactive element.
    pub fn enabled(mut self, enabled: bool) -> Self {
        match &mut self.props {
            Props::Button { enabled: value, .. }
            | Props::Input { enabled: value, .. }
            | Props::Toggle { enabled: value, .. } => *value = enabled,
            _ => panic!("enabled is valid only for an interactive element"),
        }
        self
    }

    /// Adds native hover help to a button.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        match &mut self.props {
            Props::Button { tooltip: value, .. } => *value = Some(tooltip.into()),
            _ => panic!("tooltip is valid only for a button"),
        }
        self
    }

    /// Marks label text as selectable.
    pub fn selectable(mut self, selectable: bool) -> Self {
        match &mut self.props {
            Props::Label {
                selectable: value, ..
            } => *value = selectable,
            _ => panic!("selectable is valid only for a label"),
        }
        self
    }

    pub(crate) fn take_children(&mut self) -> Vec<Self> {
        std::mem::take(&mut self.children)
    }
}

impl fmt::Debug for Element {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Element")
            .field("key", &self.key)
            .field("props", &self.props)
            .field("children", &self.children)
            .field("handlers", &self.handlers)
            .finish()
    }
}

/// Creates static text.
pub fn label(text: impl Into<String>) -> Element {
    Element::leaf(Props::Label {
        text: text.into(),
        role: TextRole::Body,
        selectable: false,
    })
}

/// Creates a push button.
pub fn button(
    label: impl Into<String>,
    accessibility_label: impl Into<String>,
    handler: impl Fn() + 'static,
) -> Element {
    let mut element = Element::leaf(Props::Button {
        label: label.into(),
        role: ButtonRole::Standard,
        size: ControlSize::Regular,
        material: ButtonMaterial::Automatic,
        enabled: true,
        tooltip: None,
        accessibility_label: accessibility_label.into(),
    });
    element.handlers.activate = Some(Rc::new(handler) as ActivateHandler);
    element
}

/// Creates a controlled native text field.
pub fn input(
    value: impl Into<String>,
    placeholder: impl Into<String>,
    kind: InputKind,
    accessibility_label: impl Into<String>,
    handler: impl Fn(String) + 'static,
) -> Element {
    let mut element = Element::leaf(Props::Input {
        value: value.into(),
        placeholder: placeholder.into(),
        kind,
        enabled: true,
        accessibility_label: accessibility_label.into(),
    });
    element.handlers.input = Some(Rc::new(handler) as InputHandler);
    element
}

/// Creates a controlled binary native control.
pub fn toggle(
    label: impl Into<String>,
    value: bool,
    accessibility_label: impl Into<String>,
    handler: impl Fn(bool) + 'static,
) -> Element {
    let mut element = Element::leaf(Props::Toggle {
        label: label.into(),
        value,
        size: ControlSize::Regular,
        enabled: true,
        accessibility_label: accessibility_label.into(),
    });
    element.handlers.toggle = Some(Rc::new(handler) as ToggleHandler);
    element
}

/// Creates a native progress indicator.
pub fn progress(fraction: f64, accessibility_label: impl Into<String>) -> Element {
    Element::leaf(Props::Progress {
        fraction: fraction.clamp(0.0, 1.0),
        accessibility_label: accessibility_label.into(),
    })
}

/// Creates a native separator.
pub fn separator(axis: Axis) -> Element {
    Element::leaf(Props::Separator { axis })
}

/// Creates flexible layout space.
pub fn spacer(horizontal: bool, vertical: bool) -> Element {
    Element::leaf(Props::Spacer {
        horizontal,
        vertical,
    })
}

fn stack(axis: Axis, children: impl IntoIterator<Item = Element>) -> Element {
    Element::parent(
        Props::Stack {
            axis,
            spacing: Spacing::Related,
            padding: None,
            align: Align::Stretch,
            justify: Justify::Start,
        },
        children,
    )
}

/// Creates a horizontal native stack.
pub fn row(children: impl IntoIterator<Item = Element>) -> Element {
    stack(Axis::Horizontal, children)
}

/// Creates a vertical native stack.
pub fn column(children: impl IntoIterator<Item = Element>) -> Element {
    stack(Axis::Vertical, children)
}

/// Creates a native scrolling container with exactly one child.
pub fn scroll(axis: Axis, child: Element) -> Element {
    Element::parent(Props::Scroll { axis }, [child])
}

/// Creates a native two-pane container with exactly two children.
pub fn split(role: SplitRole, collapsible: bool, first: Element, second: Element) -> Element {
    Element::parent(Props::Split { role, collapsible }, [first, second])
}

/// Creates one native navigation workspace with sidebar, content, and inspector.
pub fn workspace(
    sidebar_collapsible: bool,
    inspector_collapsible: bool,
    sidebar: Element,
    content: Element,
    inspector: Element,
) -> Element {
    Element::parent(
        Props::Workspace {
            sidebar_collapsible,
            inspector_collapsible,
        },
        [sidebar, content, inspector],
    )
}

/// Creates a native list.
pub fn list(
    accessibility_label: impl Into<String>,
    rows: impl IntoIterator<Item = Element>,
) -> Element {
    Element::parent(
        Props::List {
            accessibility_label: accessibility_label.into(),
            style: ListStyle::Content,
            columns: Vec::new(),
        },
        rows,
    )
}

/// Creates an activatable native list row.
#[allow(clippy::too_many_arguments)]
pub fn list_row(
    title: impl Into<String>,
    subtitle: Option<String>,
    symbol: Option<Symbol>,
    selected: bool,
    disclosure: bool,
    accessibility_label: impl Into<String>,
    handler: impl Fn() + 'static,
) -> Element {
    let mut element = Element::leaf(Props::ListRow {
        title: title.into(),
        subtitle,
        cells: Vec::new(),
        role: ListRowRole::Item,
        expanded: false,
        symbol,
        selected,
        disclosure,
        accessibility_label: accessibility_label.into(),
    });
    element.handlers.activate = Some(Rc::new(handler) as ActivateHandler);
    element
}

/// Creates a native empty, busy, error, or informational status page.
pub fn status(title: impl Into<String>, message: impl Into<String>, tone: StatusTone) -> Element {
    Element::leaf(Props::Status {
        title: title.into(),
        message: message.into(),
        tone,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_accepts_a_native_control_size() {
        let element = toggle("Setting", false, "Setting", |_| {}).control_size(ControlSize::Small);

        assert!(matches!(
            element.props(),
            Props::Toggle {
                size: ControlSize::Small,
                ..
            }
        ));
    }
}
