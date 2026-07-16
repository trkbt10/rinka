//! Clipboard service round-trips driven entirely from `Component::update`.

use rinka_core::{
    AppRuntime, Clipboard, ClipboardError, Component, Dispatch, Element, PlatformServices, Props,
    Renderer, UpdateContext, WindowContent, WindowRuntime, button, column, label,
};
use rinka_headless::{FakeClipboard, HeadlessBackend};
use std::cell::RefCell;
use std::rc::Rc;

const CJK_MULTILINE: &str = "日本語\nline two";

/// Collects a read outcome delivered through the `'static` completion.
fn collect_read(clipboard: &rinka_core::Clipboard) -> Result<Option<String>, ClipboardError> {
    let delivered = Rc::new(RefCell::new(None));
    let sink = delivered.clone();
    clipboard.read_text(move |result| *sink.borrow_mut() = Some(result));
    delivered
        .borrow_mut()
        .take()
        .expect("the fake delivers synchronously")
}

#[test]
fn the_fake_clipboard_round_trips_cjk_and_multiline_text() {
    let fake = FakeClipboard::new();
    let handle = fake.handle();
    for text in [CJK_MULTILINE, "first\nsecond\nthird", "ascii"] {
        handle.write_text(text).expect("fake write");
        assert_eq!(collect_read(&handle), Ok(Some(text.to_owned())));
        assert_eq!(fake.text().as_deref(), Some(text));
    }
}

#[test]
fn an_empty_fake_clipboard_reads_as_no_text() {
    assert_eq!(collect_read(&FakeClipboard::new().handle()), Ok(None));
}

/// Component whose copy and paste transitions run through the service.
struct CopyPasteComponent {
    draft: &'static str,
    note: String,
}

enum CopyPasteMessage {
    Copy,
    Paste,
    ClipboardRead(Result<Option<String>, ClipboardError>),
}

impl Component for CopyPasteComponent {
    type Message = CopyPasteMessage;

    fn update(&mut self, message: Self::Message, context: &UpdateContext<Self::Message>) {
        match message {
            CopyPasteMessage::Copy => match context.clipboard().write_text(self.draft) {
                Ok(()) => self.note = "copied".to_owned(),
                Err(error) => self.note = format!("copy failed: {error}"),
            },
            CopyPasteMessage::Paste => {
                let dispatch = context.dispatch().clone();
                context.clipboard().read_text(move |result| {
                    dispatch.emit(CopyPasteMessage::ClipboardRead(result));
                });
            }
            CopyPasteMessage::ClipboardRead(result) => {
                self.note = match result {
                    Ok(Some(text)) => format!("pasted: {text}"),
                    Ok(None) => "clipboard empty".to_owned(),
                    Err(error) => format!("paste failed: {error}"),
                };
            }
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let copy = dispatch.clone();
        let paste = dispatch;
        column([
            label(self.note.as_str()).with_key("note"),
            button("Copy", "Copy", move || copy.emit(CopyPasteMessage::Copy)).with_key("copy"),
            button("Paste", "Paste", move || {
                paste.emit(CopyPasteMessage::Paste)
            })
            .with_key("paste"),
        ])
        .with_key("root")
    }
}

fn note_text(backend: &HeadlessBackend) -> String {
    let handle = backend.find_by_key("note").expect("mounted note label");
    match backend.props_of(handle).expect("note props") {
        Props::Label { text, .. } => text.clone(),
        other => panic!("note is not a label: {other:?}"),
    }
}

#[test]
fn update_driven_copy_reaches_the_service_through_the_app_runtime() {
    let fake = FakeClipboard::new();
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        CopyPasteComponent {
            draft: CJK_MULTILINE,
            note: "idle".to_owned(),
        },
        PlatformServices::new(fake.handle()),
    )
    .expect("initial render");

    let copy = runtime.with_renderer(|renderer| {
        let root = renderer.mounted().expect("mounted root");
        root.children()[1].events().clone()
    });
    copy.emit_activate();

    assert_eq!(fake.text().as_deref(), Some(CJK_MULTILINE));
    runtime.with_component(|component| assert_eq!(component.note, "copied"));
    assert!(runtime.take_error().is_none());
}

#[test]
fn a_synchronous_read_completion_is_queued_and_applied_after_the_update() {
    // The fake delivers inside `read_text`, i.e. while `update` is still on
    // the stack; the runtime must queue the mapped message instead of
    // re-entering the component.
    let fake = FakeClipboard::new();
    fake.handle().write_text(CJK_MULTILINE).expect("preload");
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        CopyPasteComponent {
            draft: "",
            note: "idle".to_owned(),
        },
        PlatformServices::new(fake.handle()),
    )
    .expect("initial render");

    let paste = runtime.with_renderer(|renderer| {
        let root = renderer.mounted().expect("mounted root");
        root.children()[2].events().clone()
    });
    paste.emit_activate();

    runtime.with_component(|component| {
        assert_eq!(component.note, format!("pasted: {CJK_MULTILINE}"));
    });
    assert!(runtime.take_error().is_none());
}

#[test]
fn window_content_components_share_the_same_queued_delivery() {
    let fake = FakeClipboard::new();
    fake.handle()
        .write_text("窓 line\nsecond")
        .expect("preload");
    let runtime = WindowRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        WindowContent::component(CopyPasteComponent {
            draft: "written by window content",
            note: "idle".to_owned(),
        }),
        PlatformServices::new(fake.handle()),
    )
    .expect("initial render");

    let (copy, paste) = runtime.with_renderer(|renderer| {
        let root = renderer.mounted().expect("mounted root");
        (
            root.children()[1].events().clone(),
            root.children()[2].events().clone(),
        )
    });

    paste.emit_activate();
    runtime.with_renderer(|renderer| {
        assert_eq!(note_text(renderer.backend()), "pasted: 窓 line\nsecond");
    });

    copy.emit_activate();
    assert_eq!(fake.text().as_deref(), Some("written by window content"));
    runtime.with_renderer(|renderer| {
        assert_eq!(note_text(renderer.backend()), "copied");
    });
    assert!(runtime.take_error().is_none());
}

#[test]
fn a_host_without_a_clipboard_surfaces_the_typed_rejection_to_update() {
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        CopyPasteComponent {
            draft: "never stored",
            note: "idle".to_owned(),
        },
        PlatformServices::new(Clipboard::unsupported("Probe Host")),
    )
    .expect("initial render");

    let (copy, paste) = runtime.with_renderer(|renderer| {
        let root = renderer.mounted().expect("mounted root");
        (
            root.children()[1].events().clone(),
            root.children()[2].events().clone(),
        )
    });

    copy.emit_activate();
    runtime.with_component(|component| {
        assert_eq!(
            component.note,
            "copy failed: Probe Host does not provide a clipboard service"
        );
    });

    paste.emit_activate();
    runtime.with_component(|component| {
        assert_eq!(
            component.note,
            "paste failed: Probe Host does not provide a clipboard service"
        );
    });
    assert!(runtime.take_error().is_none());
}
