//! Consumer-level contracts of canvas keyboard focus, raw keys, and IME.

use rinka_core::{
    AppRuntime, CanvasColor, CanvasPoint, CanvasRect, CanvasSize, Component, Dispatch, DrawCommand,
    DrawScene, Element, ImeEvent, KeyEvent, KeyIdentity, Modifiers, PlatformServices, PreeditCaret,
    Props, Renderer, UpdateContext, canvas,
};
use rinka_headless::{HeadlessBackend, SyntheticTextInput};
use std::cell::RefCell;
use std::rc::Rc;

/// Message vocabulary of the echo component, mirroring a terminal's intake.
#[derive(Clone, Debug, PartialEq, Eq)]
enum EchoMessage {
    Focus(bool),
    Key(KeyEvent),
    Ime(ImeEvent),
}

/// Minimal text-input consumer: echoes committed text and raw-key text into
/// one line, renders the preedit distinctly, and declares the caret rect.
struct EchoComponent {
    focused: bool,
    echo: String,
    preedit: Option<(String, Option<PreeditCaret>)>,
    received: Rc<RefCell<Vec<EchoMessage>>>,
}

const ECHO_FONT_SIZE: f64 = 13.0;
/// Synthetic per-glyph advance mirroring the headless monospace model.
const ECHO_GLYPH_WIDTH: f64 = ECHO_FONT_SIZE * 0.6;

impl EchoComponent {
    fn new(received: Rc<RefCell<Vec<EchoMessage>>>) -> Self {
        Self {
            focused: false,
            echo: String::new(),
            preedit: None,
            received,
        }
    }

    fn caret_rect(&self) -> CanvasRect {
        let preedit_chars = self
            .preedit
            .as_ref()
            .map_or(0, |(text, _)| text.chars().count());
        let columns = self.echo.chars().count() + preedit_chars;
        CanvasRect::new(
            columns as f64 * ECHO_GLYPH_WIDTH,
            0.0,
            1.0,
            ECHO_FONT_SIZE * 1.5,
        )
    }
}

impl Component for EchoComponent {
    type Message = EchoMessage;

    fn update(&mut self, message: Self::Message, _context: &UpdateContext<Self::Message>) {
        self.received.borrow_mut().push(message.clone());
        match message {
            EchoMessage::Focus(focused) => self.focused = focused,
            EchoMessage::Key(event) => {
                if event.key == Some(KeyIdentity::BACKSPACE) {
                    self.echo.pop();
                } else if let Some(text) = event.text {
                    self.echo.push_str(&text);
                }
            }
            EchoMessage::Ime(ImeEvent::Preedit { text, caret }) => {
                self.preedit = Some((text, caret));
            }
            EchoMessage::Ime(ImeEvent::Commit { text }) => {
                self.echo.push_str(&text);
                self.preedit = None;
            }
            EchoMessage::Ime(ImeEvent::Cancel) => self.preedit = None,
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let mut scene = DrawScene::new();
        scene.glyph_run(
            CanvasPoint::new(0.0, 0.0),
            self.echo.clone(),
            ECHO_FONT_SIZE,
            CanvasColor::rgb(0.9, 0.9, 0.9),
        );
        if let Some((preedit, _)) = &self.preedit {
            // The app owns preedit presentation: distinct color at the caret.
            scene.glyph_run(
                self.caret_rect().origin,
                preedit.clone(),
                ECHO_FONT_SIZE,
                CanvasColor::rgb(1.0, 0.8, 0.2),
            );
        }
        let focus_dispatch = dispatch.clone();
        let key_dispatch = dispatch.clone();
        canvas(CanvasSize::new(480.0, 24.0), scene, "Echo terminal line")
            .accepts_input(true)
            .ime_caret(self.caret_rect())
            .on_focus_change(move |focused| focus_dispatch.emit(EchoMessage::Focus(focused)))
            .on_key(move |event| key_dispatch.emit(EchoMessage::Key(event)))
            .on_ime(move |event| dispatch.emit(EchoMessage::Ime(event)))
            .with_key("echo")
    }
}

fn key(identity: Option<KeyIdentity>, modifiers: Modifiers, text: Option<&str>) -> KeyEvent {
    KeyEvent {
        key: identity,
        modifiers,
        text: text.map(str::to_owned),
        repeat: false,
    }
}

fn mounted_echo(runtime: &AppRuntime<HeadlessBackend, EchoComponent>) -> SyntheticTextInput {
    let handle = runtime
        .with_renderer(|renderer| renderer.backend().find_by_key("echo"))
        .expect("echo canvas is mounted");
    let events = runtime
        .with_renderer(|renderer| renderer.backend().events_of(handle))
        .expect("echo canvas has stable event bindings");
    SyntheticTextInput::new(events)
}

#[test]
fn the_full_key_and_ime_stream_arrives_in_order_with_modifiers_and_repeat() {
    let received = Rc::new(RefCell::new(Vec::new()));
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        EchoComponent::new(received.clone()),
        PlatformServices::default(),
    )
    .unwrap();
    let input = mounted_echo(&runtime);

