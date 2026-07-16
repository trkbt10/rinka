//! Native host mutation contract and property snapshots.

use crate::menu::ContextMenu;
use crate::{Element, Props};
use std::fmt;

/// Complete next semantic snapshot for an existing native object.
///
/// The patch carries the full next [`Props`] and the full next context-menu
/// model instead of duplicating every property variant in a mutation
/// hierarchy. Menu comparison uses the model's declarative equality, which
/// intentionally ignores activation handlers: handlers reach the adapter
/// through the stable event binding on every render regardless of patching.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyPatch {
    next: Props,
    next_context_menu: Option<ContextMenu>,
}

impl PropertyPatch {
    pub(crate) fn between(old: &Element, new: &Element) -> Option<Self> {
        (old.props() != new.props() || old.context_menu_model() != new.context_menu_model()).then(
            || Self {
                next: new.props().clone(),
                next_context_menu: new.context_menu_model().cloned(),
            },
        )
    }

    /// Returns the complete semantic state requested by this update.
    pub fn props(&self) -> &Props {
        &self.next
    }

    /// Returns the complete context-menu model requested by this update.
    ///
    /// `None` means the element carries no context menu after this update.
    pub fn context_menu(&self) -> Option<&ContextMenu> {
        self.next_context_menu.as_ref()
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

    /// Measures the native monospace font used by canvas glyph runs.
    ///
    /// `font_size` is in logical points. Returns [`None`] when the adapter
    /// does not implement canvas text measurement; the platform never
    /// substitutes fabricated metrics.
    fn monospace_metrics(&self, font_size: f64) -> Option<crate::MonospaceMetrics> {
        let _ = font_size;
        None
    }

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
    use crate::{MenuEntry, MenuItem, label};

    #[test]
    fn a_patch_exposes_the_next_props_snapshot() {
        let current = label("Before");
        let next = label("After");

        let patch = PropertyPatch::between(&current, &next).expect("changed properties");
        assert_eq!(patch.props(), next.props());
        assert!(patch.context_menu().is_none());
    }

    #[test]
    fn a_menu_only_change_is_a_patch_and_handler_changes_are_not() {
        let plain = label("File");
        let with_menu =
            || label("File").context_menu([MenuEntry::item(MenuItem::new("open", "Open", || {}))]);

        let patch = PropertyPatch::between(&plain, &with_menu()).expect("menu attachment");
        assert!(patch.context_menu().is_some());

        let removal = PropertyPatch::between(&with_menu(), &plain).expect("menu removal");
        assert!(removal.context_menu().is_none());

        assert!(PropertyPatch::between(&with_menu(), &with_menu()).is_none());
    }
}
