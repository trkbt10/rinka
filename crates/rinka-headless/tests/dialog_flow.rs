//! Consumer tests for the dialog service channel and the fake presenter.

use rinka_core::{
    Alert, AppRuntime, Component, DialogButton, DialogButtonRole, DialogDescription, DialogError,
    DialogOutcome, DialogRequest, DialogService, Dispatch, Element, MountedNode,
    OpenPanelDescription, PlatformServices, ProjectedHandle, Props, RenderError, Renderer,
    SavePanelDescription, UpdateContext, WindowContent, WindowProjection, WindowRuntime, button,
    column, label,
};
use rinka_headless::{FakeDialogPresenter, HeadlessBackend};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

/// A document whose close must be confirmed through Save / Discard / Cancel.
struct DirtyDocument {
    state: &'static str,
}

#[derive(Clone, Copy)]
enum DocumentMessage {
    RequestClose,
    Save,
    Discard,
    CancelClose,
}

impl Component for DirtyDocument {
    type Message = DocumentMessage;

    fn update(&mut self, message: Self::Message, context: &UpdateContext<Self::Message>) {
        match message {
            DocumentMessage::RequestClose => {
                context.dialogs().alert(
                    Alert::new("Save changes?", "Unsaved edits will be lost.")
                        .button("Save", DialogButtonRole::Standard, DocumentMessage::Save)
                        .button(
                            "Discard",
                            DialogButtonRole::Destructive,
                            DocumentMessage::Discard,
                        )
                        .button(
                            "Cancel",
                            DialogButtonRole::Cancel,
                            DocumentMessage::CancelClose,
                        )
                        .default_button(0),
                );
            }
            DocumentMessage::Save => self.state = "saved",
            DocumentMessage::Discard => self.state = "discarded",
            DocumentMessage::CancelClose => self.state = "editing",
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        column([
            label(format!("state={}", self.state)).with_key("state"),
            button("Close", "Close the document", move || {
                dispatch.emit(DocumentMessage::RequestClose);
            })
            .with_key("close"),
        ])
        .with_key("document")
    }
}

fn label_text(runtime: &AppRuntime<HeadlessBackend, DirtyDocument>, key: &str) -> String {
    runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        let handle = backend.find_by_key(key).expect("label is mounted");
        match backend.props_of(handle) {
            Some(Props::Label { text, .. }) => text.clone(),
            other => panic!("expected label props, found {other:?}"),
        }
    })
}

fn activate(runtime: &AppRuntime<HeadlessBackend, DirtyDocument>, key: &str) {
    let events = runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        let handle = backend.find_by_key(key).expect("control is mounted");
        backend.events_of(handle).expect("control has events")
    });
    events.emit_activate();
}

#[test]
fn a_confirm_flow_round_trips_the_chosen_button_message() {
    let presenter = FakeDialogPresenter::new();
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        DirtyDocument { state: "editing" },
        PlatformServices::default().with_dialog_service(presenter.clone()),
    )
    .expect("initial mount");

    activate(&runtime, "close");

    assert_eq!(presenter.presented_count(), 1);
    let Some(DialogDescription::Alert(alert)) = presenter.description(0) else {
        panic!("expected an alert presentation");
    };
    assert_eq!(alert.title, "Save changes?");
    assert_eq!(
        alert.buttons,
        vec![
            DialogButton::new("Save", DialogButtonRole::Standard),
            DialogButton::new("Discard", DialogButtonRole::Destructive),
            DialogButton::new("Cancel", DialogButtonRole::Cancel),
        ]
    );
    assert_eq!(alert.default_button, Some(0));

    assert!(presenter.deliver(0, DialogOutcome::ButtonChosen(1)));
    assert_eq!(
        runtime.with_component(|document| document.state),
        "discarded"
    );
    assert_eq!(label_text(&runtime, "state"), "state=discarded");
    assert!(runtime.take_error().is_none());

    // Native completion handlers run exactly once; a replayed outcome is
    // rejected and changes nothing.
    assert!(!presenter.deliver(0, DialogOutcome::ButtonChosen(0)));
    assert_eq!(
        runtime.with_component(|document| document.state),
        "discarded"
    );
}

