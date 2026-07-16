//! Declarative dock model: tabbed documents over user-rearrangeable splits.
//!
//! The dock is a fully controlled description, like every other element: a
//! tree of splits whose leaves are tab groups, expressed in semantic values
//! only. Native gestures never mutate the layout directly — they surface as
//! [`DockEvent`] requests through the element's stable event binding, the
//! consumer applies them to its own [`DockLayout`] (the mutation helpers
//! here implement the standard dock semantics, including
//! close-last-collapses), and the next render reconciles the change onto the
//! retained native tree.
//!
//! Tab content stays with the consumer: the dock element's children are the
//! content subtrees, one per tab, keyed by tab id. The layout carries only
//! tab identities and tab-chrome metadata.

use crate::menu::ContextMenu;
use crate::semantics::Axis;
use std::collections::HashSet;
use std::fmt::Write as _;

/// One document tab's identity and chrome metadata.
///
/// The tab id is the stable identity everything correlates on: the dock
/// element's content child key, [`DockEvent`] payloads, and the consumer's
/// own bookkeeping (for Overshell, worker events per document).
#[derive(Clone, Debug, PartialEq)]
pub struct DockTab {
    /// Stable identity, unique across the whole layout.
    pub id: String,
    /// Visible tab title.
    pub title: String,
    /// Whether the tab shows the unsaved-changes indicator.
    pub dirty: bool,
    /// Whether the tab offers a close affordance.
    pub closeable: bool,
}

impl DockTab {
    /// Creates a clean, closeable tab.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            dirty: false,
            closeable: true,
        }
    }

    /// Sets the unsaved-changes indicator.
    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }

    /// Sets whether the tab offers a close affordance.
    pub fn closeable(mut self, closeable: bool) -> Self {
        self.closeable = closeable;
        self
    }
}

/// One tab group: an ordered tab strip over a shared content area.
#[derive(Clone, Debug, PartialEq)]
pub struct DockGroup {
    /// Stable identity, unique across the whole layout.
    pub id: String,
    /// Tabs in strip order.
    pub tabs: Vec<DockTab>,
    /// Id of the visible tab; empty exactly when the group has no tabs
    /// (which is valid only for the root group).
    pub active: String,
}

impl DockGroup {
    /// Creates a group with the given tabs and active tab id.
    pub fn new(
        id: impl Into<String>,
        tabs: impl IntoIterator<Item = DockTab>,
        active: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            tabs: tabs.into_iter().collect(),
            active: active.into(),
        }
    }

    /// Creates the empty group a dock shows when no document is open.
    pub fn empty(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            tabs: Vec::new(),
            active: String::new(),
        }
    }
}

/// One weighted child of a split.
#[derive(Clone, Debug, PartialEq)]
pub struct DockSplitItem {
    /// Relative share of the split's extent; normalized against the sum of
    /// the sibling weights, so any positive finite scale works.
    pub weight: f64,
    /// The child subtree.
    pub node: DockNode,
}

/// A user-rearrangeable split of the dock area.
#[derive(Clone, Debug, PartialEq)]
pub struct DockSplit {
    /// Layout direction of the children: [`Axis::Horizontal`] places them
    /// side by side, [`Axis::Vertical`] stacks them top to bottom.
    pub axis: Axis,
    /// Weighted children in layout order; a valid split has at least two.
    pub items: Vec<DockSplitItem>,
}

/// One node of the dock tree.
#[derive(Clone, Debug, PartialEq)]
pub enum DockNode {
    /// A recursive split.
    Split(DockSplit),
    /// A leaf tab group.
    Group(DockGroup),
}

/// The complete declarative dock layout.
#[derive(Clone, Debug, PartialEq)]
pub struct DockLayout {
    root: DockNode,
}

/// Edge of a tab group receiving a split-by-drop.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DockEdge {
    /// The leading (left in a left-to-right locale) edge.
    Leading,
    /// The trailing edge.
    Trailing,
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
}

impl DockEdge {
    /// Returns the split axis this edge produces.
    pub const fn axis(self) -> Axis {
        match self {
            Self::Leading | Self::Trailing => Axis::Horizontal,
            Self::Top | Self::Bottom => Axis::Vertical,
        }
    }

    /// Returns whether the new group lands before the target in layout order.
    pub const fn inserts_before(self) -> bool {
        matches!(self, Self::Leading | Self::Top)
    }
}

