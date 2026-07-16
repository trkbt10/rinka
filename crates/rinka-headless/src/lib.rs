//! Deterministic retained-tree adapter used by tests and surface extraction.

mod clipboard;

pub use clipboard::FakeClipboard;

use rinka_core::{
    ContextMenu, Element, EventBindings, MonospaceMetrics, NativeBackend, PropertyPatch, Props,
    TextChange, TextEdit, TextRevision, TextSelection, TextSyncAction,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

/// Stable synthetic native handle.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Handle(u64);

/// Recorded host mutation.
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    /// Native object creation.
    Create {
        /// Created handle.
        handle: Handle,
        /// Initial properties.
        props: Props,
    },
    /// Property mutation.
    Patch {
        /// Updated handle.
        handle: Handle,
        /// Applied delta.
        patch: PropertyPatch,
    },
    /// Child insertion.
    Insert {
        /// Parent handle.
        parent: Handle,
        /// Child handle.
        child: Handle,
        /// Logical index.
        index: usize,
    },
    /// Child removal.
    Remove {
        /// Parent handle.
        parent: Handle,
        /// Child handle.
        child: Handle,
        /// Logical index.
        index: usize,
    },
    /// Child reorder.
    Move {
        /// Parent handle.
        parent: Handle,
        /// Child handle.
        child: Handle,
        /// Previous index.
        from: usize,
        /// New index.
        to: usize,
    },
    /// Native object destruction.
    Destroy {
        /// Destroyed handle.
        handle: Handle,
    },
}

/// One text mutation the modeled native buffer performed, in order.
///
/// The sequence mirrors the adapter contract of
/// [`rinka_core::TextContent::sync_action`], so reconciliation tests can
/// assert deterministically that an echo kept the buffer, a programmatic
/// delta applied in place, and a document load replaced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextAreaMutation {
    /// A declared content was recognized as an echo; the buffer was kept.
    KeptBuffer,
    /// Programmatic edits were applied in place.
    AppliedEdits {
        /// Number of sequential edits applied.
        edit_count: usize,
    },
    /// The whole buffer was replaced by declared text.
    ReplacedBuffer,
    /// A controlled selection differing from the native one was applied.
    SetSelection(TextSelection),
    /// A changed highlight-span set was applied in place.
    AppliedSpans {
        /// Applied span-set revision.
        revision: u64,
        /// Number of spans applied.
        span_count: usize,
    },
}

/// Deterministic model of one native text-area buffer.
#[derive(Clone, Debug)]
pub struct TextAreaModel {
    /// Modeled native document.
    pub buffer: String,
    /// Modeled native document revision.
    pub revision: TextRevision,
    /// Modeled native selection.
    pub selection: Option<TextSelection>,
    /// Last applied highlight-span revision.
    pub spans_revision: u64,
    /// Last applied highlight-span count.
    pub span_count: usize,
    /// Whether the modeled view rejects user edits.
    pub read_only: bool,
    /// Ordered log of buffer mutations since mounting.
    pub mutations: Vec<TextAreaMutation>,
}

#[derive(Clone, Debug)]
struct Node {
    key: Option<String>,
    props: Props,
    context_menu: Option<ContextMenu>,
    events: EventBindings,
    children: Vec<Handle>,
    text_area: Option<TextAreaModel>,
}

/// Deterministic adapter diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadlessError(String);

impl fmt::Display for HeadlessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for HeadlessError {}

/// In-memory model of a retained native view tree.
#[derive(Debug)]
pub struct HeadlessBackend {
    root: Handle,
    next: u64,
    nodes: BTreeMap<Handle, Node>,
    operations: Vec<Operation>,
}

impl HeadlessBackend {
    /// Creates a backend with one persistent host root.
    pub fn new() -> Self {
        let root = Handle(0);
        let mut nodes = BTreeMap::new();
        nodes.insert(
            root,
            Node {
                key: Some("__host_root__".to_owned()),
                props: Props::Stack {
                    axis: rinka_core::Axis::Vertical,
                    spacing: rinka_core::Spacing::Related,
                    padding: None,
                    align: rinka_core::Align::Stretch,
                    justify: rinka_core::Justify::Start,
                },
                context_menu: None,
                events: EventBindings::default(),
                children: Vec::new(),
                text_area: None,
            },
        );
        Self {
            root,
            next: 1,
            nodes,
            operations: Vec::new(),
        }
    }

