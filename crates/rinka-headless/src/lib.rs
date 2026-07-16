//! Deterministic retained-tree adapter used by tests and surface extraction.

mod clipboard;
mod text_input;

pub use clipboard::FakeClipboard;
pub use text_input::SyntheticTextInput;

use rinka_core::{
    ContextMenu, DialogDescription, DialogOutcome, DialogRequest, DialogResponder, DialogService,
    DragPayload, DropPosition, DropTarget, Element, EventBindings, FileDrop, FilePromise,
    MonospaceMetrics, NativeBackend, PayloadDrop, PropertyPatch, Props, TextChange, TextEdit,
    TextRevision, TextSelection, TextSyncAction,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

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
    file_promise: Option<FilePromise>,
    drag_payload: Option<DragPayload>,
    drop_target: Option<DropTarget>,
    events: EventBindings,
    children: Vec<Handle>,
    text_area: Option<TextAreaModel>,
}

struct PresentedDialog {
    description: DialogDescription,
    responder: Option<DialogResponder>,
}

/// Deterministic window-modal dialog service for headless consumer tests.
///
/// The service records every request it receives, in order, and lets a
/// test script the user's answer by delivering a [`DialogOutcome`] through
/// the retained single-use responder — the headless stand-in for a native
/// sheet completing. Clones share the same recording, so a test keeps one
/// clone and injects another through
/// [`rinka_core::PlatformServices::with_dialog_service`].
#[derive(Clone, Default)]
pub struct FakeDialogPresenter {
    dialogs: Rc<RefCell<Vec<PresentedDialog>>>,
}

impl FakeDialogPresenter {
    /// Creates a presenter with no recorded presentations.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns how many dialogs have been presented so far.
    pub fn presented_count(&self) -> usize {
        self.dialogs.borrow().len()
    }

    /// Reads the description of the presentation at `index`.
    pub fn description(&self, index: usize) -> Option<DialogDescription> {
        self.dialogs
            .borrow()
            .get(index)
            .map(|dialog| dialog.description.clone())
    }

    /// Scripts the outcome of the presentation at `index`.
    ///
    /// Returns whether a responder was still retained; a second delivery to
    /// the same presentation returns `false` because native completion
    /// handlers run exactly once.
    pub fn deliver(&self, index: usize, outcome: DialogOutcome) -> bool {
        let responder = self
            .dialogs
            .borrow_mut()
            .get_mut(index)
            .and_then(|dialog| dialog.responder.take());
        match responder {
            Some(responder) => {
                responder.deliver(outcome);
                true
            }
            None => false,
        }
    }
}

impl DialogService for FakeDialogPresenter {
    fn present(&self, request: DialogRequest) {
        let (description, responder) = request.into_parts();
        self.dialogs.borrow_mut().push(PresentedDialog {
            description,
            responder: Some(responder),
        });
    }
}

impl fmt::Debug for FakeDialogPresenter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FakeDialogPresenter")
            .field("presented", &self.presented_count())
            .finish()
    }
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
                file_promise: None,
                drag_payload: None,
                drop_target: None,
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

    /// Returns the current native file-promise drag-source model.
    pub fn file_promise_of(&self, handle: Handle) -> Option<&FilePromise> {
        self.nodes
            .get(&handle)
            .and_then(|node| node.file_promise.as_ref())
    }

    /// Returns the current native typed-payload drag-source model.
    pub fn drag_payload_of(&self, handle: Handle) -> Option<&DragPayload> {
        self.nodes
            .get(&handle)
            .and_then(|node| node.drag_payload.as_ref())
    }

    /// Returns the current native drop-target model.
    pub fn drop_target_of(&self, handle: Handle) -> Option<&DropTarget> {
        self.nodes
            .get(&handle)
            .and_then(|node| node.drop_target.as_ref())
    }

    /// Simulates the operating system dropping files at a point inside the
    /// target element, expressed in the target's local coordinates.
    ///
    /// Delivery goes through the target's stable event binding, which
    /// refuses the drop — exactly like a native session's validation phase —
    /// when the current drop-target model does not accept files.
    pub fn simulate_file_drop(
        &self,
        target: Handle,
        paths: impl IntoIterator<Item = PathBuf>,
        position: DropPosition,
    ) -> Result<(), HeadlessError> {
        let node = self
            .nodes
            .get(&target)
            .ok_or_else(|| HeadlessError(format!("unknown drop target {}", target.0)))?;
        if node.events.emit_file_drop(FileDrop {
            paths: paths.into_iter().collect(),
            position,
        }) {
            Ok(())
        } else {
            Err(HeadlessError(format!(
                "target {} refused the file drop",
                target.0
            )))
        }
    }

    /// Simulates one complete intra-application drag session: the source's
    /// current typed payload is dropped onto the target at a point in the
    /// target's local coordinates.
    ///
    /// Returns the delivered payload. The session fails deterministically
    /// when the source declares no payload or the target's current model
    /// does not accept the payload's type — the same refusals a native
    /// session's validation phase produces.
    pub fn simulate_payload_drag(
        &self,
        source: Handle,
        target: Handle,
        position: DropPosition,
    ) -> Result<DragPayload, HeadlessError> {
        let source_node = self
            .nodes
            .get(&source)
            .ok_or_else(|| HeadlessError(format!("unknown drag source {}", source.0)))?;
        let payload = source_node.events.drag_payload().ok_or_else(|| {
            HeadlessError(format!("source {} declares no drag payload", source.0))
        })?;
        let target_node = self
            .nodes
            .get(&target)
            .ok_or_else(|| HeadlessError(format!("unknown drop target {}", target.0)))?;
        if target_node.events.emit_payload_drop(PayloadDrop {
            payload: payload.clone(),
            position,
        }) {
            Ok(payload)
        } else {
            Err(HeadlessError(format!(
                "target {} refused the '{}' payload",
                target.0,
                payload.payload_type()
            )))
        }
    }

    /// Simulates a destination accepting the source's promised file: the
    /// promise's write callback materializes the content inside the
    /// destination directory, exactly once, and the written path returns.
    pub fn materialize_file_promise(
        &self,
        source: Handle,
        destination_directory: &Path,
    ) -> Result<PathBuf, HeadlessError> {
        let node = self
            .nodes
            .get(&source)
            .ok_or_else(|| HeadlessError(format!("unknown drag source {}", source.0)))?;
        let promise = node.events.file_promise().ok_or_else(|| {
            HeadlessError(format!("source {} declares no file promise", source.0))
        })?;
        let destination = destination_directory.join(promise.file_name());
        promise
            .write_to(&destination)
            .map_err(|reason| HeadlessError(format!("promised write failed: {reason}")))?;
        Ok(destination)
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
                file_promise: element.file_promise_model().cloned(),
                drag_payload: element.drag_payload_model().cloned(),
                drop_target: element.drop_target_model().cloned(),
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
        node.file_promise = patch.file_promise().cloned();
        node.drag_payload = patch.drag_payload().cloned();
        node.drop_target = patch.drop_target().cloned();
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