/// A dock operation requested by a native gesture.
///
/// Every variant is a request: the consumer applies it to its layout state
/// (or refuses — the close-veto round trip for dirty tabs) and re-renders.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DockEvent {
    /// A tab was chosen in its group's strip.
    SelectTab {
        /// Group owning the tab.
        group: String,
        /// Chosen tab id.
        tab: String,
    },
    /// A tab's close affordance was activated. Consumers veto by not
    /// removing the tab — for a dirty document, the idiomatic round trip
    /// presents a confirmation dialog and closes only on an explicit answer.
    CloseTab {
        /// Group owning the tab.
        group: String,
        /// Tab whose close was requested.
        tab: String,
    },
    /// A tab was dragged onto a strip position: a reorder when the groups
    /// match, a move across groups otherwise.
    MoveTab {
        /// Dragged tab id.
        tab: String,
        /// Group the tab came from.
        from_group: String,
        /// Group receiving the tab.
        to_group: String,
        /// Insertion position in the receiving strip at drop time.
        index: usize,
    },
    /// A tab was dropped on a group content edge, requesting a split.
    SplitGroup {
        /// Dragged tab id.
        tab: String,
        /// Group the tab came from.
        from_group: String,
        /// Group whose content area received the drop.
        target_group: String,
        /// The dropped edge, which determines the split axis and order.
        edge: DockEdge,
    },
}

impl DockLayout {
    /// Creates a layout from its root node.
    pub fn new(root: DockNode) -> Self {
        Self { root }
    }

    /// Creates a layout with one group filling the dock.
    pub fn single_group(group: DockGroup) -> Self {
        Self {
            root: DockNode::Group(group),
        }
    }

    /// Returns the root node.
    pub fn root(&self) -> &DockNode {
        &self.root
    }

    /// Returns every group in depth-first layout order.
    pub fn groups(&self) -> Vec<&DockGroup> {
        let mut groups = Vec::new();
        collect_groups(&self.root, &mut groups);
        groups
    }

    /// Returns every tab in depth-first layout order.
    pub fn tabs(&self) -> Vec<&DockTab> {
        self.groups()
            .into_iter()
            .flat_map(|group| group.tabs.iter())
            .collect()
    }

    /// Returns every tab id in depth-first layout order.
    pub fn tab_ids(&self) -> Vec<&str> {
        self.tabs().into_iter().map(|tab| tab.id.as_str()).collect()
    }

    /// Finds a group by id.
    pub fn find_group(&self, group_id: &str) -> Option<&DockGroup> {
        self.groups().into_iter().find(|group| group.id == group_id)
    }

    /// Finds the group containing a tab.
    pub fn group_of_tab(&self, tab_id: &str) -> Option<&DockGroup> {
        self.groups()
            .into_iter()
            .find(|group| group.tabs.iter().any(|tab| tab.id == tab_id))
    }

    /// Finds a tab by id.
    pub fn tab(&self, tab_id: &str) -> Option<&DockTab> {
        self.tabs().into_iter().find(|tab| tab.id == tab_id)
    }

    /// Returns whether a tab id exists in the layout.
    pub fn contains_tab(&self, tab_id: &str) -> bool {
        self.tab(tab_id).is_some()
    }

    /// Makes a tab the active tab of its group. Returns whether it applied.
    pub fn select_tab(&mut self, tab_id: &str) -> bool {
        match group_of_tab_mut(&mut self.root, tab_id) {
            Some(group) => {
                group.active = tab_id.to_owned();
                true
            }
            None => false,
        }
    }

    /// Sets a tab's unsaved-changes indicator. Returns whether it applied.
    pub fn set_dirty(&mut self, tab_id: &str, dirty: bool) -> bool {
        match tab_mut(&mut self.root, tab_id) {
            Some(tab) => {
                tab.dirty = dirty;
                true
            }
            None => false,
        }
    }

    /// Sets a tab's title. Returns whether it applied.
    pub fn set_title(&mut self, tab_id: &str, title: impl Into<String>) -> bool {
        match tab_mut(&mut self.root, tab_id) {
            Some(tab) => {
                tab.title = title.into();
                true
            }
            None => false,
        }
    }

    /// Removes a tab, activating its neighbor, and collapses the split when
    /// the group empties — the standard close semantics. The root group is
    /// kept (empty) when the last tab of the whole dock closes.
    pub fn close_tab(&mut self, tab_id: &str) -> Option<DockTab> {
        let group = group_of_tab_mut(&mut self.root, tab_id)?;
        let index = group
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
            .expect("the group containing the tab holds it");
        let removed = group.tabs.remove(index);
        if group.active == tab_id {
            group.active = group
                .tabs
                .get(index)
                .or_else(|| group.tabs.last())
                .map(|tab| tab.id.clone())
                .unwrap_or_default();
        }
        let fallback_group_id = group.id.clone();
        self.collapse_empty_groups(&fallback_group_id);
        Some(removed)
    }

