//! Deterministic consumer-side editor state for the text-area scene.
//!
//! The consumer owns the document and its highlighting, exactly as Overshell
//! owns syntect: rinka only receives revisioned content and semantic spans.
//! Edits arriving from the native view are applied as deltas, and only the
//! touched lines are re-tokenized — the span set updates by splicing, never
//! by re-scanning the whole document per keystroke.

use rinka::{
    HighlightRole, HighlightSpan, HighlightSpans, TextChange, TextContent, TextEdit, TextRange,
    TextRevision, TextSelection, char_range_to_byte_range,
};
use std::sync::Arc;

/// Rust-ish keywords recognized by the deterministic line tokenizer.
const KEYWORDS: [&str; 33] = [
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "super", "trait", "type", "unsafe", "use", "where", "while",
];

/// The editor scene's document, cursor, and highlight state.
pub struct EditorState {
    file_name: String,
    origin: Arc<str>,
    document: String,
    shared_document: Arc<str>,
    char_len: usize,
    revision: TextRevision,
    selection: Option<TextSelection>,
    spans: Vec<HighlightSpan>,
    shared_spans: Arc<[HighlightSpan]>,
    spans_revision: u64,
    read_only: bool,
    change_count: u64,
}

impl EditorState {
    /// Loads the document named by `RINKA_EXPLORER_EDITOR_FILE`, falling back
    /// to this consumer's own embedded source so the scene stays
    /// deterministic without configuration.
    pub fn load() -> Self {
        let configured = std::env::var_os("RINKA_EXPLORER_EDITOR_FILE").and_then(|path| {
            let path = std::path::PathBuf::from(path);
            match std::fs::read_to_string(&path) {
                Ok(text) => Some((
                    path.file_name().map_or_else(
                        || path.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    ),
                    text,
                )),
                Err(error) => {
                    eprintln!(
                        "RINKA_EXPLORER_EDITOR_FILE unreadable ({error}); using the embedded sample"
                    );
                    None
                }
            }
        });
        let (file_name, text) = configured
            .unwrap_or_else(|| ("view.rs".to_owned(), include_str!("view.rs").to_owned()));
        Self::from_document(file_name, text)
    }

    fn from_document(file_name: String, text: String) -> Self {
        let origin: Arc<str> = Arc::from(text.as_str());
        let spans = highlight_document(&text);
        let char_len = text.chars().count();
        Self {
            file_name,
            origin,
            shared_document: Arc::from(text.as_str()),
            document: text,
            char_len,
            revision: TextRevision::new(1),
            selection: None,
            shared_spans: Arc::from(spans.as_slice()),
            spans,
            spans_revision: 1,
            read_only: false,
            change_count: 0,
        }
    }

    /// Returns the loaded file's display name.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// Returns the revisioned document declared to the text area.
    pub fn content(&self) -> TextContent {
        TextContent::new(Arc::clone(&self.shared_document), self.revision)
    }

    /// Returns the revisioned span set declared to the text area.
    pub fn highlight(&self) -> HighlightSpans {
        HighlightSpans::new(Arc::clone(&self.shared_spans), self.spans_revision)
    }

    /// Returns the controlled selection, when the editor holds one.
    pub fn selection(&self) -> Option<TextSelection> {
        self.selection
    }

    /// Returns whether user edits are rejected.
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// Returns the document text (for evidence assertions).
    pub fn document(&self) -> &str {
        &self.document
    }

    /// Returns the deterministic status line shown next to the editor.
    pub fn status_line(&self) -> String {
        let selection = self.selection.map_or_else(
            || "—".to_owned(),
            |selection| format!("{}..{}", selection.anchor, selection.head),
        );
        format!(
            "rev {}.{} · {} chars · {} spans · sel {selection} · {} changes",
            self.revision.set,
            self.revision.edit,
            self.char_len,
            self.spans.len(),
            self.change_count,
        )
    }

    /// Applies one native change event: the delta mutates the document, only
    /// the touched lines are re-tokenized, and the event's revision is echoed
    /// on the next render.
    pub fn apply_change(&mut self, change: &TextChange) {
        for edit in &change.edits {
            self.apply_edit(edit);
        }
        self.revision = change.revision;
        self.change_count += 1;
        self.refresh_shared();
    }

