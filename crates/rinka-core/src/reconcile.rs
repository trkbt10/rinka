//! Keyed reconciliation into retained native objects.

use crate::{Element, ElementKind, EventBindings, NativeBackend, PropertyPatch};
use std::collections::HashSet;
use std::error::Error;
use std::fmt;

/// Invalid declarative tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TreeError {
    /// Two siblings used the same key.
    DuplicateKey {
        /// Duplicated key text.
        key: String,
        /// Parent path used for diagnostics.
        parent: String,
    },
    /// A container received an invalid child count.
    ChildCount {
        /// Element path.
        path: String,
        /// Required minimum.
        minimum: usize,
        /// Required maximum.
        maximum: usize,
        /// Actual count.
        actual: usize,
    },
    /// A non-row element was placed directly in a list.
    InvalidListChild {
        /// Child path.
        path: String,
    },
    /// A list row used semantics that its presentation cannot represent.
    InvalidListSchema {
        /// Element path.
        path: String,
        /// Human-readable invariant violation.
        reason: String,
    },
    /// A table declared invalid columns or row values.
    InvalidTableSchema {
        /// Element path.
        path: String,
        /// Human-readable invariant violation.
        reason: String,
    },
    /// A mounted native window attempted to replace its promoted root class.
    WindowRootKindChanged {
        /// Root kind retained by the native window.
        previous: ElementKind,
        /// Root kind requested by the next component view.
        next: ElementKind,
    },
}

impl fmt::Display for TreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateKey { key, parent } => {
                write!(formatter, "duplicate sibling key '{key}' under {parent}")
            }
            Self::ChildCount {
                path,
                minimum,
                maximum,
                actual,
            } => write!(
                formatter,
                "{path} requires {minimum}..={maximum} children, received {actual}"
            ),
            Self::InvalidListChild { path } => {
                write!(formatter, "list child at {path} is not a list row")
            }
            Self::InvalidListSchema { path, reason } => {
                write!(formatter, "invalid list at {path}: {reason}")
            }
            Self::InvalidTableSchema { path, reason } => {
                write!(formatter, "invalid table at {path}: {reason}")
            }
            Self::WindowRootKindChanged { previous, next } => write!(
                formatter,
                "native window root kind must remain stable: mounted {previous:?}, received {next:?}"
            ),
        }
    }
}

impl Error for TreeError {}

/// Reconciliation diagnostic.
#[derive(Debug)]
pub enum RenderError<E> {
    /// The declarative tree violates a structural invariant.
    Tree(TreeError),
    /// The native adapter rejected an operation.
    Backend(E),
}

impl<E: fmt::Display> fmt::Display for RenderError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tree(error) => error.fmt(formatter),
            Self::Backend(error) => error.fmt(formatter),
        }
    }
}

impl<E: Error + 'static> Error for RenderError<E> {}

/// Mutation totals from one render.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RenderStats {
    /// New native objects.
    pub created: usize,
    /// Destroyed native objects.
    pub removed: usize,
    /// Reordered native objects.
    pub moved: usize,
    /// Patched native objects.
    pub patched: usize,
    /// Reused native objects.
    pub reused: usize,
    /// Replaced native objects.
    pub replaced: usize,
}

/// Declarative description paired with its retained native identity.
#[derive(Debug)]
pub struct MountedNode<H> {
    handle: H,
    descriptor: Element,
    events: EventBindings,
    children: Vec<Self>,
}

impl<H> MountedNode<H> {
    /// Returns the native handle.
    pub fn handle(&self) -> &H {
        &self.handle
    }

    /// Returns the last rendered descriptor.
    pub fn element(&self) -> &Element {
        &self.descriptor
    }

    /// Returns the stable event target connected to this retained identity.
    pub fn events(&self) -> &EventBindings {
        &self.events
    }

    /// Returns mounted children.
    pub fn children(&self) -> &[Self] {
        &self.children
    }
}