struct DestructiveDefault;

impl Component for DestructiveDefault {
    type Message = ();

    fn update(&mut self, (): Self::Message, context: &UpdateContext<Self::Message>) {
        context.dialogs().alert(
            Alert::new("Delete?", "This cannot be undone.")
                .button("Delete", DialogButtonRole::Destructive, ())
                .button("Cancel", DialogButtonRole::Cancel, ())
                .default_button(0),
        );
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        column([button("Delete", "Delete the item", move || dispatch.emit(())).with_key("delete")])
            .with_key("root")
    }
}

#[test]
fn a_destructive_return_key_default_is_rejected_with_a_typed_error() {
    let presenter = FakeDialogPresenter::new();
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        DestructiveDefault,
        PlatformServices::default().with_dialog_service(presenter.clone()),
    )
    .expect("initial mount");

    let events = runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        let handle = backend.find_by_key("delete").expect("button is mounted");
        backend.events_of(handle).expect("button has events")
    });
    events.emit_activate();

    assert_eq!(presenter.presented_count(), 0);
    assert!(matches!(
        runtime.take_error(),
        Some(RenderError::Dialog(DialogError::DestructiveDefault {
            index: 0,
            ..
        }))
    ));
}

#[test]
fn a_dialog_request_without_a_service_is_a_typed_error_and_still_renders() {
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        DirtyDocument { state: "editing" },
        PlatformServices::default(),
    )
    .expect("initial mount");

    activate(&runtime, "close");

    assert!(matches!(
        runtime.take_error(),
        Some(RenderError::Dialog(DialogError::NoPresenter))
    ));
    // The update itself reconciled normally; only the presentation failed.
    assert_eq!(label_text(&runtime, "state"), "state=editing");
}

/// A picker exercising both panels and structural churn during the request.
struct ProjectPicker {
    opened: Vec<PathBuf>,
    saved_to: Option<PathBuf>,
    requests: u32,
}

enum PickerMessage {
    RequestOpen,
    RequestSave,
    Opened(Vec<PathBuf>),
    SavedTo(PathBuf),
}

impl Component for ProjectPicker {
    type Message = PickerMessage;

    fn update(&mut self, message: Self::Message, context: &UpdateContext<Self::Message>) {
        match message {
            PickerMessage::RequestOpen => {
                // Structural churn in the same update that requests the
                // dialog: the request must not corrupt this reconciliation.
                self.requests += 1;
                context.dialogs().open_panel(
                    OpenPanelDescription {
                        title: Some("Choose project files".to_owned()),
                        choose_files: true,
                        choose_directories: false,
                        allows_multiple: true,
                        starting_directory: Some(PathBuf::from("/tmp")),
                    },
                    |outcome| match outcome {
                        DialogOutcome::PathsChosen(paths) => Some(PickerMessage::Opened(paths)),
                        _ => None,
                    },
                );
            }
            PickerMessage::RequestSave => {
                self.requests += 1;
                context.dialogs().save_panel(
                    SavePanelDescription {
                        title: None,
                        suggested_filename: Some("export.json".to_owned()),
                        starting_directory: None,
                    },
                    |outcome| match outcome {
                        DialogOutcome::SavePathChosen(path) => Some(PickerMessage::SavedTo(path)),
                        _ => None,
                    },
                );
            }
            PickerMessage::Opened(paths) => self.opened = paths,
            PickerMessage::SavedTo(path) => self.saved_to = Some(path),
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let open = dispatch.clone();
        let mut children = vec![
            label(format!("requests={}", self.requests)).with_key("requests"),
            label(format!("opened={}", self.opened.len())).with_key("opened"),
            button("Open", "Open project files", move || {
                open.emit(PickerMessage::RequestOpen);
            })
            .with_key("open"),
            button("Save", "Save the export", move || {
                dispatch.emit(PickerMessage::RequestSave);
            })
            .with_key("save"),
        ];
        // Keyed rows appear per request, so each dialog-raising update also
        // rewrites sibling structure.
        for index in 0..self.requests {
            children.push(label(format!("request-{index}")).with_key(format!("request-{index}")));
        }
        if let Some(path) = &self.saved_to {
            children.push(label(format!("saved={}", path.display())).with_key("saved"));
        }
        column(children).with_key("picker")
    }
}

fn projected_node<'a>(
    node: &'a MountedNode<ProjectedHandle>,
    key: &str,
) -> Option<&'a MountedNode<ProjectedHandle>> {
    if node
        .element()
        .key()
        .is_some_and(|candidate| candidate.as_str() == key)
    {
        return Some(node);
    }
    node.children()
        .iter()
        .find_map(|child| projected_node(child, key))
}

