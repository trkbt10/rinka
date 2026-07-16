//! Immutable declarative element descriptions and semantic roles.

use crate::accelerator::Accelerator;
use crate::drag::{DragPayload, DropTarget, FileDrop, FilePromise, PayloadDrop};
use crate::event::{EventHandlers, InputHandler, SelectionChangeHandler, TextChangeHandler};
use crate::menu::{ContextMenu, MenuEntry};
use crate::semantics::*;
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

/// Immutable description of a native UI subtree.
#[derive(Clone)]
pub struct Element {
    pub(crate) key: Option<Key>,
    pub(crate) props: Props,
    pub(crate) children: Vec<Self>,
    pub(crate) handlers: EventHandlers,
    pub(crate) accelerators: Vec<Accelerator>,
}

impl Element {
    fn leaf(props: Props) -> Self {
        Self {
            key: None,
            props,
            children: Vec::new(),
            handlers: EventHandlers::default(),
            accelerators: Vec::new(),
        }
    }

    fn parent(props: Props, children: impl IntoIterator<Item = Self>) -> Self {
        Self {
            key: None,
            props,
            children: children.into_iter().collect(),
            handlers: EventHandlers::default(),
            accelerators: Vec::new(),
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

    /// Sets a list's standard collection pattern.
    pub fn collection_pattern(mut self, pattern: CollectionPattern) -> Self {
        match &mut self.props {
            Props::List { pattern: value, .. } => *value = pattern,
            _ => panic!("collection_pattern is valid only for a list"),
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

    /// Marks a row as a section heading in a hierarchical collection.
    pub fn section_header(mut self) -> Self {
        match &mut self.props {
            Props::ListRow { role, .. } => *role = ListRowRole::Section,
            _ => panic!("section_header is valid only for a list row"),
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

    /// Adds nested rows to a hierarchical collection item or section.
    pub fn outline_children(self, rows: impl IntoIterator<Item = Element>) -> Self {
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

    /// Sets a label's or text area's semantic typography role.
    pub fn text_role(mut self, role: TextRole) -> Self {
        match &mut self.props {
            Props::Label { role: value, .. } | Props::TextArea { role: value, .. } => {
                *value = role;
            }
            _ => panic!("text_role is valid only for a label or a text area"),
        }
        self
    }

    /// Marks a text area as rejecting user edits while keeping selection,
    /// copying, and programmatic updates available.
    pub fn read_only(mut self, read_only: bool) -> Self {
        match &mut self.props {
            Props::TextArea {
                read_only: value, ..
            } => *value = read_only,
            _ => panic!("read_only is valid only for a text area"),
        }
        self
    }

    /// Declares a text area's revisioned semantic highlight spans.
    pub fn highlight_spans(mut self, spans: crate::HighlightSpans) -> Self {
        match &mut self.props {
            Props::TextArea { spans: value, .. } => *value = spans,
            _ => panic!("highlight_spans is valid only for a text area"),
        }
        self
    }

    /// Controls a text area's selection.
    ///
    /// Applying a selection that differs from the native one also scrolls
    /// the caret into view; echoing the selection last reported through
    /// [`Element::on_selection_change`] leaves the native view untouched.
    pub fn text_selection(mut self, selection: crate::TextSelection) -> Self {
        match &mut self.props {
            Props::TextArea {
                selection: value, ..
            } => *value = Some(selection),
            _ => panic!("text_selection is valid only for a text area"),
        }
        self
    }

    /// Handles native selection changes in a text area.
    pub fn on_selection_change(mut self, handler: impl Fn(crate::TextSelection) + 'static) -> Self {
        match &self.props {
            Props::TextArea { .. } => {
                self.handlers.selection_change = Some(Rc::new(handler) as SelectionChangeHandler);
            }
            _ => panic!("on_selection_change is valid only for a text area"),
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

    /// Sets an image's semantic scaling mode.
    pub fn image_scaling(mut self, scaling: ImageScaling) -> Self {
        match &mut self.props {
            Props::Image { scaling: value, .. } => *value = scaling,
            _ => panic!("image_scaling is valid only for an image"),
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

    /// Handles element-local pointer events on an owned-drawing canvas.
    pub fn on_pointer(mut self, handler: impl Fn(crate::PointerEvent) + 'static) -> Self {
        match &self.props {
            Props::Canvas { .. } => {
                self.handlers.pointer = Some(Rc::new(handler) as crate::PointerHandler);
            }
            _ => panic!("on_pointer is valid only for a canvas"),
        }
        self
    }

    /// Declares the window's accelerator table on its content root.
    ///
    /// Entries map key chords to messages and are reconciled like every other
    /// description: re-rendering with different entries adds, removes,
    /// enables, or disables chords without reconnecting the platform's native
    /// key source. Validation rejects a table declared below the root.
    pub fn accelerators(mut self, entries: impl IntoIterator<Item = Accelerator>) -> Self {
        self.accelerators = entries.into_iter().collect();
        self
    }

    /// Returns the declared accelerator table.
    pub fn accelerator_table(&self) -> &[Accelerator] {
        &self.accelerators
    }

    /// Attaches a declarative native context menu to this element.
    ///
    /// The platform opens the menu through its contextual interaction
    /// (secondary click, ctrl-click, keyboard menu key, or the accessibility
    /// show-menu action), anchored at the interaction point. Activation
    /// dispatches through the element's stable event binding, so handlers
    /// stay current across renders without reconnecting the native menu.
    pub fn context_menu(mut self, entries: impl IntoIterator<Item = MenuEntry>) -> Self {
        self.handlers.context_menu = Some(ContextMenu::new(entries));
        self
    }

    /// Returns the attached declarative context menu model.
    pub fn context_menu_model(&self) -> Option<&ContextMenu> {
        self.handlers.context_menu.as_ref()
    }

    /// Accepts operating-system file drops on this element.
    ///
    /// The handler receives the dropped file paths and the drop position in
    /// this element's local coordinates. Valid on layout containers, native
    /// lists, and canvases — the surfaces whose platform realizations serve
    /// a drop region.
    pub fn on_file_drop(mut self, handler: impl Fn(FileDrop) + 'static) -> Self {
        match self.kind() {
            ElementKind::Stack | ElementKind::List | ElementKind::Canvas => {
                self.handlers.file_drop = Some(Rc::new(handler));
                self.handlers
                    .drop_target
                    .get_or_insert_with(DropTarget::default)
                    .accept_files();
            }
            _ => panic!("on_file_drop is valid only for a stack, list, or canvas"),
        }
        self
    }

    /// Makes this list row draggable out of the application as a promised
    /// file.
    ///
    /// The promise's write callback materializes the content lazily, only
    /// when the destination accepts the drop. A row may declare a file
    /// promise and a typed payload together: one native drag session then
    /// carries both representations, and the destination consumes the flavor
    /// it understands.
    pub fn draggable_file(mut self, promise: FilePromise) -> Self {
        match self.kind() {
            ElementKind::ListRow => self.handlers.file_promise = Some(promise),
            _ => panic!("draggable_file is valid only for a list row"),
        }
        self
    }

    /// Makes this list row draggable within the application with a typed
    /// payload.
    pub fn drag_payload(mut self, payload: DragPayload) -> Self {
        match self.kind() {
            ElementKind::ListRow => self.handlers.drag_payload = Some(payload),
            _ => panic!("drag_payload is valid only for a list row"),
        }
        self
    }

    /// Accepts typed intra-application payload drops on this element.
    ///
    /// `types` lists the accepted payload type identifiers; the handler
    /// receives the payload plus the drop position in this element's local
    /// coordinates. Valid on layout containers, native lists, list rows, and
    /// canvases.
    pub fn on_drop_accepting(
        mut self,
        types: impl IntoIterator<Item = impl Into<String>>,
        handler: impl Fn(PayloadDrop) + 'static,
    ) -> Self {
        match self.kind() {
            ElementKind::Stack | ElementKind::List | ElementKind::ListRow | ElementKind::Canvas => {
                self.handlers.payload_drop = Some(Rc::new(handler));
                self.handlers
                    .drop_target
                    .get_or_insert_with(DropTarget::default)
                    .accept_payload_types(types.into_iter().map(Into::into));
            }
            _ => panic!("on_drop_accepting is valid only for a stack, list, list row, or canvas"),
        }
        self
    }

    /// Returns the attached declarative file-promise drag-source model.
    pub fn file_promise_model(&self) -> Option<&FilePromise> {
        self.handlers.file_promise.as_ref()
    }

    /// Returns the attached declarative typed-payload drag-source model.
    pub fn drag_payload_model(&self) -> Option<&DragPayload> {
        self.handlers.drag_payload.as_ref()
    }

    /// Returns the attached declarative drop-target model.
    pub fn drop_target_model(&self) -> Option<&DropTarget> {
        self.handlers.drop_target.as_ref()
    }

    pub(crate) fn take_children(&mut self) -> Vec<Self> {
        std::mem::take(&mut self.children)
    }

    pub(crate) fn take_accelerators(&mut self) -> Vec<Accelerator> {
        std::mem::take(&mut self.accelerators)
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
            .field("accelerators", &self.accelerators)
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

/// Creates a multi-line editable text view backed by the platform's native
/// text control.
///
/// The document is reconciled by revision under the controlled-text protocol
/// documented on [`crate::TextContent`]: native edits arrive as
/// [`crate::TextChange`] deltas through `handler`, the application applies
/// them to its own copy, and echoes the event's revision on the next render
/// so reconciliation never disturbs in-flight typing or IME composition.
pub fn text_area(
    content: crate::TextContent,
    accessibility_label: impl Into<String>,
    handler: impl Fn(crate::TextChange) + 'static,
) -> Element {
    let mut element = Element::leaf(Props::TextArea {
        content,
        spans: crate::HighlightSpans::none(),
        selection: None,
        read_only: false,
        role: TextRole::Body,
        accessibility_label: accessibility_label.into(),
    });
    element.handlers.text_change = Some(Rc::new(handler) as TextChangeHandler);
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

/// Creates a native bitmap image view from decoded RGBA content.
///
/// The picture fits its view proportionally by default; select another
/// mapping with [`Element::image_scaling`].
pub fn image(content: ImageContent, accessibility_label: impl Into<String>) -> Element {
    Element::leaf(Props::Image {
        content,
        scaling: ImageScaling::Fit,
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

/// Mounts a standard desktop UI pattern with children in its declared region order.
pub fn mount_pattern(
    pattern: crate::UiPattern,
    regions: impl IntoIterator<Item = Element>,
) -> Element {
    Element::parent(Props::Pattern { pattern }, regions)
}

/// Creates a native list.
pub fn list(
    accessibility_label: impl Into<String>,
    rows: impl IntoIterator<Item = Element>,
) -> Element {
    Element::parent(
        Props::List {
            accessibility_label: accessibility_label.into(),
            pattern: CollectionPattern::ContentList,
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

/// Creates an owned-drawing canvas surface.
///
/// The canvas is reserved for content that is inherently graphical — a
/// terminal cell grid, an audio meter, a dashboard widget face. It is not an
/// escape hatch for imitating native controls: a canvas that draws a fake
/// button, list, input, or any other control violates the design contract.
///
/// `size` is the intrinsic content extent in logical points and `scene` is
/// the recorded display list the application rebuilds each render; the
/// reconciler compares scenes by value and patches the native surface only
/// when the drawing actually changed.
pub fn canvas(
    size: crate::CanvasSize,
    scene: crate::DrawScene,
    accessibility_label: impl Into<String>,
) -> Element {
    Element::leaf(Props::Canvas {
        size,
        scene,
        accessibility_label: accessibility_label.into(),
    })
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

    #[test]
    fn image_accepts_a_semantic_scaling_mode() {
        let content = ImageContent::from_rgba8(2, 2, 8, vec![0_u8; 32], 1);
        let element = image(content, "Preview").image_scaling(ImageScaling::Center);

        assert!(matches!(
            element.props(),
            Props::Image {
                scaling: ImageScaling::Center,
                ..
            }
        ));
    }

    #[test]
    fn image_content_identity_follows_geometry_density_and_revision() {
        let same_picture_new_allocation = (
            ImageContent::from_rgba8(2, 2, 8, vec![1_u8; 32], 9).with_scale(2.0),
            ImageContent::from_rgba8(2, 2, 8, vec![1_u8; 32], 9).with_scale(2.0),
        );
        assert_eq!(same_picture_new_allocation.0, same_picture_new_allocation.1);

        let base = ImageContent::from_rgba8(2, 2, 8, vec![1_u8; 32], 9);
        assert_ne!(base, ImageContent::from_rgba8(2, 2, 8, vec![1_u8; 32], 10));
        assert_ne!(
            base,
            ImageContent::from_rgba8(2, 2, 8, vec![1_u8; 32], 9).with_scale(2.0)
        );
    }
}
