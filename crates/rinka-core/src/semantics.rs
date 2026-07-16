//! Platform-neutral semantic values and comparable native properties.

use std::sync::Arc;

/// Element category understood by native adapters.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ElementKind {
    /// Static text.
    Label,
    /// Push button.
    Button,
    /// Editable text or search field.
    Input,
    /// Multi-line editable text view.
    TextArea,
    /// Binary control.
    Toggle,
    /// Progress indicator.
    Progress,
    /// Bitmap picture.
    Image,
    /// Visual separator.
    Separator,
    /// Flexible space.
    Spacer,
    /// Horizontal or vertical layout container.
    Stack,
    /// Scrolling container.
    Scroll,
    /// Mounted standard desktop UI pattern.
    Pattern,
    /// Native list container.
    List,
    /// Native list row.
    ListRow,
    /// Empty, busy, or error state.
    Status,
    /// Owned-drawing surface reserved for inherently graphical content.
    Canvas,
    /// Tabbed-document area over user-rearrangeable splits.
    Dock,
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

/// Platform-neutral collection pattern routed to native list controls.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CollectionPattern {
    /// Leading navigation sidebar, optionally with sections and hierarchy.
    NavigationSidebar,
    /// Hierarchical content whose rows expand and collapse.
    Outline,
    /// Primary flat content list.
    ContentList,
    /// Column-oriented data table.
    DataTable,
    /// Undecorated list embedded inside another surface.
    EmbeddedList,
}

impl CollectionPattern {
    /// Returns whether rows may contain declarative child rows.
    pub const fn supports_hierarchy(self) -> bool {
        matches!(self, Self::NavigationSidebar | Self::Outline)
    }

    /// Returns whether the collection declares native columns.
    pub const fn presents_columns(self) -> bool {
        matches!(self, Self::DataTable)
    }
}

/// Semantic role of one collection row.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ListRowRole {
    /// Selectable navigation or content item.
    Item,
    /// Native collection section heading.
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

/// Semantic mapping from a bitmap picture to its native view bounds.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ImageScaling {
    /// Scale preserving the aspect ratio so the whole picture fits inside
    /// the view, leaving empty space on the unfilled axis.
    Fit,
    /// Scale each axis independently so the picture covers the view exactly,
    /// distorting the aspect ratio when the shapes differ.
    Fill,
    /// Draw at the buffer's logical size anchored to the top leading corner,
    /// cropping whatever exceeds the view.
    Actual,
    /// Draw at the buffer's logical size centered in the view, cropping
    /// evenly on every overflowing edge.
    Center,
}

/// Decoded RGBA bitmap content presented by an image element.
///
/// The buffer is row-major with a top-left origin, eight bits per channel in
/// R, G, B, A order, straight (non-premultiplied) alpha, interpreted in the
/// sRGB color space. `stride` counts the bytes between row starts and must
/// cover at least `width * 4` bytes. Decoding stays on the consumer side;
/// the core carries only pixels.
///
/// Reconciliation identifies pixel content by `revision`, never by comparing
/// bytes: within one mounted element, a producer must supply a new revision
/// whenever it supplies a different picture, and two buffers carrying the
/// same revision and geometry are treated as the same picture so the
/// retained native image is not rebuilt or re-uploaded.
#[derive(Clone)]
pub struct ImageContent {
    width: u32,
    height: u32,
    stride: u32,
    scale: f64,
    revision: u64,
    bytes: Arc<[u8]>,
}

impl ImageContent {
    /// Wraps a decoded straight-alpha sRGB RGBA8 buffer at 1.0 pixel density.
    pub fn from_rgba8(
        width: u32,
        height: u32,
        stride: u32,
        bytes: impl Into<Arc<[u8]>>,
        revision: u64,
    ) -> Self {
        Self {
            width,
            height,
            stride,
            scale: 1.0,
            revision,
            bytes: bytes.into(),
        }
    }

    /// Declares the buffer's pixel density in pixels per logical point.
    ///
    /// A buffer of 2.0 density renders at half its pixel extent in layout
    /// points and stays crisp on a 2x display.
    pub fn with_scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    /// Returns the buffer width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the buffer height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the byte distance between row starts.
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    /// Returns the pixel density in pixels per logical point.
    pub const fn scale(&self) -> f64 {
        self.scale
    }

    /// Returns the consumer-declared pixel-content identity.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the RGBA8 bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the layout width in logical points.
    pub fn logical_width(&self) -> f64 {
        f64::from(self.width) / self.scale
    }

    /// Returns the layout height in logical points.
    pub fn logical_height(&self) -> f64 {
        f64::from(self.height) / self.scale
    }

    pub(crate) fn validity_error(&self) -> Option<String> {
        if self.width == 0 || self.height == 0 {
            return Some(format!(
                "image dimensions must be nonzero, received {}x{}",
                self.width, self.height
            ));
        }
        let row_bytes = u64::from(self.width) * 4;
        if u64::from(self.stride) < row_bytes {
            return Some(format!(
                "stride {} does not cover the {} bytes of one {}-pixel row",
                self.stride, row_bytes, self.width
            ));
        }
        let required = u64::from(self.stride) * u64::from(self.height - 1) + row_bytes;
        if (self.bytes.len() as u64) < required {
            return Some(format!(
                "buffer holds {} bytes but the declared geometry requires {required}",
                self.bytes.len()
            ));
        }
        if !self.scale.is_finite() || self.scale <= 0.0 {
            return Some(format!(
                "scale must be a positive finite pixel density, received {}",
                self.scale
            ));
        }
        None
    }
}