fn projected_label_text(projection: &WindowProjection, key: &str) -> String {
    projection
        .with_root(|root| {
            let node = projected_node(root, key).expect("label is mounted");
            match node.element().props() {
                Props::Label { text, .. } => text.clone(),
                other => panic!("expected label props, found {other:?}"),
            }
        })
        .expect("projection has a root")
}

#[test]
fn open_and_save_panels_round_trip_paths_as_messages() {
    // The projection is the same mounted surface the WinUI host consumes, so
    // this proves the dialog channel through the adapter-facing contract.
    let presenter = FakeDialogPresenter::new();
    let projection = WindowProjection::mount(
        WindowContent::component(ProjectPicker {
            opened: Vec::new(),
            saved_to: None,
            requests: 0,
        }),
        PlatformServices::default().with_dialog_service(presenter.clone()),
    )
    .expect("initial mount");
    let reconciled = Rc::new(Cell::new(0_u32));
    let observed = reconciled.clone();
    projection.set_reconciled_handler(move || observed.set(observed.get() + 1));

    let open_events = projection
        .with_root(|root| {
            projected_node(root, "open")
                .expect("open button")
                .events()
                .clone()
        })
        .expect("projection has a root");
    open_events.emit_activate();

    // The request's own structural churn reconciled cleanly.
    assert_eq!(reconciled.get(), 1);
    assert_eq!(presenter.presented_count(), 1);
    let Some(DialogDescription::OpenPanel(panel)) = presenter.description(0) else {
        panic!("expected an open panel presentation");
    };
    assert!(panel.choose_files);
    assert!(panel.allows_multiple);
    assert_eq!(panel.starting_directory, Some(PathBuf::from("/tmp")));

    presenter.deliver(
        0,
        DialogOutcome::PathsChosen(vec![
            PathBuf::from("/tmp/alpha.txt"),
            PathBuf::from("/tmp/beta.txt"),
        ]),
    );
    assert_eq!(projected_label_text(&projection, "opened"), "opened=2");

    let save_events = projection
        .with_root(|root| {
            projected_node(root, "save")
                .expect("save button")
                .events()
                .clone()
        })
        .expect("projection has a root");
    save_events.emit_activate();

    assert_eq!(presenter.presented_count(), 2);
    let Some(DialogDescription::SavePanel(panel)) = presenter.description(1) else {
        panic!("expected a save panel presentation");
    };
    assert_eq!(panel.suggested_filename.as_deref(), Some("export.json"));

    presenter.deliver(
        1,
        DialogOutcome::SavePathChosen(PathBuf::from("/tmp/export.json")),
    );
    assert_eq!(
        projected_label_text(&projection, "saved"),
        "saved=/tmp/export.json"
    );
    assert!(projection.take_error().is_none());
}

#[test]
fn a_cancelled_panel_delivers_no_message_and_keeps_the_tree_consistent() {
    let presenter = FakeDialogPresenter::new();
    let runtime = WindowRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        WindowContent::component(ProjectPicker {
            opened: Vec::new(),
            saved_to: None,
            requests: 0,
        }),
        PlatformServices::default().with_dialog_service(presenter.clone()),
    )
    .expect("initial mount");

    let events = runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        let handle = backend.find_by_key("open").expect("open button");
        backend.events_of(handle).expect("open events")
    });
    events.emit_activate();
    presenter.deliver(0, DialogOutcome::Cancelled);

    runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        let handle = backend.find_by_key("opened").expect("opened label");
        assert!(matches!(
            backend.props_of(handle),
            Some(Props::Label { text, .. }) if text == "opened=0"
        ));
        assert!(backend.find_by_key("request-0").is_some());
    });
    assert!(runtime.take_error().is_none());
}

