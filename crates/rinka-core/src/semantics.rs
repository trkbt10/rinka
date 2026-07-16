//! Platform-neutral semantic values and comparable native properties.

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