    /// Moves a tab to a position in a group's strip: a reorder within the
    /// group or a transfer across groups. The moved tab becomes active in
    /// the receiving group and an emptied source group collapses. `index`
    /// is the insertion position observed at drop time (before removal).
    /// Returns whether it applied.
    pub fn move_tab(&mut self, tab_id: &str, to_group: &str, index: usize) -> bool {
        if self.find_group(to_group).is_none() {
            return false;
        }
        let Some(source) = group_of_tab_mut(&mut self.root, tab_id) else {
            return false;
        };
        let source_id = source.id.clone();
        let from_index = source
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
            .expect("the group containing the tab holds it");
        let tab = source.tabs.remove(from_index);
        if source.active == tab_id {
            source.active = source
                .tabs
                .get(from_index)
                .or_else(|| source.tabs.last())
                .map(|tab| tab.id.clone())
                .unwrap_or_default();
        }
        let mut index = index;
        if source_id == to_group && from_index < index {
            index -= 1;
        }
        let target = find_group_mut(&mut self.root, to_group)
            .expect("the target group was checked before removal");
        let index = index.min(target.tabs.len());
        target.active = tab.id.clone();
        target.tabs.insert(index, tab);
        self.collapse_empty_groups(&source_id);
        true
    }

    /// Splits the target group along an edge, moving an existing tab into a
    /// new group on that edge — the drop-on-edge semantics. When the target
    /// already sits in a split of the same axis the new group joins as a
    /// sibling (taking half the target's share); otherwise the target is
    /// replaced by a nested two-way split. An emptied source group
    /// collapses. Returns whether it applied; splitting a group by its own
    /// only tab is refused because it would leave an empty group behind.
    pub fn split_with_tab(
        &mut self,
        target_group: &str,
        edge: DockEdge,
        new_group_id: &str,
        tab_id: &str,
    ) -> bool {
        if new_group_id.is_empty()
            || self.find_group(new_group_id).is_some()
            || self.find_group(target_group).is_none()
        {
            return false;
        }
        let Some(source) = self.group_of_tab(tab_id) else {
            return false;
        };
        if source.id == target_group && source.tabs.len() == 1 {
            return false;
        }
        let source =
            group_of_tab_mut(&mut self.root, tab_id).expect("the source group was located above");
        let source_id = source.id.clone();
        let from_index = source
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
            .expect("the group containing the tab holds it");
        let tab = source.tabs.remove(from_index);
        if source.active == tab_id {
            source.active = source
                .tabs
                .get(from_index)
                .or_else(|| source.tabs.last())
                .map(|tab| tab.id.clone())
                .unwrap_or_default();
        }
        let mut new_group = Some(DockGroup {
            id: new_group_id.to_owned(),
            active: tab.id.clone(),
            tabs: vec![tab],
        });
        let inserted = split_at_group(&mut self.root, target_group, edge, &mut new_group);
        debug_assert!(inserted, "the target group was checked before removal");
        self.collapse_empty_groups(&source_id);
        true
    }

    /// Inserts a new tab into a group's strip and activates it. Returns
    /// whether it applied; a duplicate tab id or unknown group is refused.
    pub fn insert_tab(&mut self, group_id: &str, index: usize, tab: DockTab) -> bool {
        if tab.id.is_empty() || self.contains_tab(&tab.id) {
            return false;
        }
        let Some(group) = find_group_mut(&mut self.root, group_id) else {
            return false;
        };
        let index = index.min(group.tabs.len());
        group.active = tab.id.clone();
        group.tabs.insert(index, tab);
        true
    }

    /// Removes empty groups and dissolves degenerate splits, preserving the
    /// dock itself: when everything collapses the root becomes an empty
    /// group carrying `fallback_group_id`, so the retained native dock host
    /// keeps a stable group identity.
    fn collapse_empty_groups(&mut self, fallback_group_id: &str) {
        let root = std::mem::replace(&mut self.root, DockNode::Group(DockGroup::empty("")));
        self.root = normalized(root)
            .unwrap_or_else(|| DockNode::Group(DockGroup::empty(fallback_group_id.to_owned())));
    }

    /// Checks the structural invariants and returns the first violation.
    ///
    /// A valid layout has unique non-empty group ids, globally unique
    /// non-empty tab ids, an active id naming one of each group's tabs,
    /// splits of at least two children with positive finite weights, and no
    /// empty group anywhere except at the root.
    pub fn invalid_reason(&self) -> Option<String> {
        let mut group_ids = HashSet::new();
        let mut tab_ids = HashSet::new();
        validate_model(&self.root, true, &mut group_ids, &mut tab_ids).err()
    }

