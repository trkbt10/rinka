//! Locators and mounted-tree queries shared by every driver.
//!
//! Finding happens against the mounted element tree — the descriptors the
//! reconciler actually realized — matching either the declarative key or
//! [`rinka_core::Props::accessibility_name`], the same value the adapters
//! map onto each platform's native accessibility attribute. The walk is
//! in-process: no external accessibility API and therefore no
//! screen-reader/TCC permission is involved (external `AXUIElement` access
//! requires a TCC grant the consumer's CI cannot answer; the recorded
//! justification lives in the consumer-test-harness report).

use rinka_core::MountedNode;
use std::fmt;

/// How a driver locates one mounted element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Locator {
    /// Match the declarative sibling key ([`rinka_core::Element::with_key`]).
    Key(String),
    /// Match the accessibility name derived from the element's properties.
    Label(String),
}

impl fmt::Display for Locator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(key) => write!(formatter, "key '{key}'"),
            Self::Label(label) => write!(formatter, "accessibility label '{label}'"),
        }
    }
}

/// Finds the first mounted node matching the locator, depth first.
pub fn find_node<'a, H>(node: &'a MountedNode<H>, locator: &Locator) -> Option<&'a MountedNode<H>> {
    let matched = match locator {
        Locator::Key(key) => node
            .element()
            .key()
            .is_some_and(|candidate| candidate.as_str() == key),
        Locator::Label(label) => node
            .element()
            .props()
            .accessibility_name()
            .is_some_and(|candidate| candidate == label),
    };
    if matched {
        return Some(node);
    }
    node.children()
        .iter()
        .find_map(|child| find_node(child, locator))
}

/// Renders the mounted element tree as an indented, line-per-element
/// snapshot: kind, declarative key, and accessibility name.
pub fn tree_snapshot<H>(node: &MountedNode<H>) -> String {
    let mut lines = String::new();
    append_snapshot(node, 0, &mut lines);
    lines
}

fn append_snapshot<H>(node: &MountedNode<H>, depth: usize, lines: &mut String) {
    use fmt::Write as _;
    let indent = "  ".repeat(depth);
    let kind = node.element().kind();
    let key = node
        .element()
        .key()
        .map_or_else(String::new, |key| format!(" key={}", key.as_str()));
    let name = node
        .element()
        .props()
        .accessibility_name()
        .map_or_else(String::new, |name| format!(" name={name:?}"));
    let _ = writeln!(lines, "{indent}{kind:?}{key}{name}");
    for child in node.children() {
        append_snapshot(child, depth + 1, lines);
    }
}
