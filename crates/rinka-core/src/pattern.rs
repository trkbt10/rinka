//! Shared vocabulary for mounting standard desktop UI patterns.

/// Semantic region occupied by one child of a standard UI pattern.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PatternRegion {
    /// Primary navigation choices, normally at the leading edge.
    NavigationSidebar,
    /// Main application content.
    Content,
    /// Secondary controls or details for the current selection.
    Inspector,
}

/// Platform-neutral desktop UI pattern routed to each toolkit's standard UI.
///
/// A pattern defines intent and ordered regions, not a particular native
/// widget. Adapters may use one standard container or compose several standard
/// containers when their toolkit does not expose the complete pattern.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UiPattern {
    /// Navigation sidebar followed by primary content.
    NavigationSplit {
        /// Whether the navigation sidebar can be hidden or collapsed.
        sidebar_collapsible: bool,
    },
    /// Primary content followed by an inspector or utility pane.
    UtilitySplit {
        /// Whether the inspector can be hidden or collapsed.
        inspector_collapsible: bool,
    },
    /// Navigation sidebar, primary content, and inspector.
    NavigationWorkspace {
        /// Whether the navigation sidebar can be hidden or collapsed.
        sidebar_collapsible: bool,
        /// Whether the inspector can be hidden or collapsed.
        inspector_collapsible: bool,
    },
}

const NAVIGATION_SPLIT_REGIONS: [PatternRegion; 2] =
    [PatternRegion::NavigationSidebar, PatternRegion::Content];
const UTILITY_SPLIT_REGIONS: [PatternRegion; 2] =
    [PatternRegion::Content, PatternRegion::Inspector];
const WORKSPACE_REGIONS: [PatternRegion; 3] = [
    PatternRegion::NavigationSidebar,
    PatternRegion::Content,
    PatternRegion::Inspector,
];

impl UiPattern {
    /// Returns the ordered semantic regions required by this pattern.
    pub const fn regions(self) -> &'static [PatternRegion] {
        match self {
            Self::NavigationSplit { .. } => &NAVIGATION_SPLIT_REGIONS,
            Self::UtilitySplit { .. } => &UTILITY_SPLIT_REGIONS,
            Self::NavigationWorkspace { .. } => &WORKSPACE_REGIONS,
        }
    }

    /// Returns the auxiliary region of a two-region pattern.
    pub const fn auxiliary_region(self) -> Option<PatternRegion> {
        match self {
            Self::NavigationSplit { .. } => Some(PatternRegion::NavigationSidebar),
            Self::UtilitySplit { .. } => Some(PatternRegion::Inspector),
            Self::NavigationWorkspace { .. } => None,
        }
    }

    /// Returns whether the named region belongs to this pattern and collapses.
    pub const fn region_is_collapsible(self, region: PatternRegion) -> bool {
        let (sidebar, inspector) = match self {
            Self::NavigationSplit {
                sidebar_collapsible,
            } => (sidebar_collapsible, false),
            Self::UtilitySplit {
                inspector_collapsible,
            } => (false, inspector_collapsible),
            Self::NavigationWorkspace {
                sidebar_collapsible,
                inspector_collapsible,
            } => (sidebar_collapsible, inspector_collapsible),
        };
        match region {
            PatternRegion::NavigationSidebar => sidebar,
            PatternRegion::Content => false,
            PatternRegion::Inspector => inspector,
        }
    }
}