    /// Returns recorded mutations.
    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    /// Clears recorded mutations without changing the tree.
    pub fn clear_operations(&mut self) {
        self.operations.clear();
    }

    /// Finds a mounted native object by declarative key.
    pub fn find_by_key(&self, key: &str) -> Option<Handle> {
        self.nodes
            .iter()
            .find_map(|(handle, node)| (node.key.as_deref() == Some(key)).then_some(*handle))
    }

    /// Returns children in native order.
    pub fn children_of(&self, handle: Handle) -> Option<&[Handle]> {
        self.nodes.get(&handle).map(|node| node.children.as_slice())
    }

    /// Returns current native properties.
    pub fn props_of(&self, handle: Handle) -> Option<&Props> {
        self.nodes.get(&handle).map(|node| &node.props)
    }

    /// Returns the current native context-menu model.
    pub fn context_menu_of(&self, handle: Handle) -> Option<&ContextMenu> {
        self.nodes
            .get(&handle)
            .and_then(|node| node.context_menu.as_ref())
    }

    /// Returns the stable event target.
    pub fn events_of(&self, handle: Handle) -> Option<EventBindings> {
        self.nodes.get(&handle).map(|node| node.events.clone())
    }

    /// Returns the modeled native text-area buffer.
    pub fn text_area_model(&self, handle: Handle) -> Option<&TextAreaModel> {
        self.nodes
            .get(&handle)
            .and_then(|node| node.text_area.as_ref())
    }

    /// Performs one native user edit on a modeled text area, exactly as a
    /// platform view would: the buffer changes first, the edit revision
    /// advances, and the returned delta-only [`TextChange`] is what the
    /// adapter reports. Emit it through [`Self::events_of`] to drive the
    /// application round trip.
    pub fn commit_text_edit(
        &mut self,
        handle: Handle,
        edits: Vec<TextEdit>,
    ) -> Result<TextChange, HeadlessError> {
        let node = self.node_mut(&handle)?;
        let model = node
            .text_area
            .as_mut()
            .ok_or_else(|| HeadlessError(format!("handle {} is not a text area", handle.0)))?;
        if model.read_only {
            return Err(HeadlessError(
                "a read-only text area rejects user edits".to_owned(),
            ));
        }
        let buffer = TextEdit::apply_all(&model.buffer, &edits).ok_or_else(|| {
            HeadlessError("a native edit addressed characters outside the buffer".to_owned())
        })?;
        let base_revision = model.revision;
        let revision = base_revision.next_edit();
        model.buffer = buffer;
        model.revision = revision;
        Ok(TextChange {
            base_revision,
            revision,
            edits,
            composing: false,
        })
    }

    /// Performs one native selection change on a modeled text area and
    /// returns the selection the adapter would report.
    pub fn commit_text_selection(
        &mut self,
        handle: Handle,
        selection: TextSelection,
    ) -> Result<TextSelection, HeadlessError> {
        let node = self.node_mut(&handle)?;
        let model = node
            .text_area
            .as_mut()
            .ok_or_else(|| HeadlessError(format!("handle {} is not a text area", handle.0)))?;
        model.selection = Some(selection);
        Ok(selection)
    }

    fn node_mut(&mut self, handle: &Handle) -> Result<&mut Node, HeadlessError> {
        self.nodes
            .get_mut(handle)
            .ok_or_else(|| HeadlessError(format!("unknown handle {}", handle.0)))
    }
}

impl Default for HeadlessBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeBackend for HeadlessBackend {
    type Handle = Handle;
    type Error = HeadlessError;

    fn root(&self) -> Self::Handle {
        self.root
    }

    fn validate(&self, _element: &Element) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Returns the synthetic headless font model.
    ///
    /// The values are deterministic stand-ins for tests, not platform truth:
    /// the row advances one and a half times the font size and every glyph
    /// advances six tenths of it, which preserves the monospace invariants a
    /// terminal grid depends on (constant row height, constant glyph width).
    fn monospace_metrics(&self, font_size: f64) -> Option<MonospaceMetrics> {
        if !font_size.is_finite() || font_size <= 0.0 {
            return None;
        }
        Some(MonospaceMetrics {
            row_height: font_size * 1.5,
            glyph_width: font_size * 0.6,
        })
    }

