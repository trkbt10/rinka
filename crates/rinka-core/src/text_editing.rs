//! Multi-line text editing contracts shared by every native adapter.
//!
//! # The controlled-text protocol
//!
//! A text area is reconciled **by revision, not by value**. The native view
//! owns the live buffer (typing, IME composition, undo all stay native); the
//! declarative tree carries a [`TextContent`] whose [`TextRevision`] names the
//! document state the application knows about. Reconciliation never compares
//! document bytes and never re-ships the document into the native view unless
//! the revision says the application changed it programmatically.
//!
//! The revision is a composite with one writer per field:
//!
//! - [`TextRevision::set`] is written **only by the application**. Bump it
//!   (via [`TextRevision::next_set`]) for every programmatic content change —
//!   loading a file, a programmatic insert, a formatting pass.
//! - [`TextRevision::edit`] is written **only by the native adapter**. Every
//!   native user edit bumps it and reports the delta through a [`TextChange`]
//!   event; the application applies the delta to its own copy and echoes the
//!   event's revision back on the next render.
//!
//! Because the echo carries the revision the adapter itself assigned, the
//! adapter recognizes it (see [`TextContent::sync_action`]) and leaves the
//! native buffer untouched: in-flight typing and IME composition are never
//! clobbered by a re-render, and a keystroke never round-trips the document.
//!
//! # Index space
//!
//! Every range in this module is measured in **Unicode scalar values** (Rust
//! `char` indices). Adapters translate to their native index space (UTF-16
//! code units on macOS and Windows, byte offsets on GTK).
//!
//! # Highlight vocabulary
//!
//! Syntax highlighting is semantic: the application computes
//! [`HighlightSpan`] ranges (it owns the parser) and tags each with a
//! [`HighlightRole`]; each platform adapter resolves the role to a native
//! palette color. The core carries no color values, per the design contract
//! that common code expresses meaning, not platform pixels.

use std::sync::Arc;

/// Composite identity of a text area's document state.
///
/// Ordering is lexicographic: a programmatic `set` supersedes any number of
/// native `edit` increments recorded against the previous set.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextRevision {
    /// Application-assigned counter, bumped once per programmatic content
    /// change. Only the application writes this field.
    pub set: u64,
    /// Adapter-assigned counter, bumped once per native user edit and reset
    /// to zero by every programmatic change. The application only echoes it.
    pub edit: u64,
}

impl TextRevision {
    /// Creates the revision of a freshly set document.
    pub const fn new(set: u64) -> Self {
        Self { set, edit: 0 }
    }

    /// Returns the revision of the next programmatic content change.
    pub const fn next_set(self) -> Self {
        Self {
            set: self.set + 1,
            edit: 0,
        }
    }

    /// Returns the revision after one further native user edit.
    pub const fn next_edit(self) -> Self {
        Self {
            set: self.set,
            edit: self.edit + 1,
        }
    }
}

/// Half-open range of Unicode scalar values `start..end` in a document.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextRange {
    /// First character index covered by the range.
    pub start: usize,
    /// First character index past the range.
    pub end: usize,
}

impl TextRange {
    /// Creates a half-open character range.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Returns the number of characters covered.
    pub const fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Returns whether the range covers no characters.
    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// One replacement in a text buffer.
///
/// A sequence of edits is **sequential**: each edit's range addresses the
/// buffer as left by the previous edit, exactly as native views report them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    /// Replaced character range (empty for a pure insertion).
    pub range: TextRange,
    /// Text inserted in place of the range (empty for a pure deletion).
    pub replacement: String,
}

impl TextEdit {
    /// Creates one replacement edit.
    pub fn new(range: TextRange, replacement: impl Into<String>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }

    /// Applies sequential edits to a document, or returns [`None`] when an
    /// edit addresses characters outside the buffer.
    pub fn apply_all(text: &str, edits: &[Self]) -> Option<String> {
        let mut document = text.to_owned();
        for edit in edits {
            let byte_range = char_range_to_byte_range(&document, edit.range)?;
            document.replace_range(byte_range, &edit.replacement);
        }
        Some(document)
    }
}