    /// Serializes the layout into a plain persistence string.
    ///
    /// The format is a versioned, self-contained text value (see
    /// [`Self::from_persisted`]); every field round-trips exactly,
    /// weights included.
    pub fn to_persisted(&self) -> String {
        let mut output = String::from(PERSIST_HEADER);
        write_persisted_node(&mut output, &self.root);
        output
    }

    /// Restores a layout serialized by [`Self::to_persisted`].
    ///
    /// The restored layout is validated before it is returned, so persisted
    /// state from disk can never smuggle an invalid tree past reconciliation.
    pub fn from_persisted(text: &str) -> Result<Self, String> {
        let body = text
            .strip_prefix(PERSIST_HEADER)
            .ok_or_else(|| "unrecognized dock persistence header".to_owned())?;
        let mut cursor = PersistCursor::new(body);
        let root = parse_persisted_node(&mut cursor)?;
        if !cursor.at_end() {
            return Err(cursor.error("trailing data after the dock layout"));
        }
        let layout = Self { root };
        if let Some(reason) = layout.invalid_reason() {
            return Err(format!("persisted dock layout is invalid: {reason}"));
        }
        Ok(layout)
    }
}

fn collect_groups<'node>(node: &'node DockNode, groups: &mut Vec<&'node DockGroup>) {
    match node {
        DockNode::Group(group) => groups.push(group),
        DockNode::Split(split) => {
            for item in &split.items {
                collect_groups(&item.node, groups);
            }
        }
    }
}

fn find_group_mut<'node>(
    node: &'node mut DockNode,
    group_id: &str,
) -> Option<&'node mut DockGroup> {
    match node {
        DockNode::Group(group) => (group.id == group_id).then_some(group),
        DockNode::Split(split) => split
            .items
            .iter_mut()
            .find_map(|item| find_group_mut(&mut item.node, group_id)),
    }
}

fn group_of_tab_mut<'node>(
    node: &'node mut DockNode,
    tab_id: &str,
) -> Option<&'node mut DockGroup> {
    match node {
        DockNode::Group(group) => group
            .tabs
            .iter()
            .any(|tab| tab.id == tab_id)
            .then_some(group),
        DockNode::Split(split) => split
            .items
            .iter_mut()
            .find_map(|item| group_of_tab_mut(&mut item.node, tab_id)),
    }
}

fn tab_mut<'node>(node: &'node mut DockNode, tab_id: &str) -> Option<&'node mut DockTab> {
    group_of_tab_mut(node, tab_id)
        .and_then(|group| group.tabs.iter_mut().find(|tab| tab.id == tab_id))
}

/// Removes empty groups and dissolves splits left with fewer than two
/// children; `None` means the whole subtree vanished.
fn normalized(node: DockNode) -> Option<DockNode> {
    match node {
        DockNode::Group(group) => {
            if group.tabs.is_empty() {
                None
            } else {
                Some(DockNode::Group(group))
            }
        }
        DockNode::Split(split) => {
            let items: Vec<DockSplitItem> = split
                .items
                .into_iter()
                .filter_map(|item| {
                    normalized(item.node).map(|node| DockSplitItem {
                        weight: item.weight,
                        node,
                    })
                })
                .collect();
            match items.len() {
                0 => None,
                1 => items.into_iter().next().map(|item| item.node),
                _ => Some(DockNode::Split(DockSplit {
                    axis: split.axis,
                    items,
                })),
            }
        }
    }
}

fn split_at_group(
    node: &mut DockNode,
    target_group: &str,
    edge: DockEdge,
    new_group: &mut Option<DockGroup>,
) -> bool {
    match node {
        DockNode::Group(group) => {
            if group.id != target_group {
                return false;
            }
            let added = new_group.take().expect("the new group is consumed once");
            let current = std::mem::replace(node, DockNode::Group(DockGroup::empty("")));
            let (first, second) = if edge.inserts_before() {
                (DockNode::Group(added), current)
            } else {
                (current, DockNode::Group(added))
            };
            *node = DockNode::Split(DockSplit {
                axis: edge.axis(),
                items: vec![
                    DockSplitItem {
                        weight: 1.0,
                        node: first,
                    },
                    DockSplitItem {
                        weight: 1.0,
                        node: second,
                    },
                ],
            });
            true
        }
        DockNode::Split(split) => {
            if split.axis == edge.axis()
                && let Some(position) = split.items.iter().position(
                    |item| matches!(&item.node, DockNode::Group(group) if group.id == target_group),
                )
            {
                let added = new_group.take().expect("the new group is consumed once");
                let shared = split.items[position].weight / 2.0;
                split.items[position].weight = shared;
                let insert_at = if edge.inserts_before() {
                    position
                } else {
                    position + 1
                };
                split.items.insert(
                    insert_at,
                    DockSplitItem {
                        weight: shared,
                        node: DockNode::Group(added),
                    },
                );
                return true;
            }
            split
                .items
                .iter_mut()
                .any(|item| split_at_group(&mut item.node, target_group, edge, new_group))
        }
    }
}

