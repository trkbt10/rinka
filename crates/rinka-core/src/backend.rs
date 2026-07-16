//! Native host mutation contract and property deltas.

use crate::{
    Align, Axis, ButtonMaterial, ButtonRole, ControlSize, Element, InputKind, Justify, ListRowRole,
    ListStyle, Props, Spacing, SplitRole, StatusTone, Symbol, TableColumn, TextRole,
};
use std::fmt;

/// Minimal update to an existing native object.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyPatch {
    /// Static text and typography.
    Label {
        /// New text.
        text: String,
        /// New role.
        role: TextRole,
        /// Selection behavior.
        selectable: bool,
    },
    /// Button presentation.
    Button {
        /// New title.
        label: String,
        /// New role.
        role: ButtonRole,
        /// New native metric.
        size: ControlSize,
        /// New native backing material.
        material: ButtonMaterial,
        /// New enabled state.
        enabled: bool,
        /// New hover help.
        tooltip: Option<String>,
        /// New accessible name.
        accessibility_label: String,
    },
    /// Input presentation.
    Input {
        /// New controlled value.
        value: String,
        /// New prompt.
        placeholder: String,
        /// Native variant.
        kind: InputKind,
        /// New enabled state.
        enabled: bool,
        /// New accessible name.
        accessibility_label: String,
    },
    /// Toggle presentation.
    Toggle {
        /// New title.
        label: String,
        /// New controlled state.
        value: bool,
        /// New native metric.
        size: ControlSize,
        /// New enabled state.
        enabled: bool,
        /// New accessible name.
        accessibility_label: String,
    },
    /// Progress value.
    Progress {
        /// New fraction.
        fraction: f64,
        /// New accessible description.
        accessibility_label: String,
    },
    /// Separator direction.
    Separator {
        /// New direction.
        axis: Axis,
    },
    /// Stack layout.
    Stack {
        /// New direction.
        axis: Axis,
        /// New spacing intent.
        spacing: Spacing,
        /// New inset intent.
        padding: Option<Spacing>,
        /// New cross alignment.
        align: Align,
        /// New primary-axis placement.
        justify: Justify,
    },
    /// Spacer growth.
    Spacer {
        /// Horizontal growth.
        horizontal: bool,
        /// Vertical growth.
        vertical: bool,
    },
    /// Scrolling direction.
    Scroll {
        /// New direction.
        axis: Axis,
    },
    /// Split-view presentation.
    Split {
        /// New semantic role.
        role: SplitRole,
        /// New collapse behavior.
        collapsible: bool,
    },
    /// Three-region navigation workspace presentation.
    Workspace {
        /// New sidebar collapse behavior.
        sidebar_collapsible: bool,
        /// New inspector collapse behavior.
        inspector_collapsible: bool,
    },
    /// List presentation.
    List {
        /// New accessible name.
        accessibility_label: String,
        /// New native treatment.
        style: ListStyle,
        /// New native table columns.
        columns: Vec<TableColumn>,
    },
    /// List row presentation.
    ListRow {
        /// New title.
        title: String,
        /// New subtitle.
        subtitle: Option<String>,
        /// New values for table columns after the title.
        cells: Vec<String>,
        /// New source-list row semantics.
        role: ListRowRole,
        /// New source-list expansion state.
        expanded: bool,
        /// New system symbol.
        symbol: Option<Symbol>,
        /// New selection state.
        selected: bool,
        /// New disclosure state.
        disclosure: bool,
        /// New accessible name.
        accessibility_label: String,
    },
    /// Status presentation.
    Status {
        /// New title.
        title: String,
        /// New message.
        message: String,
        /// New tone.
        tone: StatusTone,
    },
}

