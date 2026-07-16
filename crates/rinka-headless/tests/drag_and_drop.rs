//! Consumer-level drag-and-drop contracts: OS file drop-in, promised file
//! drag-out, and intra-application typed-payload drags, simulated
//! deterministically by the headless host.
//!
//! Reactive tests extract the stable event binding first and emit outside
//! the renderer borrow — the same discipline a platform adapter follows,
//! because delivery re-renders through the runtime. Non-reactive tests
//! drive the backend's deterministic drag-session simulation directly.

use rinka_core::{
    AppRuntime, CollectionPattern, Component, Dispatch, DragPayload, DropPosition, Element,
    EventBindings, FileDrop, FilePromise, PayloadDrop, PlatformServices, Renderer, UpdateContext,
    column, label, list, list_row,
};
use rinka_headless::HeadlessBackend;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// A two-pane file-manager model: one file list whose rows drag typed
/// payloads and promised files, plus a folder list accepting the payloads.
struct TwoPanes {
    files: Vec<&'static str>,
    moves: Vec<(String, &'static str, DropPosition)>,
    file_drops: Vec<FileDrop>,
    exports: Vec<Result<String, String>>,
}

enum PaneMessage {
    MovedToFolder(&'static str, PayloadDrop),
    FilesDropped(FileDrop),
    Exported(Result<String, String>),
}

const FILE_PAYLOAD_TYPE: &str = "demo.file";

impl Component for TwoPanes {
    type Message = PaneMessage;