/// Converts a character range into the byte range of the same text.
///
/// Returns [`None`] when the range addresses characters past the end.
pub fn char_range_to_byte_range(text: &str, range: TextRange) -> Option<std::ops::Range<usize>> {
    if range.end < range.start {
        return None;
    }
    let mut start_byte = None;
    let mut end_byte = None;
    for (character_index, (byte_index, _)) in text.char_indices().enumerate() {
        if character_index == range.start {
            start_byte = Some(byte_index);
        }
        if character_index == range.end {
            end_byte = Some(byte_index);
            break;
        }
    }
    let char_len_reached = |target: usize| target == text.chars().count();
    let start = match start_byte {
        Some(byte) => byte,
        None if char_len_reached(range.start) => text.len(),
        None => return None,
    };
    let end = match end_byte {
        Some(byte) => byte,
        None if char_len_reached(range.end) => text.len(),
        None => return None,
    };
    Some(start..end)
}

/// Cursor or selection in a document, in character indices.
///
/// `anchor` is where the selection began and `head` is the moving end (the
/// caret). A caret has `anchor == head`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextSelection {
    /// Fixed end of the selection.
    pub anchor: usize,
    /// Moving end of the selection: the caret position.
    pub head: usize,
}

impl TextSelection {
    /// Creates a caret with no extent.
    pub const fn caret(position: usize) -> Self {
        Self {
            anchor: position,
            head: position,
        }
    }

    /// Creates a selection from anchor to head.
    pub const fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// Returns whether the selection is a caret.
    pub const fn is_caret(&self) -> bool {
        self.anchor == self.head
    }

    /// Returns the selection normalized into an ascending character range.
    pub const fn range(&self) -> TextRange {
        if self.anchor <= self.head {
            TextRange::new(self.anchor, self.head)
        } else {
            TextRange::new(self.head, self.anchor)
        }
    }
}

/// A native text edit reported by a platform text view.
///
/// The event carries only the delta — a single-character edit never re-ships
/// the document. The application applies `edits` to its own copy (for
/// example through [`TextEdit::apply_all`]) and echoes `revision` in the next
/// render's [`TextContent`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextChange {
    /// Document revision the edits apply to: the adapter's state before them.
    pub base_revision: TextRevision,
    /// Document revision after the edits; echo it in the next render.
    pub revision: TextRevision,
    /// Sequential character-range replacements.
    pub edits: Vec<TextEdit>,
    /// Whether an IME composition was active when the change was recorded.
    /// Composition deltas mirror uncommitted preedit text; applications may
    /// defer expensive work (such as re-highlighting) until this turns false.
    pub composing: bool,
}

/// Revisioned document content declared by a text area.
///
/// Reconciliation identifies content by [`TextRevision`], never by comparing
/// text: within one mounted element, a producer must supply a new revision
/// whenever it supplies different text, and two contents carrying the same
/// revision and length are treated as the same document.
#[derive(Clone)]
pub struct TextContent {
    text: Arc<str>,
    char_len: usize,
    revision: TextRevision,
    base_revision: Option<TextRevision>,
    edits: Arc<[TextEdit]>,
}

impl TextContent {
    /// Wraps a document under an application-assigned revision.
    pub fn new(text: impl Into<Arc<str>>, revision: TextRevision) -> Self {
        let text = text.into();
        let char_len = text.chars().count();
        Self {
            text,
            char_len,
            revision,
            base_revision: None,
            edits: Arc::from([]),
        }
    }

    /// Declares that this content was produced from `base` by `edits`,
    /// letting adapters apply the change incrementally — preserving native
    /// scroll, selection, and view state — instead of replacing the buffer.
    pub fn with_edits(mut self, base: TextRevision, edits: impl Into<Arc<[TextEdit]>>) -> Self {
        self.base_revision = Some(base);
        self.edits = edits.into();
        self
    }

    /// Returns the full document text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the document text's shared allocation.
    pub fn shared_text(&self) -> Arc<str> {
        Arc::clone(&self.text)
    }

    /// Returns the document length in Unicode scalar values.
    pub const fn char_len(&self) -> usize {
        self.char_len
    }

    /// Returns the declared document revision.
    pub const fn revision(&self) -> TextRevision {
        self.revision
    }

    /// Returns the revision this content declares its edits against.
    pub const fn base_revision(&self) -> Option<TextRevision> {
        self.base_revision
    }

    /// Returns the declared edits transforming the base into this content.
    pub fn edits(&self) -> &[TextEdit] {
        &self.edits
    }

