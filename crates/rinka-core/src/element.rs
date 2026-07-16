//! Immutable declarative element descriptions and semantic roles.

use crate::event::{EventHandlers, InputHandler};
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
}