fn validate_model(
    node: &DockNode,
    is_root: bool,
    group_ids: &mut HashSet<String>,
    tab_ids: &mut HashSet<String>,
) -> Result<(), String> {
    match node {
        DockNode::Group(group) => {
            if group.id.is_empty() {
                return Err("a dock group has an empty id".to_owned());
            }
            if !group_ids.insert(group.id.clone()) {
                return Err(format!("dock group id '{}' is duplicated", group.id));
            }
            if group.tabs.is_empty() {
                if !is_root {
                    return Err(format!(
                        "dock group '{}' is empty; only the root group may be empty",
                        group.id
                    ));
                }
                if !group.active.is_empty() {
                    return Err(format!(
                        "empty dock group '{}' declares active tab '{}'",
                        group.id, group.active
                    ));
                }
                return Ok(());
            }
            for tab in &group.tabs {
                if tab.id.is_empty() {
                    return Err(format!(
                        "a tab in dock group '{}' has an empty id",
                        group.id
                    ));
                }
                if !tab_ids.insert(tab.id.clone()) {
                    return Err(format!("dock tab id '{}' is duplicated", tab.id));
                }
            }
            if !group.tabs.iter().any(|tab| tab.id == group.active) {
                return Err(format!(
                    "active tab '{}' is not a tab of dock group '{}'",
                    group.active, group.id
                ));
            }
            Ok(())
        }
        DockNode::Split(split) => {
            if split.items.len() < 2 {
                return Err(format!(
                    "a dock split needs at least two children, found {}",
                    split.items.len()
                ));
            }
            for item in &split.items {
                if !item.weight.is_finite() || item.weight <= 0.0 {
                    return Err(format!(
                        "dock split weight {} is not a positive finite value",
                        item.weight
                    ));
                }
                validate_model(&item.node, false, group_ids, tab_ids)?;
            }
            Ok(())
        }
    }
}

/// Version header of the plain persistence value.
const PERSIST_HEADER: &str = "rinka-dock-v1:";

fn write_persisted_string(output: &mut String, value: &str) {
    let _ = write!(output, "{}:{value}", value.len());
}

fn write_persisted_node(output: &mut String, node: &DockNode) {
    match node {
        DockNode::Group(group) => {
            output.push('G');
            write_persisted_string(output, &group.id);
            write_persisted_string(output, &group.active);
            let _ = write!(output, "{};", group.tabs.len());
            for tab in &group.tabs {
                write_persisted_string(output, &tab.id);
                write_persisted_string(output, &tab.title);
                output.push(if tab.dirty { '1' } else { '0' });
                output.push(if tab.closeable { '1' } else { '0' });
            }
        }
        DockNode::Split(split) => {
            output.push('S');
            output.push(match split.axis {
                Axis::Horizontal => 'h',
                Axis::Vertical => 'v',
            });
            let _ = write!(output, "{};", split.items.len());
            for item in &split.items {
                // `{:?}` on f64 is Rust's shortest exact round-trip form.
                let _ = write!(output, "{:?};", item.weight);
                write_persisted_node(output, &item.node);
            }
        }
    }
}

struct PersistCursor<'text> {
    text: &'text str,
    position: usize,
}

impl<'text> PersistCursor<'text> {
    fn new(text: &'text str) -> Self {
        Self { text, position: 0 }
    }

    fn error(&self, message: &str) -> String {
        format!(
            "{message} at byte {} of the dock persistence value",
            self.position
        )
    }

    fn at_end(&self) -> bool {
        self.position == self.text.len()
    }

    fn next_byte(&mut self) -> Result<u8, String> {
        let byte = *self
            .text
            .as_bytes()
            .get(self.position)
            .ok_or_else(|| self.error("unexpected end"))?;
        self.position += 1;
        Ok(byte)
    }

