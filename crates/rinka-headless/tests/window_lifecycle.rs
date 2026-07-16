//! Consumer-side proof of the runtime window lifecycle over the headless
//! window host: open, programmatic close, focus, live titles, geometry
//! events, and the complete close-interception protocol with its typed
//! errors.

use rinka_core::{
    Alert, Component, DialogButtonRole, DialogOutcome, Dispatch, Element, LastWindowClosedPolicy,
    PlatformServices, RenderError, Renderer, Size, ToolbarDisplay, UpdateContext, WindowContent,
    WindowError, WindowEvent, WindowId, WindowKind, WindowPosition, WindowRuntime, WindowSpec,
    button, column, label,
};
use rinka_headless::{
    CloseRequestOutcome, FakeDialogPresenter, HeadlessBackend, HeadlessWindowHost, WindowOperation,
};

/// A minimal multi-window shell: it can open a child window, close windows
/// programmatically, retitle itself from state, observe lifecycle events,
/// and intercept its own close behind a confirmation dialog when it holds
/// unsaved-ish state (`dirty`).
struct ShellComponent {
    id: WindowId,
    title: String,
    dirty: bool,
    observed: Vec<String>,
}

enum ShellMessage {
    OpenChild { child_dirty: bool },
    OpenDuplicate,
    CloseChild,
    Retitle(String),
    Observed(WindowEvent),
    CloseRequested,
    ConfirmClose,
    VetoClose,
}

impl ShellComponent {
    fn new(id: impl Into<String>, dirty: bool) -> Self {
        let id = WindowId::new(id);
        Self {
            title: format!("Shell {}", id.as_str()),
            id,
            dirty,
            observed: Vec::new(),
        }
    }
}

fn shell_spec(id: &str, dirty: bool) -> WindowSpec {
    WindowSpec {
        id: WindowId::new(id),
        title: format!("Shell {id}"),
        kind: WindowKind::Main,
        initial_size: Size::new(640.0, 480.0),
        minimum_size: Size::new(320.0, 240.0),
        toolbar: Vec::new(),
        toolbar_display: ToolbarDisplay::Automatic,
        content: WindowContent::component(ShellComponent::new(id, dirty)),
    }
}

impl Component for ShellComponent {
    type Message = ShellMessage;