/// Stateful keyed renderer.
#[derive(Debug)]
pub struct Renderer<B: NativeBackend> {
    backend: B,
    mounted: Option<MountedNode<B::Handle>>,
    last_stats: RenderStats,
}

impl<B: NativeBackend> Renderer<B> {
    /// Creates an empty renderer over a persistent native root.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            mounted: None,
            last_stats: RenderStats::default(),
        }
    }

    /// Validates and reconciles a complete next tree.
    pub fn render(&mut self, next: Element) -> Result<RenderStats, RenderError<B::Error>> {
        validate_tree(&next).map_err(RenderError::Tree)?;
        validate_backend(&self.backend, &next).map_err(RenderError::Backend)?;

        let root = self.backend.root();
        let mut stats = RenderStats::default();
        let mounted = match self.mounted.take() {
            None => {
                let mounted = mount_subtree(&mut self.backend, next, &mut stats)
                    .map_err(RenderError::Backend)?;
                self.backend
                    .insert_child(&root, &mounted.handle, 0)
                    .map_err(RenderError::Backend)?;
                mounted
            }
            Some(current) => reconcile_node(&mut self.backend, &root, 0, current, next, &mut stats)
                .map_err(RenderError::Backend)?,
        };
        self.mounted = Some(mounted);
        self.last_stats = stats;
        Ok(stats)
    }

    /// Returns the adapter.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Returns the mutable adapter.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Returns the mounted root.
    pub fn mounted(&self) -> Option<&MountedNode<B::Handle>> {
        self.mounted.as_ref()
    }

    /// Returns statistics from the latest successful render.
    pub fn last_stats(&self) -> RenderStats {
        self.last_stats
    }

    /// Consumes the renderer and returns the adapter.
    pub fn into_backend(self) -> B {
        self.backend
    }
}

fn validate_backend<B: NativeBackend>(backend: &B, element: &Element) -> Result<(), B::Error> {
    backend.validate(element)?;
    for child in element.children() {
        validate_backend(backend, child)?;
    }
    Ok(())
}

fn validate_tree(root: &Element) -> Result<(), TreeError> {
    validate_node(root, "root")
}

fn validate_node(element: &Element, path: &str) -> Result<(), TreeError> {
    let children = element.children();
    let mut keys = HashSet::new();
    for child in children {
        if let Some(key) = child.key()
            && !keys.insert(key.as_str())
        {
            return Err(TreeError::DuplicateKey {
                key: key.as_str().to_owned(),
                parent: path.to_owned(),
            });
        }
    }

    let exact = match element.kind() {
        crate::ElementKind::Scroll => Some(1),
        crate::ElementKind::Split => Some(2),
        crate::ElementKind::Workspace => Some(3),
        crate::ElementKind::Label
        | crate::ElementKind::Button
        | crate::ElementKind::Input
        | crate::ElementKind::Toggle
        | crate::ElementKind::Progress
        | crate::ElementKind::Separator
        | crate::ElementKind::Spacer
        | crate::ElementKind::Status => Some(0),
        crate::ElementKind::Stack | crate::ElementKind::List | crate::ElementKind::ListRow => None,
    };
    if let Some(exact) = exact
        && children.len() != exact
    {
        return Err(TreeError::ChildCount {
            path: path.to_owned(),
            minimum: exact,
            maximum: exact,
            actual: children.len(),
        });
    }

    if element.kind() == crate::ElementKind::List {
        for (index, child) in children.iter().enumerate() {
            if child.kind() != crate::ElementKind::ListRow {
                return Err(TreeError::InvalidListChild {
                    path: format!("{path}/{index}"),
                });
            }
        }
        validate_table_schema(element, path)?;
    }
    if element.kind() == crate::ElementKind::ListRow {
        for (index, child) in children.iter().enumerate() {
            if child.kind() != crate::ElementKind::ListRow {
                return Err(TreeError::InvalidListChild {
                    path: format!("{path}/{index}"),
                });
            }
        }
    }

    for (index, child) in children.iter().enumerate() {
        let name = child
            .key()
            .map_or_else(|| index.to_string(), |key| key.as_str().to_owned());
        validate_node(child, &format!("{path}/{name}"))?;
    }
    Ok(())
}