    /// Decides how an adapter synchronizes its native buffer, currently at
    /// `native`, with this declared content.
    ///
    /// - [`TextSyncAction::Keep`]: the declaration is an echo (or trails
    ///   pending native edits within the same `set`); the native buffer
    ///   already contains at least this content. The adapter must not touch
    ///   the buffer — this is the guard that protects in-flight typing and
    ///   IME composition.
    /// - [`TextSyncAction::ApplyEdits`]: a programmatic change declared
    ///   against exactly the adapter's revision; apply the edits in place and
    ///   adopt [`TextContent::revision`].
    /// - [`TextSyncAction::Replace`]: a programmatic change with no usable
    ///   delta; replace the whole buffer and adopt the revision. This resets
    ///   native view state by design (document loads take this path).
    pub fn sync_action(&self, native: TextRevision) -> TextSyncAction<'_> {
        if self.revision.set == native.set {
            if self.revision.edit <= native.edit {
                return TextSyncAction::Keep;
            }
            // The application may not mint `edit` revisions; a declaration
            // ahead of the native edit counter violates the protocol, so the
            // adapter falls back to the always-correct full replacement.
            return TextSyncAction::Replace;
        }
        if self.base_revision == Some(native) {
            return TextSyncAction::ApplyEdits(&self.edits);
        }
        TextSyncAction::Replace
    }
}

impl PartialEq for TextContent {
    /// Compares document identity: revision and character length.
    ///
    /// Text bytes never participate, per the revision contract documented on
    /// [`TextContent`]; equal identity means reconciliation keeps the
    /// retained native buffer.
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision && self.char_len == other.char_len
    }
}

impl std::fmt::Debug for TextContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextContent")
            .field("char_len", &self.char_len)
            .field("revision", &self.revision)
            .field("base_revision", &self.base_revision)
            .field("edit_count", &self.edits.len())
            .finish()
    }
}

/// How an adapter synchronizes its native buffer with declared content.
///
/// Produced by [`TextContent::sync_action`]; the deterministic headless model
/// and every native adapter follow the same decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextSyncAction<'content> {
    /// The native buffer already contains the declared content.
    Keep,
    /// Apply these sequential edits to the native buffer in place.
    ApplyEdits(&'content [TextEdit]),
    /// Replace the whole native buffer with the declared text.
    Replace,
}

/// Semantic role of one syntax-highlight span.
///
/// The vocabulary is code-oriented and deliberately small; each platform
/// adapter resolves a role to a native palette color that follows the
/// system's light and dark appearance. The core never carries color values.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HighlightRole {
    /// Language keyword such as `fn`, `if`, `return`.
    Keyword,
    /// Quoted string or character literal.
    String,
    /// Numeric literal.
    Number,
    /// Comment, including documentation comments.
    Comment,
    /// Type, class, struct, or interface name.
    Type,
    /// Function or method name.
    Function,
    /// Variable or parameter name.
    Variable,
    /// Named constant or enumeration case.
    Constant,
    /// Operator token.
    Operator,
    /// Structural punctuation such as brackets and delimiters.
    Punctuation,
    /// Annotation or attribute such as `#[derive(..)]` or `@Override`.
    Attribute,
    /// Preprocessor or macro invocation.
    Preprocessor,
}

/// One highlighted range of a text area's document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighlightSpan {
    /// Highlighted character range.
    pub range: TextRange,
    /// Semantic meaning resolved to a native color by each adapter.
    pub role: HighlightRole,
}

impl HighlightSpan {
    /// Creates one highlighted range.
    pub const fn new(range: TextRange, role: HighlightRole) -> Self {
        Self { range, role }
    }
}

/// Revisioned set of highlight spans declared by a text area.
///
/// Validation requires spans to be ordered by start and non-overlapping,
/// with every range non-empty and inside the declared document.
///
/// Reconciliation identifies span sets by `revision`, never by comparing
/// spans: a producer must supply a new revision whenever it supplies a
/// different span set. Revision zero is reserved for [`HighlightSpans::none`].
#[derive(Clone)]
pub struct HighlightSpans {
    spans: Arc<[HighlightSpan]>,
    revision: u64,
}

impl HighlightSpans {
    /// Returns the empty span set carried by an unhighlighted text area.
    pub fn none() -> Self {
        Self {
            spans: Arc::from([]),
            revision: 0,
        }
    }

    /// Wraps computed spans under an application-assigned revision.
    ///
    /// The revision must be nonzero (zero identifies the empty set) and must
    /// change whenever the spans change.
    pub fn new(spans: impl Into<Arc<[HighlightSpan]>>, revision: u64) -> Self {
        Self {
            spans: spans.into(),
            revision,
        }
    }