    // Focus-in → plain keys → chorded key → repeated arrow → composition
    // that commits → composition that cancels → focus-out.
    input.focus();
    input.key(key(KeyIdentity::letter('h'), Modifiers::NONE, Some("h")));
    input.key(key(KeyIdentity::letter('i'), Modifiers::NONE, Some("i")));
    input.key(key(
        KeyIdentity::letter('c'),
        Modifiers::NONE.with_control(),
        None,
    ));
    input.key(KeyEvent {
        key: Some(KeyIdentity::ARROW_RIGHT),
        modifiers: Modifiers::NONE,
        text: None,
        repeat: true,
    });
    input.compose_and_commit(
        &[
            ("に", Some(PreeditCaret::new(1, 1))),
            ("にほ", Some(PreeditCaret::new(2, 2))),
            ("にほんご", Some(PreeditCaret::new(4, 4))),
        ],
        "日本語",
    );
    input.compose_and_cancel(&[("か", Some(PreeditCaret::new(1, 1)))]);
    input.blur();

    let observed = received.borrow();
    let expected = [
        EchoMessage::Focus(true),
        EchoMessage::Key(key(KeyIdentity::letter('h'), Modifiers::NONE, Some("h"))),
        EchoMessage::Key(key(KeyIdentity::letter('i'), Modifiers::NONE, Some("i"))),
        EchoMessage::Key(key(
            KeyIdentity::letter('c'),
            Modifiers::NONE.with_control(),
            None,
        )),
        EchoMessage::Key(KeyEvent {
            key: Some(KeyIdentity::ARROW_RIGHT),
            modifiers: Modifiers::NONE,
            text: None,
            repeat: true,
        }),
        EchoMessage::Ime(ImeEvent::Preedit {
            text: "に".to_owned(),
            caret: Some(PreeditCaret::new(1, 1)),
        }),
        EchoMessage::Ime(ImeEvent::Preedit {
            text: "にほ".to_owned(),
            caret: Some(PreeditCaret::new(2, 2)),
        }),
        EchoMessage::Ime(ImeEvent::Preedit {
            text: "にほんご".to_owned(),
            caret: Some(PreeditCaret::new(4, 4)),
        }),
        EchoMessage::Ime(ImeEvent::Commit {
            text: "日本語".to_owned(),
        }),
        EchoMessage::Ime(ImeEvent::Preedit {
            text: "か".to_owned(),
            caret: Some(PreeditCaret::new(1, 1)),
        }),
        EchoMessage::Ime(ImeEvent::Cancel),
        EchoMessage::Focus(false),
    ];
    assert_eq!(observed.as_slice(), expected.as_slice());

    // The echo line holds the raw text plus the committed composition; the
    // canceled composition left no trace.
    runtime.with_component(|component| {
        assert_eq!(component.echo, "hi日本語");
        assert_eq!(component.preedit, None);
        assert!(!component.focused);
    });
}