impl PropertyPatch {
    pub(crate) fn between(old: &Props, new: &Props) -> Option<Self> {
        if old == new {
            return None;
        }
        match new {
            Props::Label {
                text,
                role,
                selectable,
            } => Some(Self::Label {
                text: text.clone(),
                role: *role,
                selectable: *selectable,
            }),
            Props::Button {
                label,
                role,
                size,
                material,
                enabled,
                tooltip,
                accessibility_label,
            } => Some(Self::Button {
                label: label.clone(),
                role: *role,
                size: *size,
                material: *material,
                enabled: *enabled,
                tooltip: tooltip.clone(),
                accessibility_label: accessibility_label.clone(),
            }),
            Props::Input {
                value,
                placeholder,
                kind,
                enabled,
                accessibility_label,
            } => Some(Self::Input {
                value: value.clone(),
                placeholder: placeholder.clone(),
                kind: *kind,
                enabled: *enabled,
                accessibility_label: accessibility_label.clone(),
            }),
            Props::Toggle {
                label,
                value,
                size,
                enabled,
                accessibility_label,
            } => Some(Self::Toggle {
                label: label.clone(),
                value: *value,
                size: *size,
                enabled: *enabled,
                accessibility_label: accessibility_label.clone(),
            }),
            Props::Progress {
                fraction,
                accessibility_label,
            } => Some(Self::Progress {
                fraction: *fraction,
                accessibility_label: accessibility_label.clone(),
            }),
            Props::Separator { axis } => Some(Self::Separator { axis: *axis }),
            Props::Stack {
                axis,
                spacing,
                padding,
                align,
                justify,
            } => Some(Self::Stack {
                axis: *axis,
                spacing: *spacing,
                padding: *padding,
                align: *align,
                justify: *justify,
            }),
            Props::Spacer {
                horizontal,
                vertical,
            } => Some(Self::Spacer {
                horizontal: *horizontal,
                vertical: *vertical,
            }),
            Props::Scroll { axis } => Some(Self::Scroll { axis: *axis }),
            Props::Split { role, collapsible } => Some(Self::Split {
                role: *role,
                collapsible: *collapsible,
            }),
            Props::Workspace {
                sidebar_collapsible,
                inspector_collapsible,
            } => Some(Self::Workspace {
                sidebar_collapsible: *sidebar_collapsible,
                inspector_collapsible: *inspector_collapsible,
            }),
            Props::List {
                accessibility_label,
                style,
                columns,
            } => Some(Self::List {
                accessibility_label: accessibility_label.clone(),
                style: *style,
                columns: columns.clone(),
            }),
            Props::ListRow {
                title,
                subtitle,
                cells,
                role,
                expanded,
                symbol,
                selected,
                disclosure,
                accessibility_label,
            } => Some(Self::ListRow {
                title: title.clone(),
                subtitle: subtitle.clone(),
                cells: cells.clone(),
                role: *role,
                expanded: *expanded,
                symbol: *symbol,
                selected: *selected,
                disclosure: *disclosure,
                accessibility_label: accessibility_label.clone(),
            }),
            Props::Status {
                title,
                message,
                tone,
            } => Some(Self::Status {
                title: title.clone(),
                message: message.clone(),
                tone: *tone,
            }),
        }
    }
}

/// Adapter between reconciliation and a retained native view tree.
pub trait NativeBackend {
    /// Opaque native object identity.
    type Handle: Clone + fmt::Debug;
    /// Platform diagnostic.
    type Error;

    /// Returns a persistent container owned by a window.
    fn root(&self) -> Self::Handle;

    /// Checks whether one element and its semantic options are supported.
    ///
    /// The renderer calls this for the complete next tree before issuing any
    /// native mutation.
    fn validate(&self, element: &Element) -> Result<(), Self::Error>;

    /// Creates a native object without declarative children.
    fn create(
        &mut self,
        element: &Element,
        events: crate::EventBindings,
    ) -> Result<Self::Handle, Self::Error>;

    /// Applies one property update.
    fn apply(&mut self, handle: &Self::Handle, patch: &PropertyPatch) -> Result<(), Self::Error>;

    /// Inserts a child at a logical index.
    fn insert_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error>;

    /// Removes a child at a logical index.
    fn remove_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error>;

    /// Moves an existing child while preserving native identity.
    fn move_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        from: usize,
        to: usize,
    ) -> Result<(), Self::Error>;

    /// Releases adapter-owned resources associated with an object.
    fn destroy(&mut self, _handle: &Self::Handle) -> Result<(), Self::Error> {
        Ok(())
    }
}