fn validate_table_schema(element: &Element, path: &str) -> Result<(), TreeError> {
    let crate::Props::List { style, columns, .. } = element.props() else {
        return Ok(());
    };
    if *style != crate::ListStyle::Table {
        if !columns.is_empty() {
            return Err(TreeError::InvalidTableSchema {
                path: path.to_owned(),
                reason: "columns require table presentation".to_owned(),
            });
        }
        return validate_non_table_rows(element.children(), path, *style);
    }
    let mut identifiers = HashSet::new();
    let mut active_sort_count = 0;
    for column in columns {
        if column.id.is_empty() || !identifiers.insert(column.id.as_str()) {
            return Err(TreeError::InvalidTableSchema {
                path: path.to_owned(),
                reason: format!("column identifier '{}' is empty or duplicated", column.id),
            });
        }
        if column.sort_direction.is_some() {
            active_sort_count += 1;
            if !column.sortable {
                return Err(TreeError::InvalidTableSchema {
                    path: path.to_owned(),
                    reason: format!("active sort column '{}' is not sortable", column.id),
                });
            }
        }
    }
    if active_sort_count > 1 {
        return Err(TreeError::InvalidTableSchema {
            path: path.to_owned(),
            reason: "only one active sort column is supported".to_owned(),
        });
    }
    let expected = columns.len().saturating_sub(1);
    validate_table_rows(element.children(), path, expected)
}

fn validate_table_rows(rows: &[Element], path: &str, expected: usize) -> Result<(), TreeError> {
    for (index, child) in rows.iter().enumerate() {
        let row_path = format!("{path}/{index}");
        let crate::Props::ListRow { cells, .. } = child.props() else {
            continue;
        };
        if cells.len() != expected {
            return Err(TreeError::InvalidTableSchema {
                path: row_path.clone(),
                reason: format!(
                    "row provides {} secondary cells for {expected} secondary columns",
                    cells.len()
                ),
            });
        }
        let crate::Props::ListRow { role, .. } = child.props() else {
            continue;
        };
        if *role != crate::ListRowRole::Item {
            return Err(TreeError::InvalidTableSchema {
                path: row_path.clone(),
                reason: "table rows cannot declare section semantics".to_owned(),
            });
        }
        validate_table_rows(child.children(), &row_path, expected)?;
    }
    Ok(())
}

fn validate_non_table_rows(
    rows: &[Element],
    path: &str,
    style: crate::ListStyle,
) -> Result<(), TreeError> {
    for (index, child) in rows.iter().enumerate() {
        let row_path = format!("{path}/{index}");
        let crate::Props::ListRow {
            cells,
            role,
            expanded,
            ..
        } = child.props()
        else {
            continue;
        };
        if !cells.is_empty() {
            return Err(TreeError::InvalidTableSchema {
                path: row_path,
                reason: "secondary cells require table presentation".to_owned(),
            });
        }
        if style != crate::ListStyle::Source {
            if !child.children().is_empty() || *role != crate::ListRowRole::Item || *expanded {
                return Err(TreeError::InvalidListSchema {
                    path: row_path,
                    reason: "hierarchy, sections, and expansion require source presentation"
                        .to_owned(),
                });
            }
            continue;
        }
        validate_non_table_rows(child.children(), &row_path, style)?;
    }
    Ok(())
}

fn mount_subtree<B: NativeBackend>(
    backend: &mut B,
    mut element: Element,
    stats: &mut RenderStats,
) -> Result<MountedNode<B::Handle>, B::Error> {
    let children = element.take_children();
    let events = EventBindings::new(&element.handlers);
    let handle = backend.create(&element, events.clone())?;
    stats.created += 1;

    let mut mounted_children = Vec::with_capacity(children.len());
    for (index, child) in children.into_iter().enumerate() {
        let mounted = mount_subtree(backend, child, stats)?;
        backend.insert_child(&handle, &mounted.handle, index)?;
        mounted_children.push(mounted);
    }
    Ok(MountedNode {
        handle,
        descriptor: element,
        events,
        children: mounted_children,
    })
}