/// A wizard whose first answer immediately raises a second confirmation,
/// proving a synchronously answered dialog cannot re-enter the component.
struct ChainedWizard {
    stage: &'static str,
}

#[derive(Clone, Copy)]
enum WizardMessage {
    Start,
    FirstConfirmed,
    SecondConfirmed,
}

impl Component for ChainedWizard {
    type Message = WizardMessage;

    fn update(&mut self, message: Self::Message, context: &UpdateContext<Self::Message>) {
        match message {
            WizardMessage::Start => {
                self.stage = "asking-first";
                context.dialogs().alert(
                    Alert::new("First?", "Step one.")
                        .button(
                            "Continue",
                            DialogButtonRole::Standard,
                            WizardMessage::FirstConfirmed,
                        )
                        .default_button(0),
                );
            }
            WizardMessage::FirstConfirmed => {
                self.stage = "asking-second";
                context.dialogs().alert(
                    Alert::new("Second?", "Step two.")
                        .button(
                            "Finish",
                            DialogButtonRole::Standard,
                            WizardMessage::SecondConfirmed,
                        )
                        .default_button(0),
                );
            }
            WizardMessage::SecondConfirmed => self.stage = "done",
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        column([
            label(format!("stage={}", self.stage)).with_key("stage"),
            button("Start", "Start the wizard", move || {
                dispatch.emit(WizardMessage::Start);
            })
            .with_key("start"),
        ])
        .with_key("wizard")
    }
}

/// A service answering every alert immediately from inside `present` — the
/// most hostile re-entrancy a platform host could exhibit.
struct ImmediateAnswer {
    answered: Rc<Cell<u32>>,
}

impl DialogService for ImmediateAnswer {
    fn present(&self, request: DialogRequest) {
        self.answered.set(self.answered.get() + 1);
        let (_, responder) = request.into_parts();
        responder.deliver(DialogOutcome::ButtonChosen(0));
    }
}

#[test]
fn a_synchronously_answered_dialog_chains_without_reentering_the_component() {
    let answered = Rc::new(Cell::new(0_u32));
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        ChainedWizard { stage: "idle" },
        PlatformServices::default().with_dialog_service(ImmediateAnswer {
            answered: answered.clone(),
        }),
    )
    .expect("initial mount");

    let events = runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        let handle = backend.find_by_key("start").expect("start button");
        backend.events_of(handle).expect("start events")
    });
    events.emit_activate();

    assert_eq!(answered.get(), 2);
    assert_eq!(runtime.with_component(|wizard| wizard.stage), "done");
    runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        let handle = backend.find_by_key("stage").expect("stage label");
        assert!(matches!(
            backend.props_of(handle),
            Some(Props::Label { text, .. }) if text == "stage=done"
        ));
    });
    assert!(runtime.take_error().is_none());
}

/// The same hostile synchronous answer through the queued window-content
/// dispatch: `WindowContent::component` must apply the chained messages
/// after the running update returns, never re-entering the component.
#[test]
fn window_content_survives_a_synchronous_answer_mid_update() {
    let answered = Rc::new(Cell::new(0_u32));
    let runtime = WindowRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        WindowContent::component(ChainedWizard { stage: "idle" }),
        PlatformServices::default().with_dialog_service(ImmediateAnswer {
            answered: answered.clone(),
        }),
    )
    .expect("initial mount");

    let events = runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        let handle = backend.find_by_key("start").expect("start button");
        backend.events_of(handle).expect("start events")
    });
    events.emit_activate();

    assert_eq!(answered.get(), 2);
    runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        let handle = backend.find_by_key("stage").expect("stage label");
        assert!(matches!(
            backend.props_of(handle),
            Some(Props::Label { text, .. }) if text == "stage=done"
        ));
    });
    assert!(runtime.take_error().is_none());
}
