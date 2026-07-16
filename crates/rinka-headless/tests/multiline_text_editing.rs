//! Consumer-level contracts for the multi-line text area.

use rinka_core::{
    Element, HighlightRole, HighlightSpan, HighlightSpans, Props, Renderer, TextChange,
    TextContent, TextEdit, TextRange, TextRevision, TextSelection, column, text_area,
};
use rinka_headless::{HeadlessBackend, TextAreaMutation};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

fn editor(content: TextContent) -> Element {
    column([text_area(content, "Editor", |_| {}).with_key("editor")]).with_key("screen")
}

fn recording_editor(content: TextContent, changes: Rc<RefCell<Vec<TextChange>>>) -> Element {
    column([text_area(content, "Editor", move |change| {
        changes.borrow_mut().push(change);
    })
    .with_key("editor")])
    .with_key("screen")
}

#[test]
fn mounting_a_text_area_records_document_spans_selection_and_read_only() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let revision = TextRevision::new(1);
    let content = TextContent::new("fn main() {}\n", revision);
    let spans = HighlightSpans::new(
        vec![HighlightSpan::new(
            TextRange::new(0, 2),
            HighlightRole::Keyword,
        )],
        1,
    );

    renderer
        .render(
            column([text_area(content, "Editor", |_| {})
                .read_only(true)
                .highlight_spans(spans)
                .text_selection(TextSelection::caret(3))
                .with_key("editor")])
            .with_key("screen"),
        )
        .unwrap();

    let handle = renderer.backend().find_by_key("editor").unwrap();
    let model = renderer.backend().text_area_model(handle).unwrap();
    assert_eq!(model.buffer, "fn main() {}\n");
    assert_eq!(model.revision, revision);
    assert_eq!(model.selection, Some(TextSelection::caret(3)));
    assert_eq!(model.spans_revision, 1);
    assert_eq!(model.span_count, 1);
    assert!(model.read_only);
}

#[test]
fn a_single_character_edit_round_trips_as_a_delta_without_reshipping_the_document() {
    let changes = Rc::new(RefCell::new(Vec::new()));
    let document: Arc<str> = Arc::from("hello world");
    let revision = TextRevision::new(1);
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(recording_editor(
            TextContent::new(Arc::clone(&document), revision),
            Rc::clone(&changes),
        ))
        .unwrap();
    let handle = renderer.backend().find_by_key("editor").unwrap();

    // The native view types one character; the adapter reports the delta.
    let change = renderer
        .backend_mut()
        .commit_text_edit(handle, vec![TextEdit::new(TextRange::new(5, 5), "!")])
        .unwrap();
    renderer
        .backend()
        .events_of(handle)
        .unwrap()
        .emit_text_change(change);

    let received = changes.borrow();
    assert_eq!(received.len(), 1);
    let change = &received[0];
    assert_eq!(change.base_revision, revision);
    assert_eq!(change.revision, revision.next_edit());
    // The event is delta-only: one inserted character, nothing re-shipped.
    assert_eq!(change.edits.len(), 1);
    assert_eq!(change.edits[0].replacement, "!");
    assert!(change.edits[0].range.is_empty());
    drop(received);

    // The application applies the delta and echoes the revision; the echoed
    // content shares the application's allocation and the modeled native
    // buffer is kept, not rewritten.
    let echoed: Arc<str> = Arc::from("hello! world");
    renderer
        .render(recording_editor(
            TextContent::new(Arc::clone(&echoed), revision.next_edit()),
            Rc::clone(&changes),
        ))
        .unwrap();
    let model = renderer.backend().text_area_model(handle).unwrap();
    assert_eq!(model.buffer, "hello! world");
    assert_eq!(model.mutations, vec![TextAreaMutation::KeptBuffer]);
    let stored = renderer.backend().props_of(handle).unwrap();
    let Props::TextArea { content, .. } = stored else {
        panic!("mounted node must carry text-area properties, got {stored:?}");
    };
    assert!(
        Arc::ptr_eq(&content.shared_text(), &echoed),
        "the pipeline must not copy the document"
    );
}

#[test]
fn a_stale_echo_never_clobbers_newer_native_edits() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let revision = TextRevision::new(1);
    renderer
        .render(editor(TextContent::new("ab", revision)))
        .unwrap();
    let handle = renderer.backend().find_by_key("editor").unwrap();

    // Two native edits land before the application's next render.
    let first = renderer
        .backend_mut()
        .commit_text_edit(handle, vec![TextEdit::new(TextRange::new(2, 2), "c")])
        .unwrap();
    renderer
        .backend_mut()
        .commit_text_edit(handle, vec![TextEdit::new(TextRange::new(3, 3), "d")])
        .unwrap();

    // The application echoes only the first edit; the native buffer is ahead
    // and must be kept exactly as typed.
    renderer
        .render(editor(TextContent::new("abc", first.revision)))
        .unwrap();
    let model = renderer.backend().text_area_model(handle).unwrap();
    assert_eq!(model.buffer, "abcd");
    assert_eq!(model.revision, TextRevision { set: 1, edit: 2 });
    assert_eq!(model.mutations, vec![TextAreaMutation::KeptBuffer]);
}