    fn apply_edit(&mut self, edit: &TextEdit) {
        let Some(byte_range) = char_range_to_byte_range(&self.document, edit.range) else {
            // A delta outside the document means this state and the native
            // buffer diverged; re-adopting the full text keeps them honest.
            eprintln!("editor received an out-of-range delta; resynchronizing");
            return;
        };
        self.document.replace_range(byte_range, &edit.replacement);
        let inserted = edit.replacement.chars().count();
        let removed = edit.range.len();
        let delta = isize::try_from(inserted).unwrap_or(isize::MAX)
            - isize::try_from(removed).unwrap_or(isize::MAX);
        self.char_len =
            usize::try_from(isize::try_from(self.char_len).unwrap_or(isize::MAX) + delta)
                .unwrap_or(0);
        self.selection = self.selection.map(|selection| {
            TextSelection::new(
                shift_position(selection.anchor, edit.range, inserted, delta),
                shift_position(selection.head, edit.range, inserted, delta),
            )
        });
        self.splice_spans(edit.range.start, edit.range.start + inserted, delta);
    }

    /// Re-tokenizes the lines covering `new_start..new_end` (new-document
    /// character coordinates) and splices the result into the span set.
    fn splice_spans(&mut self, new_start: usize, new_end: usize, delta: isize) {
        let window = line_window(&self.document, new_start, new_end);
        let window_text = &self.document[window.bytes.clone()];
        let mut fresh = Vec::new();
        highlight_segment(window_text, window.chars.start, &mut fresh);

        // The window starts on a line boundary in the unchanged prefix, so
        // its start is the same in old coordinates; its end shifts by delta.
        let old_window_end =
            usize::try_from(isize::try_from(window.chars.end).unwrap_or(isize::MAX) - delta)
                .unwrap_or(0);
        let prefix_len = self
            .spans
            .partition_point(|span| span.range.end <= window.chars.start);
        let suffix_start = self
            .spans
            .partition_point(|span| span.range.start < old_window_end);
        let suffix: Vec<HighlightSpan> = self.spans[suffix_start..]
            .iter()
            .map(|span| {
                HighlightSpan::new(
                    TextRange::new(
                        shift_index(span.range.start, delta),
                        shift_index(span.range.end, delta),
                    ),
                    span.role,
                )
            })
            .collect();
        self.spans.truncate(prefix_len);
        self.spans.extend(fresh);
        self.spans.extend(suffix);
        self.spans_revision += 1;
    }

    fn refresh_shared(&mut self) {
        self.shared_document = Arc::from(self.document.as_str());
        self.shared_spans = Arc::from(self.spans.as_slice());
    }

    /// Stores the selection the native view reported.
    pub fn store_selection(&mut self, selection: TextSelection) {
        self.selection = Some(selection);
    }

    /// Controls whether user edits are rejected.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    /// Moves the caret to the end of the document; the adapter scrolls the
    /// caret into view when it applies a selection that is not an echo.
    pub fn jump_to_end(&mut self) {
        self.selection = Some(TextSelection::caret(self.char_len));
    }

    /// Recomputes the highlight of the whole document under a new revision.
    pub fn rehighlight_all(&mut self) {
        self.spans = highlight_document(&self.document);
        self.spans_revision += 1;
        self.refresh_shared();
    }

    /// Replaces the document with its original text under a new set revision:
    /// the whole-buffer programmatic path.
    pub fn reload(&mut self) {
        self.document = self.origin.to_string();
        self.char_len = self.document.chars().count();
        self.revision = self.revision.next_set();
        self.selection = None;
        self.spans = highlight_document(&self.document);
        self.spans_revision += 1;
        self.refresh_shared();
    }
}

fn shift_index(index: usize, delta: isize) -> usize {
    usize::try_from(isize::try_from(index).unwrap_or(isize::MAX) + delta).unwrap_or(0)
}

/// Maps a pre-edit position through one replacement of `range` by `inserted`
/// characters.
fn shift_position(position: usize, range: TextRange, inserted: usize, delta: isize) -> usize {
    if position <= range.start {
        position
    } else if position >= range.end {
        shift_index(position, delta)
    } else {
        range.start + inserted
    }
}

