//! Native host mutation contract and property snapshots.

use crate::{Element, Props};
use std::fmt;

/// Complete next [`Props`] snapshot for an existing native object.
///
/// This avoids duplicating every property variant in a mutation hierarchy.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyPatch {
    next: Props,
}

impl PropertyPatch {
    pub(crate) fn between(old: &Props, new: &Props) -> Option<Self> {
        (old != new).then(|| Self { next: new.clone() })
    }

    /// Returns the complete semantic state requested by this update.
    pub fn props(&self) -> &Props {
        &self.next
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

#[cfg(test)]
mod tests {
    use super::PropertyPatch;
    use crate::{ListRowRole, Props, Symbol};

    #[test]
    fn a_patch_exposes_the_next_props_snapshot() {
        let current = Props::ListRow {
            title: "Before".to_owned(),
            subtitle: None,
            cells: vec!["1 KB".to_owned()],
            role: ListRowRole::Item,
            expanded: false,
            symbol: Some(Symbol::File),
            selected: false,
            disclosure: false,
            accessibility_label: "Before, 1 KB".to_owned(),
        };
        let next = Props::ListRow {
            title: "After".to_owned(),
            subtitle: Some("Changed".to_owned()),
            cells: vec!["2 KB".to_owned()],
            role: ListRowRole::Item,
            expanded: true,
            symbol: Some(Symbol::Code),
            selected: true,
            disclosure: true,
            accessibility_label: "After, Changed, 2 KB".to_owned(),
        };

        let patch = PropertyPatch::between(&current, &next).expect("changed properties");
        assert_eq!(patch.props(), &next);
    }
}
