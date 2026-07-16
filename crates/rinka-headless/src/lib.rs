//! Deterministic retained-tree adapter used by tests and surface extraction.

use rinka_core::{
    ContextMenu, Element, EventBindings, MonospaceMetrics, NativeBackend, PropertyPatch, Props,
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

#[derive(Clone, Debug)]
struct Node {
    key: Option<String>,
    props: Props,
    context_menu: Option<ContextMenu>,
    events: EventBindings,
    children: Vec<Handle>,
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
        self.nodes.insert(
            handle,
            Node {
                key: element.key().map(|key| key.as_str().to_owned()),
                props: props.clone(),
                context_menu: element.context_menu_model().cloned(),
                events,
                children: Vec::new(),
            },
        );
        self.operations.push(Operation::Create { handle, props });
        Ok(handle)
    }

    fn apply(&mut self, handle: &Self::Handle, patch: &PropertyPatch) -> Result<(), Self::Error> {
        let node = self.node_mut(handle)?;
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