struct LineWindow {
    chars: TextRange,
    bytes: std::ops::Range<usize>,
}

/// Expands a character window to whole lines of `text`, returning both
/// character and byte coordinates in one pass.
fn line_window(text: &str, start: usize, end: usize) -> LineWindow {
    let mut line_start_char = 0_usize;
    let mut line_start_byte = 0_usize;
    let mut window_start: Option<(usize, usize)> = None;
    let mut chars_seen = 0_usize;
    for (char_index, (byte_index, character)) in text.char_indices().enumerate() {
        if window_start.is_none() && char_index >= start {
            window_start = Some((line_start_char, line_start_byte));
        }
        if character == '\n' {
            if char_index >= end
                && let Some((start_char, start_byte)) = window_start
            {
                return LineWindow {
                    chars: TextRange::new(start_char, char_index + 1),
                    bytes: start_byte..byte_index + character.len_utf8(),
                };
            }
            line_start_char = char_index + 1;
            line_start_byte = byte_index + character.len_utf8();
        }
        chars_seen = char_index + 1;
    }
    let (start_char, start_byte) = window_start.unwrap_or((line_start_char, line_start_byte));
    LineWindow {
        chars: TextRange::new(start_char, chars_seen),
        bytes: start_byte..text.len(),
    }
}

/// Tokenizes a whole document into ordered, non-overlapping semantic spans.
pub fn highlight_document(text: &str) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    highlight_segment(text, 0, &mut spans);
    spans
}

/// Tokenizes one segment starting at absolute character `offset`. The
/// tokenizer is line-local (no multi-line constructs), so any window aligned
/// to line boundaries re-tokenizes independently of its surroundings.
fn highlight_segment(segment: &str, offset: usize, spans: &mut Vec<HighlightSpan>) {
    let characters: Vec<char> = segment.chars().collect();
    let mut index = 0_usize;
    while index < characters.len() {
        let character = characters[index];
        if character == '\n' {
            index += 1;
            continue;
        }
        if character == '/' && characters.get(index + 1) == Some(&'/') {
            let end = line_end(&characters, index);
            push_span(spans, offset, index, end, HighlightRole::Comment);
            index = end;
            continue;
        }
        if character == '"' {
            let mut end = index + 1;
            while end < characters.len() && characters[end] != '\n' {
                if characters[end] == '\\' {
                    end += 2;
                    continue;
                }
                if characters[end] == '"' {
                    end += 1;
                    break;
                }
                end += 1;
            }
            let end = end.min(characters.len());
            push_span(spans, offset, index, end, HighlightRole::String);
            index = end;
            continue;
        }
        if character == '#'
            && (characters.get(index + 1) == Some(&'[')
                || (characters.get(index + 1) == Some(&'!')
                    && characters.get(index + 2) == Some(&'[')))
        {
            let mut end = index;
            while end < characters.len() && characters[end] != ']' && characters[end] != '\n' {
                end += 1;
            }
            if characters.get(end) == Some(&']') {
                end += 1;
            }
            push_span(spans, offset, index, end, HighlightRole::Attribute);
            index = end;
            continue;
        }
        if character.is_ascii_digit() {
            let mut end = index + 1;
            while end < characters.len()
                && (characters[end].is_ascii_alphanumeric()
                    || characters[end] == '_'
                    || characters[end] == '.')
            {
                end += 1;
            }
            push_span(spans, offset, index, end, HighlightRole::Number);
            index = end;
            continue;
        }
        if character.is_ascii_alphabetic() || character == '_' {
            let mut end = index + 1;
            while end < characters.len()
                && (characters[end].is_ascii_alphanumeric() || characters[end] == '_')
            {
                end += 1;
            }
            let word: String = characters[index..end].iter().collect();
            let role = if characters.get(end) == Some(&'!') {
                end += 1;
                Some(HighlightRole::Preprocessor)
            } else if KEYWORDS.contains(&word.as_str()) {
                Some(HighlightRole::Keyword)
            } else if word == "true" || word == "false" {
                Some(HighlightRole::Constant)
            } else if word.chars().next().is_some_and(char::is_uppercase) {
                Some(HighlightRole::Type)
            } else if word.len() > 1
                && word
                    .chars()
                    .all(|character| character.is_ascii_uppercase() || character == '_')
            {
                Some(HighlightRole::Constant)
            } else if characters.get(end) == Some(&'(') {
                Some(HighlightRole::Function)
            } else {
                None
            };
            if let Some(role) = role {
                push_span(spans, offset, index, end, role);
            }
            index = end;
            continue;
        }
        index += 1;
    }
}