impl PartialEq for ImageContent {
    /// Compares picture identity: geometry, density, and revision.
    ///
    /// Bytes never participate, per the revision contract documented on
    /// [`ImageContent`]; equal identity means reconciliation keeps the
    /// retained native image.
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.stride == other.stride
            && self.scale == other.scale
            && self.revision == other.revision
    }
}

impl std::fmt::Debug for ImageContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ImageContent")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("stride", &self.stride)
            .field("scale", &self.scale)
            .field("revision", &self.revision)
            .field("byte_len", &self.bytes.len())
            .finish()
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
    /// Multi-line text area properties.
    TextArea {
        /// Revisioned document content reconciled by the controlled-text
        /// protocol documented on [`crate::TextContent`].
        content: crate::TextContent,
        /// Revisioned semantic highlight spans.
        spans: crate::HighlightSpans,
        /// Controlled selection; [`None`] leaves selection to the native view.
        selection: Option<crate::TextSelection>,
        /// Whether user edits are rejected while selection and copying stay
        /// available.
        read_only: bool,
        /// Typography intent of the editable text.
        role: TextRole,
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
    /// Bitmap image properties.
    Image {
        /// Decoded RGBA pixel content.
        content: ImageContent,
        /// Mapping from the picture to the native view bounds.
        scaling: ImageScaling,
        /// Screen-reader description of the picture.
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
    /// Standard desktop UI pattern properties.
    Pattern {
        /// Platform-neutral pattern routed by the native adapter.
        pattern: crate::UiPattern,
    },
    /// List container properties.
    List {
        /// Screen-reader description.
        accessibility_label: String,
        /// Native collection treatment.
        pattern: CollectionPattern,
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
        /// Controlled hierarchical expansion state.
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
    /// Owned-drawing canvas properties.
    ///
    /// Reserved for inherently graphical content such as terminal cell
    /// grids, audio meters, and dashboard widget faces. A canvas that
    /// imitates a native control violates the design contract.
    Canvas {
        /// Intrinsic content extent in logical points.
        size: crate::CanvasSize,
        /// Recorded display list rebuilt by the application each render.
        scene: crate::DrawScene,
        /// Whether the canvas participates in keyboard focus and receives
        /// raw key and IME composition events.
        accepts_input: bool,
        /// Element-local rectangle anchoring the operating-system IME
        /// candidate window, reconciled like every other property. `None`
        /// while the application declares no caret.
        ime_caret: Option<crate::CanvasRect>,
        /// Screen-reader description of the graphical content.
        accessibility_label: String,
    },
    /// Tabbed-document dock properties.
    ///
    /// The layout is the complete declarative split-and-group description;
    /// the element's children are the tab content subtrees, one per tab,
    /// keyed by tab id. Native gestures surface as [`crate::DockEvent`]
    /// requests through the element's stable event binding.
    Dock {
        /// Declarative split tree whose leaves are tab groups.
        layout: crate::DockLayout,
        /// Screen-reader label of the dock area.
        accessibility_label: String,
    },
}

impl Props {
    /// Returns the native accessibility name derived from these properties.
    pub fn accessibility_name(&self) -> Option<&str> {
        match self {
            Self::Label { text, .. } => Some(text),
            Self::Button {
                accessibility_label,
                ..
            }
            | Self::Input {
                accessibility_label,
                ..
            }
            | Self::TextArea {
                accessibility_label,
                ..
            }
            | Self::Toggle {
                accessibility_label,
                ..
            }
            | Self::Progress {
                accessibility_label,
                ..
            }
            | Self::Image {
                accessibility_label,
                ..
            }
            | Self::List {
                accessibility_label,
                ..
            }
            | Self::ListRow {
                accessibility_label,
                ..
            }
            | Self::Canvas {
                accessibility_label,
                ..
            }
            | Self::Dock {
                accessibility_label,
                ..
            } => Some(accessibility_label),
            Self::Status { title, .. } => Some(title),
            _ => None,
        }
    }

    /// Returns the category represented by these properties.
    pub fn kind(&self) -> ElementKind {
        match self {
            Self::Label { .. } => ElementKind::Label,
            Self::Button { .. } => ElementKind::Button,
            Self::Input { .. } => ElementKind::Input,
            Self::TextArea { .. } => ElementKind::TextArea,
            Self::Toggle { .. } => ElementKind::Toggle,
            Self::Progress { .. } => ElementKind::Progress,
            Self::Image { .. } => ElementKind::Image,
            Self::Separator { .. } => ElementKind::Separator,
            Self::Spacer { .. } => ElementKind::Spacer,
            Self::Stack { .. } => ElementKind::Stack,
            Self::Scroll { .. } => ElementKind::Scroll,
            Self::Pattern { .. } => ElementKind::Pattern,
            Self::List { .. } => ElementKind::List,
            Self::ListRow { .. } => ElementKind::ListRow,
            Self::Status { .. } => ElementKind::Status,
            Self::Canvas { .. } => ElementKind::Canvas,
            Self::Dock { .. } => ElementKind::Dock,
        }
    }
}