    fn create(
        &mut self,
        element: &Element,
        events: EventBindings,
    ) -> Result<Self::Handle, Self::Error> {
        let handle = Handle(self.next);
        self.next += 1;
        let props = element.props().clone();
        let text_area = match &props {
            Props::TextArea {
                content,
                spans,
                selection,
                read_only,
                ..
            } => Some(TextAreaModel {
                buffer: content.text().to_owned(),
                revision: content.revision(),
                selection: *selection,
                spans_revision: spans.revision(),
                span_count: spans.spans().len(),
                read_only: *read_only,
                mutations: Vec::new(),
            }),
            _ => None,
        };
        self.nodes.insert(
            handle,
            Node {
                key: element.key().map(|key| key.as_str().to_owned()),
                props: props.clone(),
                context_menu: element.context_menu_model().cloned(),
                events,
                children: Vec::new(),
                text_area,
            },
        );
        self.operations.push(Operation::Create { handle, props });
        Ok(handle)
    }

    fn apply(&mut self, handle: &Self::Handle, patch: &PropertyPatch) -> Result<(), Self::Error> {
        let node = self.node_mut(handle)?;
        if let Props::TextArea {
            content,
            spans,
            selection,
            read_only,
            ..
        } = patch.props()
        {
            let model = node
                .text_area
                .as_mut()
                .ok_or_else(|| HeadlessError(format!("handle {} is not a text area", handle.0)))?;
            match content.sync_action(model.revision) {
                TextSyncAction::Keep => model.mutations.push(TextAreaMutation::KeptBuffer),
                TextSyncAction::ApplyEdits(edits) => {
                    model.buffer = TextEdit::apply_all(&model.buffer, edits).ok_or_else(|| {
                        HeadlessError(
                            "declared edits addressed characters outside the buffer".to_owned(),
                        )
                    })?;
                    model.revision = content.revision();
                    model.mutations.push(TextAreaMutation::AppliedEdits {
                        edit_count: edits.len(),
                    });
                }
                TextSyncAction::Replace => {
                    model.buffer = content.text().to_owned();
                    model.revision = content.revision();
                    model.mutations.push(TextAreaMutation::ReplacedBuffer);
                }
            }
            if spans.revision() != model.spans_revision {
                model.spans_revision = spans.revision();
                model.span_count = spans.spans().len();
                model.mutations.push(TextAreaMutation::AppliedSpans {
                    revision: spans.revision(),
                    span_count: spans.spans().len(),
                });
            }
            if let Some(selection) = selection
                && model.selection != Some(*selection)
            {
                model.selection = Some(*selection);
                model
                    .mutations
                    .push(TextAreaMutation::SetSelection(*selection));
            }
            model.read_only = *read_only;
        }
        node.props.clone_from(patch.props());
        node.context_menu = patch.context_menu().cloned();
        self.operations.push(Operation::Patch {
            handle: *handle,
            patch: patch.clone(),
        });
        Ok(())
    }

    fn insert_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        if !self.nodes.contains_key(child) {
            return Err(HeadlessError(format!("unknown child {}", child.0)));
        }
        let node = self.node_mut(parent)?;
        if index > node.children.len() {
            return Err(HeadlessError(format!(
                "insert index {index} exceeds child count {}",
                node.children.len()
            )));
        }
        node.children.insert(index, *child);
        self.operations.push(Operation::Insert {
            parent: *parent,
            child: *child,
            index,
        });
        Ok(())
    }

    fn remove_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        let node = self.node_mut(parent)?;
        if node.children.get(index) != Some(child) {
            return Err(HeadlessError(format!(
                "child {} is not at index {index}",
                child.0
            )));
        }
        node.children.remove(index);
        self.operations.push(Operation::Remove {
            parent: *parent,
            child: *child,
            index,
        });
        Ok(())
    }

    fn move_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        from: usize,
        to: usize,
    ) -> Result<(), Self::Error> {
        let node = self.node_mut(parent)?;
        if node.children.get(from) != Some(child) || to >= node.children.len() {
            return Err(HeadlessError(format!(
                "cannot move child {} from {from} to {to}",
                child.0
            )));
        }
        let moved = node.children.remove(from);
        node.children.insert(to, moved);
        self.operations.push(Operation::Move {
            parent: *parent,
            child: *child,
            from,
            to,
        });
        Ok(())
    }

    fn destroy(&mut self, handle: &Self::Handle) -> Result<(), Self::Error> {
        self.nodes
            .remove(handle)
            .ok_or_else(|| HeadlessError(format!("unknown handle {}", handle.0)))?;
        self.operations.push(Operation::Destroy { handle: *handle });
        Ok(())
    }
}