#[test]
fn programmatic_edits_with_a_matching_base_apply_in_place() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let revision = TextRevision::new(1);
    renderer
        .render(editor(TextContent::new("body\n", revision)))
        .unwrap();
    let handle = renderer.backend().find_by_key("editor").unwrap();

    let edits = vec![TextEdit::new(TextRange::new(0, 0), "// header\n")];
    let next = revision.next_set();
    renderer
        .render(editor(
            TextContent::new("// header\nbody\n", next).with_edits(revision, edits),
        ))
        .unwrap();

    let model = renderer.backend().text_area_model(handle).unwrap();
    assert_eq!(model.buffer, "// header\nbody\n");
    assert_eq!(model.revision, next);
    assert_eq!(
        model.mutations,
        vec![TextAreaMutation::AppliedEdits { edit_count: 1 }]
    );
}

#[test]
fn an_unrelated_document_load_replaces_the_buffer() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(editor(TextContent::new("old", TextRevision::new(1))))
        .unwrap();
    let handle = renderer.backend().find_by_key("editor").unwrap();

    renderer
        .render(editor(TextContent::new("brand new", TextRevision::new(7))))
        .unwrap();

    let model = renderer.backend().text_area_model(handle).unwrap();
    assert_eq!(model.buffer, "brand new");
    assert_eq!(model.revision, TextRevision::new(7));
    assert_eq!(model.mutations, vec![TextAreaMutation::ReplacedBuffer]);
}

#[test]
fn read_only_rejects_user_edits_but_accepts_programmatic_updates_and_selection() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let revision = TextRevision::new(1);
    renderer
        .render(
            column([
                text_area(TextContent::new("locked", revision), "Editor", |_| {})
                    .read_only(true)
                    .with_key("editor"),
            ])
            .with_key("screen"),
        )
        .unwrap();
    let handle = renderer.backend().find_by_key("editor").unwrap();

    let rejected = renderer
        .backend_mut()
        .commit_text_edit(handle, vec![TextEdit::new(TextRange::new(0, 0), "x")]);
    assert!(rejected.is_err(), "a read-only view must reject user edits");

    // Native selection still works while read-only.
    renderer
        .backend_mut()
        .commit_text_selection(handle, TextSelection::new(0, 6))
        .unwrap();

    // Programmatic replacement still works while read-only.
    renderer
        .render(
            column([text_area(
                TextContent::new("reloaded", revision.next_set()),
                "Editor",
                |_| {},
            )
            .read_only(true)
            .with_key("editor")])
            .with_key("screen"),
        )
        .unwrap();
    let model = renderer.backend().text_area_model(handle).unwrap();
    assert_eq!(model.buffer, "reloaded");
    assert_eq!(model.selection, Some(TextSelection::new(0, 6)));
}

#[test]
fn selection_set_and_get_round_trip_without_feedback() {
    let selections = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&selections);
    let revision = TextRevision::new(1);
    let view = move |selection: Option<TextSelection>| {
        let recorded = Rc::clone(&recorded);
        let mut area = text_area(TextContent::new("0123456789", revision), "Editor", |_| {})
            .on_selection_change(move |selection| recorded.borrow_mut().push(selection));
        if let Some(selection) = selection {
            area = area.text_selection(selection);
        }
        column([area.with_key("editor")]).with_key("screen")
    };
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer.render(view(None)).unwrap();
    let handle = renderer.backend().find_by_key("editor").unwrap();

    // Programmatic set: the controlled selection reaches the native model.
    renderer
        .render(view(Some(TextSelection::new(2, 6))))
        .unwrap();
    let model = renderer.backend().text_area_model(handle).unwrap();
    assert_eq!(model.selection, Some(TextSelection::new(2, 6)));
    assert_eq!(
        model.mutations,
        vec![
            TextAreaMutation::KeptBuffer,
            TextAreaMutation::SetSelection(TextSelection::new(2, 6))
        ]
    );

    // Native get: the user moves the caret, the application stores and
    // echoes it, and the echo does not re-apply the selection.
    let native = renderer
        .backend_mut()
        .commit_text_selection(handle, TextSelection::caret(9))
        .unwrap();
    renderer
        .backend()
        .events_of(handle)
        .unwrap()
        .emit_selection_change(native);
    assert_eq!(*selections.borrow(), vec![TextSelection::caret(9)]);
    renderer
        .render(view(Some(TextSelection::caret(9))))
        .unwrap();
    let model = renderer.backend().text_area_model(handle).unwrap();
    assert_eq!(
        model
            .mutations
            .iter()
            .filter(|mutation| matches!(mutation, TextAreaMutation::SetSelection(_)))
            .count(),
        1,
        "an echoed selection must not be re-applied"
    );
}