    /// Returns the highlighted ranges in document order.
    pub fn spans(&self) -> &[HighlightSpan] {
        &self.spans
    }

    /// Returns the application-declared span-set identity.
    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

impl PartialEq for HighlightSpans {
    /// Compares span-set identity: revision and span count. Span values
    /// never participate, per the revision contract on [`HighlightSpans`].
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision && self.spans.len() == other.spans.len()
    }
}

impl std::fmt::Debug for HighlightSpans {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HighlightSpans")
            .field("revision", &self.revision)
            .field("span_count", &self.spans.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_order_sets_over_edits() {
        let first = TextRevision::new(1);
        let typed = first.next_edit().next_edit();
        let reloaded = typed.next_set();
        assert!(first < typed);
        assert!(typed < reloaded);
        assert_eq!(reloaded, TextRevision { set: 2, edit: 0 });
    }

    #[test]
    fn sequential_edits_apply_in_char_space() {
        let edits = [
            TextEdit::new(TextRange::new(2, 4), "んご"),
            TextEdit::new(TextRange::new(4, 4), "!"),
        ];
        assert_eq!(
            TextEdit::apply_all("にほhippo", &edits).as_deref(),
            Some("にほんご!ppo")
        );
        assert_eq!(
            TextEdit::apply_all("ab", &[TextEdit::new(TextRange::new(1, 5), "x")]),
            None
        );
    }

    #[test]
    fn char_ranges_map_to_byte_ranges_across_multibyte_text() {
        assert_eq!(
            char_range_to_byte_range("にほんご", TextRange::new(1, 3)),
            Some(3..9)
        );
        assert_eq!(
            char_range_to_byte_range("abc", TextRange::new(3, 3)),
            Some(3..3)
        );
        assert_eq!(char_range_to_byte_range("abc", TextRange::new(2, 4)), None);
    }

    #[test]
    fn echo_and_trailing_declarations_keep_the_native_buffer() {
        let native = TextRevision { set: 3, edit: 5 };
        let echo = TextContent::new("current", native);
        assert_eq!(echo.sync_action(native), TextSyncAction::Keep);

        let trailing = TextContent::new("older", TextRevision { set: 3, edit: 4 });
        assert_eq!(trailing.sync_action(native), TextSyncAction::Keep);
    }

    #[test]
    fn programmatic_edits_with_matching_base_apply_incrementally() {
        let native = TextRevision { set: 1, edit: 7 };
        let edits = vec![TextEdit::new(TextRange::new(0, 0), "// header\n")];
        let content =
            TextContent::new("// header\nbody", native.next_set()).with_edits(native, edits);
        assert!(matches!(
            content.sync_action(native),
            TextSyncAction::ApplyEdits(edits) if edits.len() == 1
        ));
    }

    #[test]
    fn unrelated_declarations_replace_the_buffer() {
        let native = TextRevision { set: 1, edit: 2 };
        let reload = TextContent::new("fresh", TextRevision::new(9));
        assert_eq!(reload.sync_action(native), TextSyncAction::Replace);

        // The application may not mint edit revisions.
        let minted = TextContent::new("bad", TextRevision { set: 1, edit: 3 });
        assert_eq!(minted.sync_action(native), TextSyncAction::Replace);
    }

    #[test]
    fn content_identity_follows_revision_and_length_only() {
        let revision = TextRevision::new(1);
        let first = TextContent::new("same length", revision);
        let second = TextContent::new("same length", revision);
        assert_eq!(first, second);
        assert_ne!(first, TextContent::new("same length", revision.next_edit()));
    }

    #[test]
    fn span_sets_are_identified_by_revision() {
        let span = HighlightSpan::new(TextRange::new(0, 2), HighlightRole::Keyword);
        assert_eq!(
            HighlightSpans::new(vec![span], 4),
            HighlightSpans::new(vec![span], 4)
        );
        assert_ne!(
            HighlightSpans::new(vec![span], 4),
            HighlightSpans::new(vec![span], 5)
        );
        assert_eq!(HighlightSpans::none().revision(), 0);
    }

    #[test]
    fn selections_normalize_into_ascending_ranges() {
        let backwards = TextSelection::new(9, 4);
        assert_eq!(backwards.range(), TextRange::new(4, 9));
        assert!(TextSelection::caret(3).is_caret());
    }
}