#[test]
fn preedit_renders_distinctly_and_the_caret_rect_follows_the_composition() {
    let received = Rc::new(RefCell::new(Vec::new()));
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        EchoComponent::new(received),
        PlatformServices::default(),
    )
    .unwrap();
    let input = mounted_echo(&runtime);
    let handle = runtime
        .with_renderer(|renderer| renderer.backend().find_by_key("echo"))
        .unwrap();

    input.focus();
    input.key(key(KeyIdentity::letter('a'), Modifiers::NONE, Some("a")));
    input.ime(ImeEvent::Preedit {
        text: "にほ".to_owned(),
        caret: Some(PreeditCaret::new(2, 2)),
    });

    runtime.with_renderer(|renderer| {
        let Some(Props::Canvas {
            scene, ime_caret, ..
        }) = renderer.backend().props_of(handle)
        else {
            panic!("echo canvas must retain canvas properties");
        };
        // The scene carries the committed line and one distinct preedit run.
        let runs: Vec<_> = scene
            .commands()
            .iter()
            .filter_map(|command| match command {
                DrawCommand::GlyphRun { text, color, .. } => Some((text.clone(), *color)),
                _ => None,
            })
            .collect();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].0, "a");
        assert_eq!(runs[1].0, "にほ");
        assert_ne!(runs[0].1, runs[1].1, "preedit must render distinctly");
        // The declared caret rect advanced past the echo and the preedit.
        let expected_columns = 1 + 2;
        assert_eq!(
            *ime_caret,
            Some(CanvasRect::new(
                expected_columns as f64 * ECHO_GLYPH_WIDTH,
                0.0,
                1.0,
                ECHO_FONT_SIZE * 1.5,
            ))
        );
    });

    // Committing collapses the preedit and the caret keeps advancing from
    // the committed text alone.
    input.ime(ImeEvent::Commit {
        text: "にほ".to_owned(),
    });
    runtime.with_renderer(|renderer| {
        let Some(Props::Canvas { ime_caret, .. }) = renderer.backend().props_of(handle) else {
            panic!("echo canvas must retain canvas properties");
        };
        assert_eq!(
            *ime_caret,
            Some(CanvasRect::new(
                3.0 * ECHO_GLYPH_WIDTH,
                0.0,
                1.0,
                ECHO_FONT_SIZE * 1.5,
            ))
        );
    });
}

#[test]
fn input_handlers_are_replaced_without_reconnecting_native_identity() {
    let observed = Rc::new(RefCell::new(Vec::<String>::new()));
    let build = |observed: Rc<RefCell<Vec<String>>>| {
        canvas(
            CanvasSize::new(64.0, 16.0),
            DrawScene::new(),
            "Input target",
        )
        .accepts_input(true)
        .on_key(move |event| observed.borrow_mut().push(event.to_string()))
        .with_key("target")
    };
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer.render(build(observed.clone())).unwrap();
    let handle = renderer.backend().find_by_key("target").unwrap();
    let events = renderer.backend().events_of(handle).unwrap();

    renderer.render(build(observed.clone())).unwrap();
    events.emit_key(KeyEvent {
        key: Some(KeyIdentity::ENTER),
        modifiers: Modifiers::NONE,
        text: None,
        repeat: false,
    });

    assert_eq!(observed.borrow().as_slice(), ["Enter".to_owned()]);
    assert_eq!(renderer.backend().find_by_key("target"), Some(handle));
}

#[test]
fn input_declarations_without_acceptance_are_rejected_before_mutation() {
    let mut renderer = Renderer::new(HeadlessBackend::new());

    let orphaned_handler =
        canvas(CanvasSize::new(16.0, 16.0), DrawScene::new(), "Level meter").on_key(|_| {});
    let error = renderer.render(orphaned_handler).unwrap_err();
    assert!(error.to_string().contains("invalid canvas input"));
    assert!(error.to_string().contains("accepts_input"));
    assert!(renderer.backend().operations().is_empty());

    let orphaned_caret = canvas(CanvasSize::new(16.0, 16.0), DrawScene::new(), "Level meter")
        .ime_caret(CanvasRect::new(0.0, 0.0, 1.0, 16.0));
    let error = renderer.render(orphaned_caret).unwrap_err();
    assert!(error.to_string().contains("invalid canvas input"));
    assert!(renderer.backend().operations().is_empty());

    let invalid_caret = canvas(CanvasSize::new(16.0, 16.0), DrawScene::new(), "Terminal")
        .accepts_input(true)
        .ime_caret(CanvasRect::new(f64::NAN, 0.0, 1.0, 16.0));
    let error = renderer.render(invalid_caret).unwrap_err();
    assert!(error.to_string().contains("ime caret"));
    assert!(renderer.backend().operations().is_empty());
}