#[test]
fn highlight_span_updates_patch_in_place_without_recreating_the_view() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let revision = TextRevision::new(1);
    let spans = |set_revision: u64| {
        HighlightSpans::new(
            vec![
                HighlightSpan::new(TextRange::new(0, 2), HighlightRole::Keyword),
                HighlightSpan::new(TextRange::new(3, 7), HighlightRole::Function),
            ],
            set_revision,
        )
    };
    let view = |set_revision: u64| {
        column([
            text_area(TextContent::new("fn main() {}", revision), "Editor", |_| {})
                .highlight_spans(spans(set_revision))
                .with_key("editor"),
        ])
        .with_key("screen")
    };
    renderer.render(view(1)).unwrap();
    let handle = renderer.backend().find_by_key("editor").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer.render(view(2)).unwrap();

    assert_eq!(stats.replaced, 0, "span updates must not recreate the view");
    assert_eq!(stats.created, 0);
    assert_eq!(stats.patched, 1);
    assert_eq!(renderer.backend().find_by_key("editor"), Some(handle));
    let model = renderer.backend().text_area_model(handle).unwrap();
    assert_eq!(
        model.mutations,
        vec![
            TextAreaMutation::KeptBuffer,
            TextAreaMutation::AppliedSpans {
                revision: 2,
                span_count: 2
            }
        ]
    );

    // An unchanged span revision issues no span application at all.
    renderer.backend_mut().clear_operations();
    let stats = renderer.render(view(2)).unwrap();
    assert_eq!(stats.patched, 0);
    let model = renderer.backend().text_area_model(handle).unwrap();
    assert_eq!(model.mutations.len(), 2);
}

#[test]
fn invalid_spans_selections_and_edits_are_typed_tree_errors() {
    let revision = TextRevision::new(1);
    let cases: Vec<(&str, Element)> = vec![
        (
            "overlapping spans",
            column([
                text_area(TextContent::new("abcdef", revision), "Editor", |_| {})
                    .highlight_spans(HighlightSpans::new(
                        vec![
                            HighlightSpan::new(TextRange::new(0, 3), HighlightRole::Keyword),
                            HighlightSpan::new(TextRange::new(2, 5), HighlightRole::String),
                        ],
                        1,
                    ))
                    .with_key("editor"),
            ])
            .with_key("screen"),
        ),
        (
            "span past the document",
            column([
                text_area(TextContent::new("ab", revision), "Editor", |_| {})
                    .highlight_spans(HighlightSpans::new(
                        vec![HighlightSpan::new(
                            TextRange::new(0, 3),
                            HighlightRole::Keyword,
                        )],
                        1,
                    ))
                    .with_key("editor"),
            ])
            .with_key("screen"),
        ),
        (
            "empty span",
            column([
                text_area(TextContent::new("ab", revision), "Editor", |_| {})
                    .highlight_spans(HighlightSpans::new(
                        vec![HighlightSpan::new(
                            TextRange::new(1, 1),
                            HighlightRole::Keyword,
                        )],
                        1,
                    ))
                    .with_key("editor"),
            ])
            .with_key("screen"),
        ),
        (
            "selection past the document",
            column([
                text_area(TextContent::new("ab", revision), "Editor", |_| {})
                    .text_selection(TextSelection::caret(3))
                    .with_key("editor"),
            ])
            .with_key("screen"),
        ),
        (
            "inverted edit range",
            column([text_area(
                TextContent::new("ab", revision)
                    .with_edits(revision, vec![TextEdit::new(TextRange::new(2, 1), "x")]),
                "Editor",
                |_| {},
            )
            .with_key("editor")])
            .with_key("screen"),
        ),
    ];

    for (case, element) in cases {
        let mut renderer = Renderer::new(HeadlessBackend::new());
        let error = renderer.render(element).unwrap_err();
        assert!(
            error.to_string().contains("invalid text area"),
            "{case} must be a typed text-area error, got: {error}"
        );
        assert!(
            renderer.backend().operations().is_empty(),
            "{case} must be rejected before any native mutation"
        );
    }
}