    fn read_until(&mut self, terminator: u8) -> Result<&'text str, String> {
        let rest = &self.text[self.position..];
        let end = rest
            .bytes()
            .position(|byte| byte == terminator)
            .ok_or_else(|| self.error("missing field terminator"))?;
        let value = &rest[..end];
        self.position += end + 1;
        Ok(value)
    }

    fn read_usize(&mut self, terminator: u8) -> Result<usize, String> {
        let start = self.position;
        self.read_until(terminator)?
            .parse::<usize>()
            .map_err(|_| format!("invalid count at byte {start} of the dock persistence value"))
    }

    fn read_string(&mut self) -> Result<String, String> {
        let length = self.read_usize(b':')?;
        let end = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.text.len())
            .ok_or_else(|| self.error("string length exceeds the value"))?;
        let value = self
            .text
            .get(self.position..end)
            .ok_or_else(|| self.error("string length splits a character"))?;
        self.position = end;
        Ok(value.to_owned())
    }

    fn read_f64(&mut self) -> Result<f64, String> {
        let start = self.position;
        self.read_until(b';')?
            .parse::<f64>()
            .map_err(|_| format!("invalid weight at byte {start} of the dock persistence value"))
    }

    fn read_bit(&mut self) -> Result<bool, String> {
        match self.next_byte()? {
            b'0' => Ok(false),
            b'1' => Ok(true),
            _ => Err(self.error("invalid flag")),
        }
    }
}

fn parse_persisted_node(cursor: &mut PersistCursor<'_>) -> Result<DockNode, String> {
    match cursor.next_byte()? {
        b'G' => {
            let id = cursor.read_string()?;
            let active = cursor.read_string()?;
            let count = cursor.read_usize(b';')?;
            let mut tabs = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                let tab_id = cursor.read_string()?;
                let title = cursor.read_string()?;
                let dirty = cursor.read_bit()?;
                let closeable = cursor.read_bit()?;
                tabs.push(DockTab {
                    id: tab_id,
                    title,
                    dirty,
                    closeable,
                });
            }
            Ok(DockNode::Group(DockGroup { id, tabs, active }))
        }
        b'S' => {
            let axis = match cursor.next_byte()? {
                b'h' => Axis::Horizontal,
                b'v' => Axis::Vertical,
                _ => return Err(cursor.error("invalid split axis")),
            };
            let count = cursor.read_usize(b';')?;
            let mut items = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                let weight = cursor.read_f64()?;
                let node = parse_persisted_node(cursor)?;
                items.push(DockSplitItem { weight, node });
            }
            Ok(DockNode::Split(DockSplit { axis, items }))
        }
        _ => Err(cursor.error("invalid node tag")),
    }
}

/// Declarative per-tab context menus attached to one dock element.
///
/// Menus ride with the element's event handlers — their items carry
/// activation closures that must stay current across renders — while their
/// comparable state participates in property patching like the element
/// context menu does. Menus are keyed by tab id and are deliberately not
/// part of [`DockLayout`]: the layout is persistable pure data, menus are
/// per-render UI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DockTabMenus {
    entries: Vec<(String, ContextMenu)>,
}

impl DockTabMenus {
    /// Adds or replaces the menu of one tab.
    pub(crate) fn insert(&mut self, tab_id: String, menu: ContextMenu) {
        if let Some(existing) = self.entries.iter_mut().find(|(id, _)| *id == tab_id) {
            existing.1 = menu;
        } else {
            self.entries.push((tab_id, menu));
        }
    }

    /// Returns the menu declared for a tab.
    pub fn menu_for(&self, tab_id: &str) -> Option<&ContextMenu> {
        self.entries
            .iter()
            .find_map(|(id, menu)| (id == tab_id).then_some(menu))
    }

    /// Returns the declared `(tab id, menu)` pairs in declaration order.
    pub fn entries(&self) -> &[(String, ContextMenu)] {
        &self.entries
    }