    fn update(&mut self, message: Self::Message, context: &UpdateContext<Self::Message>) {
        match message {
            ShellMessage::OpenChild { child_dirty } => {
                context.windows().open(shell_spec("child", child_dirty));
            }
            ShellMessage::OpenDuplicate => {
                context
                    .windows()
                    .open(shell_spec(self.id.as_str(), self.dirty));
            }
            ShellMessage::CloseChild => context.windows().close(&WindowId::new("child")),
            ShellMessage::Retitle(title) => self.title = title,
            ShellMessage::Observed(event) => self.observed.push(match event {
                WindowEvent::Focused => "focused".to_owned(),
                WindowEvent::Resigned => "resigned".to_owned(),
                WindowEvent::Resized(size) => {
                    format!("resized {}x{}", size.width, size.height)
                }
                WindowEvent::Moved(position) => {
                    format!("moved {},{}", position.x, position.y)
                }
            }),
            ShellMessage::CloseRequested => {
                if self.dirty {
                    context.dialogs().alert(
                        Alert::new(
                            format!("Close “{}”?", self.title),
                            "Unsaved changes will be discarded.",
                        )
                        .button("Cancel", DialogButtonRole::Cancel, ShellMessage::VetoClose)
                        .button(
                            "Close",
                            DialogButtonRole::Destructive,
                            ShellMessage::ConfirmClose,
                        )
                        .default_button(0),
                    );
                } else {
                    context.windows().confirm_close(&self.id);
                }
            }
            ShellMessage::ConfirmClose => context.windows().confirm_close(&self.id),
            ShellMessage::VetoClose => context.windows().veto_close(&self.id),
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let open_child = dispatch.clone();
        let open_duplicate = dispatch.clone();
        let close_child = dispatch.clone();
        let retitle = dispatch.clone();
        let observe = dispatch.clone();
        let close_requested = dispatch.clone();
        let confirm = dispatch;
        let mut root = column([
            label(self.observed.join("|")).with_key("events"),
            button("Open Child", "Open the child window", move || {
                open_child.emit(ShellMessage::OpenChild { child_dirty: false });
            })
            .with_key("open-child"),
            button(
                "Open Duplicate",
                "Open this window's identity again",
                move || {
                    open_duplicate.emit(ShellMessage::OpenDuplicate);
                },
            )
            .with_key("open-duplicate"),
            button("Close Child", "Close the child window", move || {
                close_child.emit(ShellMessage::CloseChild);
            })
            .with_key("close-child"),
            button("Mark Edited", "Retitle this window from state", {
                let retitle = retitle.clone();
                let title = format!("{} — edited", self.title);
                move || retitle.emit(ShellMessage::Retitle(title.clone()))
            })
            .with_key("retitle"),
            button(
                "Confirm Close",
                "Answer a pending close request",
                move || {
                    confirm.emit(ShellMessage::ConfirmClose);
                },
            )
            .with_key("confirm-close"),
        ])
        .with_key("shell-root")
        .window_title(self.title.clone())
        .on_window_event(move |event| observe.emit(ShellMessage::Observed(event)));
        if self.dirty {
            root = root.on_close_request(move || {
                close_requested.emit(ShellMessage::CloseRequested);
            });
        }
        root
    }
}

fn id(text: &str) -> WindowId {
    WindowId::new(text)
}

#[test]
fn a_second_window_opens_at_runtime_from_a_component_message() {
    let host = HeadlessWindowHost::new();
    host.open(shell_spec("root", false)).expect("open root");
    assert_eq!(host.focused(), Some(id("root")));

    host.events_of(&id("root"), "open-child")
        .expect("open button mounted")
        .emit_activate();

    assert_eq!(host.open_ids(), vec![id("root"), id("child")]);
    assert_eq!(host.kind_of(&id("child")), Some(WindowKind::Main));
    // The new window took focus, and both transitions were delivered as
    // messages identifying their windows.
    assert_eq!(host.focused(), Some(id("child")));
    assert_eq!(
        host.label_text(&id("root"), "events").as_deref(),
        Some("focused|resigned")
    );
    assert_eq!(
        host.label_text(&id("child"), "events").as_deref(),
        Some("focused")
    );
    assert_eq!(
        host.operations(),
        vec![
            WindowOperation::Opened(id("root")),
            WindowOperation::Focused(id("root")),
            WindowOperation::Opened(id("child")),
            WindowOperation::Resigned(id("root")),
            WindowOperation::Focused(id("child")),
        ]
    );
    assert!(host.take_error(&id("root")).is_none());
}

#[test]
fn opening_an_already_open_identity_is_a_typed_error_not_a_focus() {
    let host = HeadlessWindowHost::new();
    host.open(shell_spec("root", false)).expect("open root");
    host.events_of(&id("root"), "open-child")
        .expect("open button mounted")
        .emit_activate();
    host.focus(&id("root")).expect("focus root");
    let operations_before = host.operations();

    // The child's identity is re-opened from the root's component.
    host.events_of(&id("root"), "open-duplicate")
        .expect("duplicate button mounted")
        .emit_activate();
    // (open-duplicate opens the *root's* identity, which is open.)
    assert!(matches!(
        host.take_error(&id("root")),
        Some(RenderError::Window(WindowError::AlreadyOpen { id })) if id.as_str() == "root"
    ));
    // The window set and focus are untouched: no new window, no focus steal.
    assert_eq!(host.open_ids(), vec![id("root"), id("child")]);
    assert_eq!(host.focused(), Some(id("root")));
    assert_eq!(host.operations(), operations_before);
}

#[test]
fn a_window_closes_programmatically_from_a_component_message() {
    let host = HeadlessWindowHost::new();
    host.open(shell_spec("root", false)).expect("open root");
    host.events_of(&id("root"), "open-child")
        .expect("open button mounted")
        .emit_activate();
    assert_eq!(host.focused(), Some(id("child")));

    host.events_of(&id("root"), "close-child")
        .expect("close button mounted")
        .emit_activate();

    assert_eq!(host.open_ids(), vec![id("root")]);
    // Focus returned to the surviving window — the signal a consumer's
    // root-window promotion listens for.
    assert_eq!(host.focused(), Some(id("root")));
    assert!(
        host.label_text(&id("root"), "events")
            .expect("events label")
            .ends_with("focused")
    );
    assert!(host.take_error(&id("root")).is_none());
    let operations = host.operations();
    assert!(operations.contains(&WindowOperation::Closed(id("child"))));
    assert!(!operations.contains(&WindowOperation::CloseRequested(id("child"))));
}

#[test]
fn closing_an_unknown_identity_is_a_typed_error() {
    let host = HeadlessWindowHost::new();
    host.open(shell_spec("root", false)).expect("open root");

    host.events_of(&id("root"), "close-child")
        .expect("close button mounted")
        .emit_activate();

    assert!(matches!(
        host.take_error(&id("root")),
        Some(RenderError::Window(WindowError::NotOpen { id })) if id.as_str() == "child"
    ));
}

#[test]
fn the_title_reconciles_from_state_without_rebuilding_the_window() {
    let host = HeadlessWindowHost::new();
    host.open(shell_spec("root", false)).expect("open root");
    assert_eq!(host.title_of(&id("root")).as_deref(), Some("Shell root"));

    host.events_of(&id("root"), "retitle")
        .expect("retitle button mounted")
        .emit_activate();

    assert_eq!(
        host.title_of(&id("root")).as_deref(),
        Some("Shell root — edited")
    );
    let operations = host.operations();
    assert!(operations.contains(&WindowOperation::TitleChanged(
        id("root"),
        "Shell root — edited".to_owned()
    )));
    // One Opened operation, ever: the window was never torn down.
    let opened = operations
        .iter()
        .filter(|operation| matches!(operation, WindowOperation::Opened(_)))
        .count();
    assert_eq!(opened, 1);
}

#[test]
fn geometry_events_and_model_follow_the_service_setters() {
    let host = HeadlessWindowHost::new();
    host.open(shell_spec("root", false)).expect("open root");

    host.set_content_size(&id("root"), Size::new(1024.0, 768.0))
        .expect("resize");
    host.set_position(&id("root"), WindowPosition::new(120.0, 80.0))
        .expect("move");

    assert_eq!(
        host.content_size_of(&id("root")),
        Some(Size::new(1024.0, 768.0))
    );
    assert_eq!(
        host.position_of(&id("root")),
        Some(WindowPosition::new(120.0, 80.0))
    );
    assert_eq!(
        host.label_text(&id("root"), "events").as_deref(),
        Some("focused|resized 1024x768|moved 120,80")
    );
    let operations = host.operations();
    assert!(operations.contains(&WindowOperation::Resized(
        id("root"),
        Size::new(1024.0, 768.0)
    )));
    assert!(operations.contains(&WindowOperation::Moved(
        id("root"),
        WindowPosition::new(120.0, 80.0)
    )));
}

#[test]
fn a_close_request_with_no_declared_handler_closes_natively() {
    let host = HeadlessWindowHost::new();
    host.open(shell_spec("root", false)).expect("open root");

    let outcome = host.request_close(&id("root")).expect("request close");

    assert_eq!(outcome, CloseRequestOutcome::ClosedImmediately);
    assert!(host.open_ids().is_empty());
    let operations = host.operations();
    assert!(!operations.contains(&WindowOperation::CloseRequested(id("root"))));
    assert_eq!(operations.last(), Some(&WindowOperation::AllWindowsClosed));
}

#[test]
fn a_dirty_window_vetoes_once_then_confirms_through_the_dialog_surface() {
    let presenter = FakeDialogPresenter::new();
    let dialog_presenter = presenter.clone();
    let host = HeadlessWindowHost::new().with_services(move || {
        PlatformServices::default().with_dialog_service(dialog_presenter.clone())
    });
    host.open(shell_spec("root", true)).expect("open root");

    // First gesture: deferred, confirm sheet presented, token pending.
    assert_eq!(
        host.request_close(&id("root")).expect("request close"),
        CloseRequestOutcome::Deferred
    );
    assert_eq!(presenter.presented_count(), 1);
    assert_eq!(host.pending_close_ids(), vec![id("root")]);
    assert!(host.is_open(&id("root")));

    // A second gesture while pending is absorbed: no second sheet.
    assert_eq!(
        host.request_close(&id("root")).expect("request close"),
        CloseRequestOutcome::AlreadyPending
    );
    assert_eq!(presenter.presented_count(), 1);

    // The user cancels: the component vetoes, the window stays open.
    assert!(presenter.deliver(0, DialogOutcome::ButtonChosen(0)));
    assert!(host.pending_close_ids().is_empty());
    assert!(host.is_open(&id("root")));

    // The user tries again and confirms: only now does the window close.
    assert_eq!(
        host.request_close(&id("root")).expect("request close"),
        CloseRequestOutcome::Deferred
    );
    assert_eq!(presenter.presented_count(), 2);
    assert!(presenter.deliver(1, DialogOutcome::ButtonChosen(1)));
    assert!(!host.is_open(&id("root")));

    assert_eq!(
        host.operations(),
        vec![
            WindowOperation::Opened(id("root")),
            WindowOperation::Focused(id("root")),
            WindowOperation::CloseRequested(id("root")),
            WindowOperation::CloseVetoed(id("root")),
            WindowOperation::CloseRequested(id("root")),
            WindowOperation::CloseConfirmed(id("root")),
            WindowOperation::Closed(id("root")),
            WindowOperation::AllWindowsClosed,
        ]
    );
}

#[test]
fn answering_a_close_without_a_pending_token_is_a_typed_error() {
    let host = HeadlessWindowHost::new();
    host.open(shell_spec("root", false)).expect("open root");

    host.events_of(&id("root"), "confirm-close")
        .expect("confirm button mounted")
        .emit_activate();

    assert!(matches!(
        host.take_error(&id("root")),
        Some(RenderError::Window(WindowError::NoPendingClose { id })) if id.as_str() == "root"
    ));
    assert!(host.is_open(&id("root")));
}

#[test]
fn a_native_focus_change_delivers_resigned_and_focused_messages() {
    let host = HeadlessWindowHost::new();
    host.open(shell_spec("root", false)).expect("open root");
    host.events_of(&id("root"), "open-child")
        .expect("open button mounted")
        .emit_activate();

    host.focus(&id("root")).expect("focus root");

    assert_eq!(host.focused(), Some(id("root")));
    assert_eq!(
        host.label_text(&id("root"), "events").as_deref(),
        Some("focused|resigned|focused")
    );
    assert_eq!(
        host.label_text(&id("child"), "events").as_deref(),
        Some("focused|resigned")
    );
    assert!(matches!(
        host.focus(&id("ghost")),
        Err(WindowError::NotOpen { .. })
    ));
}

#[test]
fn a_component_without_a_window_host_observes_the_typed_no_host_error() {
    let runtime = WindowRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        WindowContent::component(ShellComponent::new("orphan", false)),
        PlatformServices::default(),
    )
    .expect("mount");
    let events = runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        let handle = backend.find_by_key("open-child").expect("button mounted");
        backend.events_of(handle).expect("button events")
    });

    events.emit_activate();

    assert!(matches!(
        runtime.take_error(),
        Some(RenderError::Window(WindowError::NoHost))
    ));
}

#[test]
fn window_declarations_below_the_content_root_are_rejected() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let tree = column([column([label("inner")])
        .with_key("inner")
        .window_title("Nested")])
    .with_key("root");

    assert!(matches!(
        renderer.render(tree),
        Err(RenderError::Tree(rinka_core::TreeError::InvalidWindowDeclaration { path, .. }))
            if path == "root/inner"
    ));
}

#[test]
fn the_last_window_closed_policy_is_a_declared_application_fact() {
    // The policy is declarative data each platform host maps onto its own
    // lifecycle; the default is the platform's own convention.
    assert_eq!(
        LastWindowClosedPolicy::default(),
        LastWindowClosedPolicy::PlatformDefault
    );
}