    fn update(&mut self, message: Self::Message, _context: &UpdateContext<Self::Message>) {
        match message {
            PaneMessage::MovedToFolder(folder, drop) => {
                self.files.retain(|file| *file != drop.payload.id());
                self.moves
                    .push((drop.payload.id().to_owned(), folder, drop.position));
            }
            PaneMessage::FilesDropped(drop) => self.file_drops.push(drop),
            PaneMessage::Exported(result) => self.exports.push(result),
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let drop_dispatch = dispatch.clone();
        let file_rows = self.files.iter().map(|file| {
            let name = *file;
            let export_dispatch = dispatch.clone();
            list_row(name, None, None, false, false, name, || {})
                .drag_payload(DragPayload::new(FILE_PAYLOAD_TYPE, name))
                .draggable_file(FilePromise::new(
                    format!("{name}.txt"),
                    "public.plain-text",
                    move |path| {
                        let outcome = std::fs::write(path, format!("exported {name}"))
                            .map_err(|error| error.to_string());
                        export_dispatch.emit(PaneMessage::Exported(
                            outcome.clone().map(|()| path.display().to_string()),
                        ));
                        outcome
                    },
                ))
                .with_key(format!("file-{name}"))
        });
        let folder_rows = ["inbox", "archive"].into_iter().map(|folder| {
            let folder_dispatch = dispatch.clone();
            list_row(folder, None, None, false, false, folder, || {})
                .on_drop_accepting([FILE_PAYLOAD_TYPE], move |drop| {
                    folder_dispatch.emit(PaneMessage::MovedToFolder(folder, drop));
                })
                .with_key(format!("folder-{folder}"))
        });
        column([
            list("Files", file_rows)
                .collection_pattern(CollectionPattern::ContentList)
                .on_file_drop(move |drop| drop_dispatch.emit(PaneMessage::FilesDropped(drop)))
                .with_key("files"),
            list("Folders", folder_rows)
                .collection_pattern(CollectionPattern::ContentList)
                .with_key("folders"),
        ])
        .with_key("panes")
    }
}

fn mounted_two_panes() -> AppRuntime<HeadlessBackend, TwoPanes> {
    AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        TwoPanes {
            files: vec!["readme", "notes"],
            moves: Vec::new(),
            file_drops: Vec::new(),
            exports: Vec::new(),
        },
        PlatformServices::default(),
    )
    .expect("two-pane component mounts")
}

fn bindings_of(runtime: &AppRuntime<HeadlessBackend, TwoPanes>, key: &str) -> EventBindings {
    runtime
        .with_renderer(|renderer| {
            let handle = renderer.backend().find_by_key(key)?;
            renderer.backend().events_of(handle)
        })
        .expect("mounted element has a stable binding")
}

#[test]
fn a_recorded_row_drag_onto_a_sibling_target_delivers_the_move_intent() {
    let runtime = mounted_two_panes();
    let source = bindings_of(&runtime, "file-readme");
    let target = bindings_of(&runtime, "folder-archive");

    // One drag session: the source's current payload lands on the target
    // with a position in the target's local coordinates.
    let payload = source.drag_payload().expect("row declares a payload");
    assert!(target.emit_payload_drop(PayloadDrop {
        payload,
        position: DropPosition::new(24.0, 9.0),
    }));

    runtime.with_component(|component| {
        assert_eq!(
            component.moves,
            vec![("readme".to_owned(), "archive", DropPosition::new(24.0, 9.0))]
        );
        // The move intent reconciled the source pane deterministically.
        assert_eq!(component.files, vec!["notes"]);
    });
    // The dragged row unmounted with its file.
    assert!(
        runtime.with_renderer(|renderer| renderer.backend().find_by_key("file-readme").is_none())
    );
}

#[test]
fn a_payload_type_the_target_does_not_accept_is_refused() {
    let runtime = mounted_two_panes();
    let source = bindings_of(&runtime, "file-readme");
    // The files list accepts OS file drops, not the intra-app payload.
    let files_list = bindings_of(&runtime, "files");

    let payload = source.drag_payload().expect("row declares a payload");
    assert!(!files_list.emit_payload_drop(PayloadDrop {
        payload,
        position: DropPosition::new(1.0, 1.0),
    }));

    runtime.with_component(|component| assert!(component.moves.is_empty()));
}

#[test]
fn a_file_drop_arrives_with_element_local_position_and_paths_in_order() {
    let runtime = mounted_two_panes();
    let files_list = bindings_of(&runtime, "files");

    assert!(files_list.emit_file_drop(FileDrop {
        paths: vec![
            PathBuf::from("/tmp/first.txt"),
            PathBuf::from("/tmp/second.txt"),
        ],
        position: DropPosition::new(120.5, 48.25),
    }));

    runtime.with_component(|component| {
        assert_eq!(
            component.file_drops,
            vec![FileDrop {
                paths: vec![
                    PathBuf::from("/tmp/first.txt"),
                    PathBuf::from("/tmp/second.txt"),
                ],
                position: DropPosition::new(120.5, 48.25),
            }]
        );
    });
}

#[test]
fn a_file_drop_on_an_element_without_a_file_target_is_refused() {
    let runtime = mounted_two_panes();
    let folders = bindings_of(&runtime, "folders");

    assert!(!folders.emit_file_drop(FileDrop {
        paths: vec![PathBuf::from("/tmp/first.txt")],
        position: DropPosition::new(4.0, 4.0),
    }));

    runtime.with_component(|component| assert!(component.file_drops.is_empty()));
}

#[test]
fn a_materialized_promise_writes_lazily_and_reports_through_a_message() {
    let runtime = mounted_two_panes();
    let source = bindings_of(&runtime, "file-notes");
    let destination = std::env::temp_dir().join(format!(
        "rinka-drag-and-drop-promise-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&destination).expect("temp destination directory");

    let promise = source.file_promise().expect("row declares a promise");
    // Nothing materializes before the destination accepts.
    assert!(!destination.join(promise.file_name()).exists());

    let written = destination.join(promise.file_name());
    promise
        .write_to(&written)
        .expect("the promise materializes on acceptance");

    assert_eq!(
        std::fs::read_to_string(&written).expect("promised content"),
        "exported notes"
    );
    // The write callback's completion dispatched back into update.
    runtime.with_component(|component| {
        assert_eq!(component.exports, vec![Ok(written.display().to_string())]);
    });
    std::fs::remove_dir_all(&destination).expect("temp destination cleanup");
}

/// Builds a non-reactive two-list tree over recording sinks, for driving the
/// backend's deterministic drag-session simulation directly.
struct RecordedPanes {
    renderer: Renderer<HeadlessBackend>,
    moves: Rc<RefCell<Vec<PayloadDrop>>>,
    file_drops: Rc<RefCell<Vec<FileDrop>>>,
}

fn recorded_panes() -> RecordedPanes {
    let moves = Rc::new(RefCell::new(Vec::new()));
    let file_drops = Rc::new(RefCell::new(Vec::new()));
    let move_sink = moves.clone();
    let drop_sink = file_drops.clone();
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(column([
            list(
                "Files",
                [
                    list_row("readme", None, None, false, false, "readme", || {})
                        .drag_payload(DragPayload::new(FILE_PAYLOAD_TYPE, "readme"))
                        .draggable_file(FilePromise::new(
                            "readme.txt",
                            "public.plain-text",
                            |path| {
                                std::fs::write(path, "exported readme")
                                    .map_err(|error| error.to_string())
                            },
                        ))
                        .with_key("file-readme"),
                ],
            )
            .collection_pattern(CollectionPattern::ContentList)
            .on_file_drop(move |drop| drop_sink.borrow_mut().push(drop))
            .with_key("files"),
            list(
                "Folders",
                [
                    list_row("archive", None, None, false, false, "archive", || {})
                        .on_drop_accepting([FILE_PAYLOAD_TYPE], move |drop| {
                            move_sink.borrow_mut().push(drop);
                        })
                        .with_key("folder-archive"),
                ],
            )
            .collection_pattern(CollectionPattern::ContentList)
            .with_key("folders"),
        ]))
        .expect("recorded panes render");
    RecordedPanes {
        renderer,
        moves,
        file_drops,
    }
}

#[test]
fn the_backend_simulates_a_complete_drag_session_source_to_target() {
    let panes = recorded_panes();
    let backend = panes.renderer.backend();
    let source = backend.find_by_key("file-readme").unwrap();
    let target = backend.find_by_key("folder-archive").unwrap();

    let delivered = backend
        .simulate_payload_drag(source, target, DropPosition::new(32.0, 6.5))
        .expect("the simulated session delivers");

    assert_eq!(delivered, DragPayload::new(FILE_PAYLOAD_TYPE, "readme"));
    assert_eq!(
        *panes.moves.borrow(),
        vec![PayloadDrop {
            payload: DragPayload::new(FILE_PAYLOAD_TYPE, "readme"),
            position: DropPosition::new(32.0, 6.5),
        }]
    );
}

#[test]
fn the_backend_simulation_refuses_sessions_the_models_do_not_declare() {
    let panes = recorded_panes();
    let backend = panes.renderer.backend();
    let source = backend.find_by_key("file-readme").unwrap();
    let files = backend.find_by_key("files").unwrap();
    let folder = backend.find_by_key("folder-archive").unwrap();

    // The files list does not accept the intra-app payload type.
    assert!(
        backend
            .simulate_payload_drag(source, files, DropPosition::new(0.0, 0.0))
            .is_err()
    );
    // The folder row declares no drag payload of its own.
    assert!(
        backend
            .simulate_payload_drag(folder, folder, DropPosition::new(0.0, 0.0))
            .is_err()
    );
    // The folder row accepts payloads, not OS files.
    assert!(
        backend
            .simulate_file_drop(
                folder,
                [PathBuf::from("/tmp/first.txt")],
                DropPosition::new(0.0, 0.0)
            )
            .is_err()
    );
    // The folder row promises no file.
    assert!(
        backend
            .materialize_file_promise(folder, &std::env::temp_dir())
            .is_err()
    );
    assert!(panes.moves.borrow().is_empty());
    assert!(panes.file_drops.borrow().is_empty());
}

#[test]
fn the_backend_simulation_passes_the_drop_position_through_unchanged() {
    let panes = recorded_panes();
    let backend = panes.renderer.backend();
    let files = backend.find_by_key("files").unwrap();

    backend
        .simulate_file_drop(
            files,
            [PathBuf::from("/tmp/dropped.bin")],
            DropPosition::new(200.25, 90.75),
        )
        .expect("the files list accepts OS file drops");

    assert_eq!(
        *panes.file_drops.borrow(),
        vec![FileDrop {
            paths: vec![PathBuf::from("/tmp/dropped.bin")],
            position: DropPosition::new(200.25, 90.75),
        }]
    );
}

#[test]
fn the_backend_materializes_a_promise_into_the_destination_directory() {
    let panes = recorded_panes();
    let backend = panes.renderer.backend();
    let source = backend.find_by_key("file-readme").unwrap();
    let destination = std::env::temp_dir().join(format!(
        "rinka-drag-and-drop-backend-promise-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&destination).expect("temp destination directory");

    let written = backend
        .materialize_file_promise(source, &destination)
        .expect("the promise materializes");

    assert_eq!(written, destination.join("readme.txt"));
    assert_eq!(
        std::fs::read_to_string(&written).expect("promised content"),
        "exported readme"
    );
    std::fs::remove_dir_all(&destination).expect("temp destination cleanup");
}

#[test]
fn drag_models_reconcile_in_place_and_handler_only_changes_do_not_patch() {
    let tree = |payload_id: &str| {
        list(
            "Files",
            [
                list_row("readme", None, None, false, false, "readme", || {})
                    .drag_payload(DragPayload::new(FILE_PAYLOAD_TYPE, payload_id))
                    .with_key("row"),
            ],
        )
        .collection_pattern(CollectionPattern::ContentList)
        .on_file_drop(|_| {})
        .with_key("files")
    };
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer.render(tree("readme")).unwrap();
    let row = renderer.backend().find_by_key("row").unwrap();
    let files = renderer.backend().find_by_key("files").unwrap();
    assert!(
        renderer
            .backend()
            .drop_target_of(files)
            .is_some_and(rinka_core::DropTarget::accepts_files)
    );

    // Identical declarative drag state issues no patch even though the
    // handler closures are fresh instances.
    let stats = renderer.render(tree("readme")).unwrap();
    assert_eq!(stats.patched, 0);

    // A payload identity change patches the retained native object in place.
    let stats = renderer.render(tree("changelog")).unwrap();
    assert_eq!(stats.patched, 1);
    assert_eq!(renderer.backend().find_by_key("row"), Some(row));
    assert_eq!(
        renderer.backend().drag_payload_of(row),
        Some(&DragPayload::new(FILE_PAYLOAD_TYPE, "changelog"))
    );
}

#[test]
fn removing_drag_declarations_patches_the_models_away() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(
            column([label("inert").with_key("note")])
                .on_file_drop(|_| {})
                .with_key("region"),
        )
        .unwrap();
    let region = renderer.backend().find_by_key("region").unwrap();
    assert!(renderer.backend().drop_target_of(region).is_some());

    renderer
        .render(column([label("inert").with_key("note")]).with_key("region"))
        .unwrap();

    assert!(renderer.backend().drop_target_of(region).is_none());
    let refused = renderer.backend().simulate_file_drop(
        region,
        [PathBuf::from("/tmp/first.txt")],
        DropPosition::new(0.0, 0.0),
    );
    assert!(refused.is_err());
}

#[test]
fn nonsensical_drag_declarations_are_rejected_before_native_mutation() {
    let cases: Vec<(Element, &str)> = vec![
        (
            list(
                "Files",
                [
                    list_row("readme", None, None, false, false, "readme", || {}).draggable_file(
                        FilePromise::new("nested/readme.txt", "public.plain-text", |_| Ok(())),
                    ),
                ],
            ),
            "bare file name",
        ),
        (
            list(
                "Files",
                [
                    list_row("readme", None, None, false, false, "readme", || {})
                        .draggable_file(FilePromise::new("", "public.plain-text", |_| Ok(()))),
                ],
            ),
            "file name is empty",
        ),
        (
            list(
                "Files",
                [
                    list_row("readme", None, None, false, false, "readme", || {})
                        .draggable_file(FilePromise::new("readme.txt", "", |_| Ok(()))),
                ],
            ),
            "content type is empty",
        ),
        (
            list(
                "Files",
                [
                    list_row("readme", None, None, false, false, "readme", || {})
                        .drag_payload(DragPayload::new("", "readme")),
                ],
            ),
            "payload type is empty",
        ),
        (
            list(
                "Files",
                [
                    list_row("readme", None, None, false, false, "readme", || {})
                        .drag_payload(DragPayload::new("demo.file", "")),
                ],
            ),
            "empty id",
        ),
        (
            column([]).on_drop_accepting(Vec::<String>::new(), |_| {}),
            "neither files nor any payload type",
        ),
        (
            column([]).on_drop_accepting(["demo.file", "demo.file"], |_| {}),
            "duplicated",
        ),
    ];
    for (tree, expected_fragment) in cases {
        let mut renderer = Renderer::new(HeadlessBackend::new());
        let error = renderer
            .render(tree)
            .expect_err("invalid drag declaration must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("invalid drag declaration") && message.contains(expected_fragment),
            "diagnostic '{message}' must name the violation '{expected_fragment}'"
        );
        assert!(renderer.backend().operations().is_empty());
    }
}