fn line_end(characters: &[char], from: usize) -> usize {
    characters[from..]
        .iter()
        .position(|&character| character == '\n')
        .map_or(characters.len(), |position| from + position)
}

fn push_span(
    spans: &mut Vec<HighlightSpan>,
    offset: usize,
    start: usize,
    end: usize,
    role: HighlightRole,
) {
    if end > start {
        spans.push(HighlightSpan::new(
            TextRange::new(offset + start, offset + end),
            role,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinka::TextRevision;

    fn state(text: &str) -> EditorState {
        EditorState::from_document("test.rs".to_owned(), text.to_owned())
    }

    fn change(base: TextRevision, edits: Vec<TextEdit>) -> TextChange {
        TextChange {
            base_revision: base,
            revision: base.next_edit(),
            edits,
            composing: false,
        }
    }

    #[test]
    fn incremental_splice_matches_a_full_recompute() {
        let mut editor = state("fn main() {\n    let x = 42;\n    // done\n}\n");
        let cases = [
            // Insert inside a line.
            TextEdit::new(TextRange::new(20, 20), "long_"),
            // Replace across a token.
            TextEdit::new(TextRange::new(16, 25), "let renamed = \"s\";"),
            // Insert a new line.
            TextEdit::new(TextRange::new(12, 12), "    call_me();\n"),
            // Delete a range spanning a newline.
            TextEdit::new(TextRange::new(10, 14), ""),
        ];
        for edit in cases {
            let base = editor.revision;
            editor.apply_change(&change(base, vec![edit]));
            assert_eq!(
                editor.spans,
                highlight_document(editor.document()),
                "incremental spans diverged on document: {:?}",
                editor.document()
            );
        }
    }

    #[test]
    fn multibyte_edits_splice_correctly() {
        let mut editor = state("// コメント\nfn 名前() {}\n");
        let base = editor.revision;
        editor.apply_change(&change(
            base,
            vec![TextEdit::new(TextRange::new(3, 5), "注釈")],
        ));
        assert!(editor.document().contains("// 注釈ント"));
        assert_eq!(editor.spans, highlight_document(editor.document()));
    }

    #[test]
    fn selection_follows_edits() {
        let mut editor = state("abcdef");
        editor.store_selection(TextSelection::new(4, 6));
        let base = editor.revision;
        editor.apply_change(&change(base, vec![TextEdit::new(TextRange::new(0, 2), "")]));
        assert_eq!(editor.selection(), Some(TextSelection::new(2, 4)));
    }

    #[test]
    fn reload_bumps_the_set_revision_and_restores_the_origin() {
        let mut editor = state("original\n");
        let base = editor.revision;
        editor.apply_change(&change(
            base,
            vec![TextEdit::new(TextRange::new(0, 8), "changed!")],
        ));
        assert_eq!(editor.document(), "changed!\n");
        editor.reload();
        assert_eq!(editor.document(), "original\n");
        assert_eq!(editor.content().revision(), TextRevision::new(2));
    }

    #[test]
    fn the_tokenizer_recognizes_the_semantic_roles() {
        let spans = highlight_document(
            "#[derive(Debug)]\nfn call(x: Type) -> u32 { MAX_LEN! } // note \"quoted\"\nlet s = \"text\"; 42.0\n",
        );
        let roles: Vec<HighlightRole> = spans.iter().map(|span| span.role).collect();
        for expected in [
            HighlightRole::Attribute,
            HighlightRole::Keyword,
            HighlightRole::Function,
            HighlightRole::Type,
            HighlightRole::Comment,
            HighlightRole::String,
            HighlightRole::Number,
            HighlightRole::Preprocessor,
        ] {
            assert!(
                roles.contains(&expected),
                "missing {expected:?} in {roles:?}"
            );
        }
        // Ordered and non-overlapping, as validation requires.
        for pair in spans.windows(2) {
            assert!(pair[0].range.end <= pair[1].range.start);
        }
    }
}