fn reconcile_node<B: NativeBackend>(
    backend: &mut B,
    parent: &B::Handle,
    index: usize,
    mut current: MountedNode<B::Handle>,
    mut next: Element,
    stats: &mut RenderStats,
) -> Result<MountedNode<B::Handle>, B::Error> {
    if !compatible(&current.descriptor, &next) {
        let replacement = mount_subtree(backend, next, stats)?;
        backend.remove_child(parent, &current.handle, index)?;
        backend.insert_child(parent, &replacement.handle, index)?;
        destroy_subtree(backend, current, stats)?;
        stats.replaced += 1;
        return Ok(replacement);
    }

    stats.reused += 1;
    if let Some(patch) = PropertyPatch::between(current.descriptor.props(), next.props()) {
        backend.apply(&current.handle, &patch)?;
        stats.patched += 1;
    }
    current.events.replace(&next.handlers);

    let next_children = next.take_children();
    reconcile_children(
        backend,
        &current.handle,
        &mut current.children,
        next_children,
        stats,
    )?;
    current.descriptor = next;
    Ok(current)
}

fn reconcile_children<B: NativeBackend>(
    backend: &mut B,
    parent: &B::Handle,
    current: &mut Vec<MountedNode<B::Handle>>,
    next: Vec<Element>,
    stats: &mut RenderStats,
) -> Result<(), B::Error> {
    let next_len = next.len();
    for (new_index, next_child) in next.into_iter().enumerate() {
        match candidate(current, new_index, &next_child) {
            Some(old_index) => {
                if old_index != new_index {
                    let handle = current[old_index].handle.clone();
                    backend.move_child(parent, &handle, old_index, new_index)?;
                    let moved = current.remove(old_index);
                    current.insert(new_index, moved);
                    stats.moved += 1;
                }
                let previous = current.remove(new_index);
                let updated =
                    reconcile_node(backend, parent, new_index, previous, next_child, stats)?;
                current.insert(new_index, updated);
            }
            None => {
                let mounted = mount_subtree(backend, next_child, stats)?;
                backend.insert_child(parent, &mounted.handle, new_index)?;
                current.insert(new_index, mounted);
            }
        }
    }
    while current.len() > next_len {
        let removed = current.remove(next_len);
        backend.remove_child(parent, &removed.handle, next_len)?;
        destroy_subtree(backend, removed, stats)?;
    }
    Ok(())
}

fn candidate<H>(current: &[MountedNode<H>], start: usize, next: &Element) -> Option<usize> {
    if let Some(key) = next.key() {
        return current
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(index, node)| (node.descriptor.key() == Some(key)).then_some(index));
    }
    if current
        .get(start)
        .is_some_and(|node| node.descriptor.key().is_none())
    {
        return Some(start);
    }
    current
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, node)| node.descriptor.key().is_none().then_some(index))
}

fn compatible(old: &Element, new: &Element) -> bool {
    if old.kind() != new.kind() || old.key() != new.key() {
        return false;
    }
    match (old.props(), new.props()) {
        (
            crate::Props::List {
                style: old_style, ..
            },
            crate::Props::List {
                style: new_style, ..
            },
        ) => list_native_class(*old_style) == list_native_class(*new_style),
        _ => true,
    }
}

fn list_native_class(style: crate::ListStyle) -> u8 {
    match style {
        crate::ListStyle::Source => 0,
        crate::ListStyle::Table => 1,
        crate::ListStyle::Content | crate::ListStyle::Plain => 2,
    }
}

fn destroy_subtree<B: NativeBackend>(
    backend: &mut B,
    node: MountedNode<B::Handle>,
    stats: &mut RenderStats,
) -> Result<(), B::Error> {
    for child in node.children {
        destroy_subtree(backend, child, stats)?;
    }
    backend.destroy(&node.handle)?;
    stats.removed += 1;
    Ok(())
}
