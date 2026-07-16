//! Keyed reconciliation into retained native objects.

use crate::accelerator::AcceleratorBindings;
use crate::validation::{TreeError, validate_tree};
use crate::{Element, EventBindings, NativeBackend, PropertyPatch};
use std::error::Error;
use std::fmt;

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
    accelerators: AcceleratorBindings,
    last_stats: RenderStats,
}

impl<B: NativeBackend> Renderer<B> {
    /// Creates an empty renderer over a persistent native root.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            mounted: None,
            accelerators: AcceleratorBindings::default(),
            last_stats: RenderStats::default(),
        }
    }

    /// Validates and reconciles a complete next tree.
    pub fn render(&mut self, mut next: Element) -> Result<RenderStats, RenderError<B::Error>> {
        validate_tree(&next).map_err(RenderError::Tree)?;
        validate_backend(&self.backend, &next).map_err(RenderError::Backend)?;
        let accelerators = next.take_accelerators();

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
        self.accelerators.replace(&accelerators);
        self.last_stats = stats;
        Ok(stats)
    }

    /// Returns the stable accelerator table owned by this renderer.
    ///
    /// A platform host connects its native key source to this value once;
    /// every successful render replaces the entries in place.
    pub fn accelerator_bindings(&self) -> &AcceleratorBindings {
        &self.accelerators
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
                pattern: old_pattern,
                ..
            },
            crate::Props::List {
                pattern: new_pattern,
                ..
            },
        ) => list_native_class(*old_pattern) == list_native_class(*new_pattern),
        _ => true,
    }
}

fn list_native_class(pattern: crate::CollectionPattern) -> u8 {
    if pattern.supports_hierarchy() {
        0
    } else if pattern.presents_columns() {
        1
    } else {
        2
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