    /// Returns whether no tab declares a menu.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_group_layout() -> DockLayout {
        DockLayout::new(DockNode::Split(DockSplit {
            axis: Axis::Horizontal,
            items: vec![
                DockSplitItem {
                    weight: 2.0,
                    node: DockNode::Group(DockGroup::new(
                        "left",
                        [DockTab::new("a", "A"), DockTab::new("b", "B")],
                        "a",
                    )),
                },
                DockSplitItem {
                    weight: 1.0,
                    node: DockNode::Group(DockGroup::new(
                        "right",
                        [DockTab::new("c", "C").dirty(true)],
                        "c",
                    )),
                },
            ],
        }))
    }

    #[test]
    fn selection_and_chrome_updates_apply_by_tab_id() {
        let mut layout = two_group_layout();
        assert!(layout.select_tab("b"));
        assert_eq!(layout.find_group("left").unwrap().active, "b");
        assert!(layout.set_dirty("a", true));
        assert!(layout.set_title("a", "A*"));
        assert!(layout.tab("a").unwrap().dirty);
        assert_eq!(layout.tab("a").unwrap().title, "A*");
        assert!(!layout.select_tab("missing"));
    }

    #[test]
    fn closing_activates_the_neighbor_and_the_last_close_collapses_the_split() {
        let mut layout = two_group_layout();
        let removed = layout.close_tab("a").expect("tab a closes");
        assert_eq!(removed.id, "a");
        assert_eq!(layout.find_group("left").unwrap().active, "b");
        assert!(matches!(layout.root(), DockNode::Split(_)));

        layout.close_tab("c").expect("tab c closes");
        // The right group emptied: the split dissolves into the left group.
        assert!(matches!(
            layout.root(),
            DockNode::Group(group) if group.id == "left"
        ));

        layout.close_tab("b").expect("tab b closes");
        // The dock keeps one empty root group with a stable identity.
        assert!(matches!(
            layout.root(),
            DockNode::Group(group) if group.id == "left" && group.tabs.is_empty()
                && group.active.is_empty()
        ));
        assert_eq!(layout.invalid_reason(), None);
    }

    #[test]
    fn moving_reorders_within_a_group_with_drop_time_indexes() {
        let mut layout = DockLayout::single_group(DockGroup::new(
            "only",
            [
                DockTab::new("a", "A"),
                DockTab::new("b", "B"),
                DockTab::new("c", "C"),
            ],
            "a",
        ));
        // Drop "a" after "c": index 3 observed before removal.
        assert!(layout.move_tab("a", "only", 3));
        let group = layout.find_group("only").unwrap();
        assert_eq!(
            group
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "c", "a"]
        );
        assert_eq!(group.active, "a");
    }

    #[test]
    fn moving_across_groups_activates_the_tab_and_collapses_an_emptied_source() {
        let mut layout = two_group_layout();
        assert!(layout.move_tab("c", "left", 1));
        assert!(matches!(
            layout.root(),
            DockNode::Group(group) if group.id == "left"
        ));
        let group = layout.find_group("left").unwrap();
        assert_eq!(
            group
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "c", "b"]
        );
        assert_eq!(group.active, "c");
        assert!(!layout.move_tab("a", "missing", 0));
    }

    #[test]
    fn splitting_by_edge_creates_the_new_group_on_the_dropped_side() {
        let mut layout = DockLayout::single_group(DockGroup::new(
            "main",
            [DockTab::new("a", "A"), DockTab::new("b", "B")],
            "a",
        ));
        assert!(layout.split_with_tab("main", DockEdge::Trailing, "side", "b"));
        let DockNode::Split(split) = layout.root() else {
            panic!("split expected");
        };
        assert_eq!(split.axis, Axis::Horizontal);
        assert!(matches!(
            &split.items[0].node,
            DockNode::Group(group) if group.id == "main"
        ));
        assert!(matches!(
            &split.items[1].node,
            DockNode::Group(group) if group.id == "side" && group.active == "b"
        ));
        assert_eq!(layout.invalid_reason(), None);

        // Top edge: the new group lands before the target, on a new axis.
        assert!(layout.insert_tab("main", 1, DockTab::new("c", "C")));
        assert!(layout.split_with_tab("side", DockEdge::Top, "upper", "a"));
        assert_eq!(layout.groups().len(), 3);
        assert_eq!(layout.find_group("main").unwrap().active, "c");
        assert_eq!(layout.find_group("upper").unwrap().active, "a");
        assert_eq!(layout.invalid_reason(), None);
    }

    #[test]
    fn splitting_in_the_same_axis_joins_the_existing_split_as_a_sibling() {
        let mut layout = two_group_layout();
        assert!(layout.split_with_tab("left", DockEdge::Trailing, "middle", "c"));
        let DockNode::Split(split) = layout.root() else {
            panic!("split expected");
        };
        // "right" emptied and collapsed; "middle" joined the same-axis split
        // beside "left" with half its weight.
        assert_eq!(split.items.len(), 2);
        assert!(matches!(
            &split.items[0].node,
            DockNode::Group(group) if group.id == "left"
        ));
        assert!(matches!(
            &split.items[1].node,
            DockNode::Group(group) if group.id == "middle"
        ));
        assert!((split.items[0].weight - 1.0).abs() < f64::EPSILON);
        assert!((split.items[1].weight - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn splitting_a_group_by_its_own_only_tab_is_refused() {
        let mut layout =
            DockLayout::single_group(DockGroup::new("main", [DockTab::new("a", "A")], "a"));
        assert!(!layout.split_with_tab("main", DockEdge::Trailing, "side", "a"));
        assert!(matches!(layout.root(), DockNode::Group(_)));
    }

    #[test]
    fn inserting_a_tab_activates_it_and_refuses_duplicates() {
        let mut layout = two_group_layout();
        assert!(layout.insert_tab("right", 0, DockTab::new("d", "D")));
        assert_eq!(layout.find_group("right").unwrap().active, "d");
        assert!(!layout.insert_tab("right", 0, DockTab::new("a", "again")));
        assert!(!layout.insert_tab("missing", 0, DockTab::new("e", "E")));
    }

    #[test]
    fn validation_names_the_first_violation() {
        let empty_inner = DockLayout::new(DockNode::Split(DockSplit {
            axis: Axis::Vertical,
            items: vec![
                DockSplitItem {
                    weight: 1.0,
                    node: DockNode::Group(DockGroup::empty("void")),
                },
                DockSplitItem {
                    weight: 1.0,
                    node: DockNode::Group(DockGroup::new("solo", [DockTab::new("a", "A")], "a")),
                },
            ],
        }));
        assert!(empty_inner.invalid_reason().unwrap().contains("void"));

        let duplicate_tab = DockLayout::single_group(DockGroup::new(
            "main",
            [DockTab::new("a", "A"), DockTab::new("a", "A2")],
            "a",
        ));
        assert!(
            duplicate_tab
                .invalid_reason()
                .unwrap()
                .contains("duplicated")
        );

        let bad_active =
            DockLayout::single_group(DockGroup::new("main", [DockTab::new("a", "A")], "zzz"));
        assert!(bad_active.invalid_reason().unwrap().contains("active"));

        let short_split = DockLayout::new(DockNode::Split(DockSplit {
            axis: Axis::Horizontal,
            items: vec![DockSplitItem {
                weight: 1.0,
                node: DockNode::Group(DockGroup::new("main", [DockTab::new("a", "A")], "a")),
            }],
        }));
        assert!(
            short_split
                .invalid_reason()
                .unwrap()
                .contains("two children")
        );

        let bad_weight = DockLayout::new(DockNode::Split(DockSplit {
            axis: Axis::Horizontal,
            items: vec![
                DockSplitItem {
                    weight: 0.0,
                    node: DockNode::Group(DockGroup::new("main", [DockTab::new("a", "A")], "a")),
                },
                DockSplitItem {
                    weight: 1.0,
                    node: DockNode::Group(DockGroup::new("side", [DockTab::new("b", "B")], "b")),
                },
            ],
        }));
        assert!(bad_weight.invalid_reason().unwrap().contains("weight"));
    }

    #[test]
    fn persistence_round_trips_the_identical_model() {
        let mut layout = two_group_layout();
        layout.split_with_tab("right", DockEdge::Bottom, "lower", "b");
        layout.set_title("a", "weird:title;with G3:delims\n and 日本語");
        assert_eq!(layout.invalid_reason(), None);

        let persisted = layout.to_persisted();
        let restored = DockLayout::from_persisted(&persisted).expect("round trip parses");
        assert_eq!(restored, layout);
        assert_eq!(restored.to_persisted(), persisted);
    }

    #[test]
    fn persistence_rejects_foreign_and_corrupt_values() {
        assert!(DockLayout::from_persisted("not-a-dock").is_err());
        assert!(DockLayout::from_persisted("rinka-dock-v1:").is_err());
        assert!(DockLayout::from_persisted("rinka-dock-v1:X").is_err());
        // Structurally parseable but semantically invalid: duplicate tab id.
        let mut layout =
            DockLayout::single_group(DockGroup::new("main", [DockTab::new("a", "A")], "a"));
        layout.insert_tab("main", 1, DockTab::new("b", "B"));
        let good = layout.to_persisted();
        let corrupted = good.replace("1:bB", "1:aB");
        if corrupted != good {
            assert!(DockLayout::from_persisted(&corrupted).is_err());
        }
        // Truncation is an error, not a partial layout.
        assert!(DockLayout::from_persisted(&good[..good.len() - 2]).is_err());
    }

    #[test]
    fn tab_menus_replace_by_tab_id_and_compare_declaratively() {
        use crate::menu::{MenuEntry, MenuItem};
        let mut menus = DockTabMenus::default();
        menus.insert(
            "a".to_owned(),
            ContextMenu::new([MenuEntry::item(MenuItem::new("close", "Close", || {}))]),
        );
        menus.insert(
            "a".to_owned(),
            ContextMenu::new([MenuEntry::item(MenuItem::new("other", "Other", || {}))]),
        );
        assert_eq!(menus.entries().len(), 1);
        assert!(menus.menu_for("a").unwrap().find_item("other").is_some());
        assert!(menus.menu_for("b").is_none());

        let mut same = DockTabMenus::default();
        same.insert(
            "a".to_owned(),
            ContextMenu::new([MenuEntry::item(MenuItem::new("other", "Other", || {
                panic!("handlers are outside equality")
            }))]),
        );
        assert_eq!(menus, same);
    }
}
