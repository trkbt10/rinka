//! Deterministic accelerator dispatch through the headless native host.
//!
//! These tests feed synthetic chord events into [`rinka_core::AcceleratorRouter`]
//! over windows mounted on [`rinka_headless::HeadlessBackend`] and assert which
//! component message fires, how focused text input changes routing, and that
//! runtime enable/disable works without reconnecting anything native.

use rinka_core::{
    Accelerator, AcceleratorOutcome, AcceleratorRouter, AcceleratorScope, AppRuntime, Component,
    Dispatch, Element, KeyChord, KeyRoutingContext, PlatformServices, RenderError, Renderer,
    TreeError, UpdateContext, WindowId, column, label,
};
use rinka_headless::{HeadlessBackend, Operation};

fn chord(text: &str) -> KeyChord {
    text.parse().expect("test chord")
}

/// One synthetic key event: chord, key window, and text-input focus.
struct KeyEvent {
    chord: KeyChord,
    key_window: Option<&'static str>,
    text_input_focused: bool,
}

impl KeyEvent {
    const fn plain(chord: KeyChord, key_window: &'static str) -> Self {
        Self {
            chord,
            key_window: Some(key_window),
            text_input_focused: false,
        }
    }

    const fn while_typing(chord: KeyChord, key_window: &'static str) -> Self {
        Self {
            chord,
            key_window: Some(key_window),
            text_input_focused: true,
        }
    }
}

fn feed(router: &AcceleratorRouter, events: &[KeyEvent]) -> Vec<AcceleratorOutcome> {
    events
        .iter()
        .map(|event| {
            router.route(
                event.chord,
                &KeyRoutingContext {
                    key_window: event.key_window.map(WindowId::new),
                    text_input_focused: event.text_input_focused,
                },
            )
        })
        .collect()
}

/// Component whose accelerator table is derived from reconciled state.
struct ShortcutComponent {
    fired: Vec<&'static str>,
    save_enabled: bool,
    declare_extra: bool,
}

enum ShortcutMessage {
    Fired(&'static str),
    SetSaveEnabled(bool),
    DeclareExtra,
}

impl ShortcutComponent {
    const fn new() -> Self {
        Self {
            fired: Vec::new(),
            save_enabled: true,
            declare_extra: false,
        }
    }
}

impl Component for ShortcutComponent {
    type Message = ShortcutMessage;

    fn update(&mut self, message: Self::Message, _context: &UpdateContext<Self::Message>) {
        match message {
            ShortcutMessage::Fired(id) => self.fired.push(id),
            ShortcutMessage::SetSaveEnabled(enabled) => self.save_enabled = enabled,
            ShortcutMessage::DeclareExtra => self.declare_extra = true,
        }
        Effects::none()
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let save = dispatch.clone();
        let disable = dispatch.clone();
        let declare = dispatch.clone();
        let mut entries = vec![
            Accelerator::new("save", chord("Primary+S"), move || {
                save.emit(ShortcutMessage::Fired("save"));
            })
            .enabled(self.save_enabled),
            Accelerator::new("disable-save", chord("Primary+D"), move || {
                disable.emit(ShortcutMessage::SetSaveEnabled(false));
            }),
            Accelerator::new("declare-extra", chord("Primary+X"), move || {
                declare.emit(ShortcutMessage::DeclareExtra);
            }),
        ];
        if self.declare_extra {
            let extra = dispatch.clone();
            entries.push(Accelerator::new("extra", chord("Primary+E"), move || {
                extra.emit(ShortcutMessage::Fired("extra"));
            }));
        }
        column([label(format!(
            "fired={} save_enabled={}",
            self.fired.len(),
            self.save_enabled
        ))
        .with_key("state")])
        .with_key("root")
        .accelerators(entries)
    }
}

fn mount(component: ShortcutComponent) -> AppRuntime<HeadlessBackend, ShortcutComponent> {
    AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        component,
        PlatformServices::default(),
    )
    .expect("initial render succeeds")
}

fn register(
    router: &mut AcceleratorRouter,
    id: &str,
    runtime: &AppRuntime<HeadlessBackend, ShortcutComponent>,
) {
    let bindings = runtime.with_renderer(|renderer| renderer.accelerator_bindings().clone());
    router.register_window(WindowId::new(id), bindings);
}

#[test]
fn a_declared_chord_dispatches_its_message_to_the_component() {
    let runtime = mount(ShortcutComponent::new());
    let mut router = AcceleratorRouter::new();
    register(&mut router, "main", &runtime);

    let outcomes = feed(&router, &[KeyEvent::plain(chord("Primary+S"), "main")]);

    assert_eq!(
        outcomes,
        vec![AcceleratorOutcome::Dispatched {
            window: WindowId::new("main"),
            accelerator: "save".to_owned(),
        }]
    );
    runtime.with_component(|component| {
        assert_eq!(component.fired, vec!["save"]);
    });
    assert!(runtime.take_error().is_none());
}

#[test]
fn unregistered_chords_fall_through_untouched() {
    let runtime = mount(ShortcutComponent::new());
    let mut router = AcceleratorRouter::new();
    register(&mut router, "main", &runtime);

    let outcomes = feed(
        &router,
        &[
            KeyEvent::plain(chord("Primary+Q"), "main"),
            KeyEvent::plain(chord("Shift+F5"), "main"),
        ],
    );

    assert_eq!(
        outcomes,
        vec![AcceleratorOutcome::Unmatched, AcceleratorOutcome::Unmatched]
    );
    runtime.with_component(|component| assert!(component.fired.is_empty()));
}

#[test]
fn runtime_disable_stops_the_chord_without_reconnecting_native_state() {
    let runtime = mount(ShortcutComponent::new());
    let mut router = AcceleratorRouter::new();
    register(&mut router, "main", &runtime);
    runtime.with_renderer_mut(|renderer| renderer.backend_mut().clear_operations());

    // The chord fires, then a second chord disables it through a real
    // component message, then the first chord no longer fires — all through
    // the table registered exactly once above.
    let outcomes = feed(
        &router,
        &[
            KeyEvent::plain(chord("Primary+S"), "main"),
            KeyEvent::plain(chord("Primary+D"), "main"),
            KeyEvent::plain(chord("Primary+S"), "main"),
        ],
    );

    assert_eq!(
        outcomes,
        vec![
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("main"),
                accelerator: "save".to_owned(),
            },
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("main"),
                accelerator: "disable-save".to_owned(),
            },
            AcceleratorOutcome::Unmatched,
        ]
    );
    runtime.with_component(|component| {
        assert_eq!(component.fired, vec!["save"]);
        assert!(!component.save_enabled);
    });
    // The re-renders patched the state label; no native object was created,
    // destroyed, or replaced to change the accelerator table.
    runtime.with_renderer(|renderer| {
        assert!(
            renderer
                .backend()
                .operations()
                .iter()
                .all(|operation| matches!(operation, Operation::Patch { .. })),
            "accelerator changes must not touch native tree structure: {:?}",
            renderer.backend().operations()
        );
    });
}

#[test]
fn a_chord_added_on_re_render_fires_through_the_same_registration() {
    let runtime = mount(ShortcutComponent::new());
    let mut router = AcceleratorRouter::new();
    register(&mut router, "main", &runtime);

    let before = feed(&router, &[KeyEvent::plain(chord("Primary+E"), "main")]);
    assert_eq!(before, vec![AcceleratorOutcome::Unmatched]);

    // A chord bound to a real component message declares the additional
    // entry on the next render, still through the original registration.
    let declared = feed(&router, &[KeyEvent::plain(chord("Primary+X"), "main")]);
    assert_eq!(
        declared,
        vec![AcceleratorOutcome::Dispatched {
            window: WindowId::new("main"),
            accelerator: "declare-extra".to_owned(),
        }]
    );

    let after = feed(&router, &[KeyEvent::plain(chord("Primary+E"), "main")]);
    assert_eq!(
        after,
        vec![AcceleratorOutcome::Dispatched {
            window: WindowId::new("main"),
            accelerator: "extra".to_owned(),
        }]
    );
    runtime.with_component(|component| assert_eq!(component.fired, vec!["extra"]));
}

/// Two windows: a window-scoped chord fires only in its own key window while
/// an application-scoped chord fires from any key window.
struct ScopedComponent {
    fired: Vec<&'static str>,
    window_chord: &'static str,
    application_chord: Option<&'static str>,
}

enum ScopedMessage {
    Fired(&'static str),
}

impl Component for ScopedComponent {
    type Message = ScopedMessage;

    fn update(&mut self, message: Self::Message, _context: &UpdateContext<Self::Message>) {
        match message {
            ScopedMessage::Fired(id) => self.fired.push(id),
        }
        Effects::none()
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let window_dispatch = dispatch.clone();
        let mut entries = vec![Accelerator::new(
            "window-entry",
            chord(self.window_chord),
            move || window_dispatch.emit(ScopedMessage::Fired("window-entry")),
        )];
        if let Some(application_chord) = self.application_chord {
            let application_dispatch = dispatch.clone();
            entries.push(
                Accelerator::new("application-entry", chord(application_chord), move || {
                    application_dispatch.emit(ScopedMessage::Fired("application-entry"));
                })
                .scope(AcceleratorScope::Application)
                .global(true),
            );
        }
        column([label(format!("fired={}", self.fired.len())).with_key("state")])
            .with_key("root")
            .accelerators(entries)
    }
}

fn mount_scoped(component: ScopedComponent) -> AppRuntime<HeadlessBackend, ScopedComponent> {
    AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        component,
        PlatformServices::default(),
    )
    .expect("initial render succeeds")
}

#[test]
fn window_scope_is_confined_and_application_scope_reaches_every_key_window() {
    let editor = mount_scoped(ScopedComponent {
        fired: Vec::new(),
        window_chord: "Primary+1",
        application_chord: Some("Primary+Shift+A"),
    });
    let panel = mount_scoped(ScopedComponent {
        fired: Vec::new(),
        window_chord: "Primary+2",
        application_chord: None,
    });
    let mut router = AcceleratorRouter::new();
    router.register_window(
        WindowId::new("editor"),
        editor.with_renderer(|renderer| renderer.accelerator_bindings().clone()),
    );
    router.register_window(
        WindowId::new("panel"),
        panel.with_renderer(|renderer| renderer.accelerator_bindings().clone()),
    );

    let outcomes = feed(
        &router,
        &[
            // Window-scoped chord fires only while its window is key.
            KeyEvent::plain(chord("Primary+1"), "editor"),
            KeyEvent::plain(chord("Primary+1"), "panel"),
            // Application-scoped chord fires no matter which window is key.
            KeyEvent::plain(chord("Primary+Shift+A"), "editor"),
            KeyEvent::plain(chord("Primary+Shift+A"), "panel"),
        ],
    );

    assert_eq!(
        outcomes,
        vec![
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("editor"),
                accelerator: "window-entry".to_owned(),
            },
            AcceleratorOutcome::Unmatched,
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("editor"),
                accelerator: "application-entry".to_owned(),
            },
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("editor"),
                accelerator: "application-entry".to_owned(),
            },
        ]
    );
    editor.with_component(|component| {
        assert_eq!(
            component.fired,
            vec!["window-entry", "application-entry", "application-entry"]
        );
    });
    panel.with_component(|component| assert!(component.fired.is_empty()));
}

#[test]
fn focused_text_input_withholds_typing_chords_but_admits_global_entries() {
    let editor = mount_scoped(ScopedComponent {
        fired: Vec::new(),
        window_chord: "Primary+1",
        application_chord: Some("Primary+Shift+A"),
    });
    let mut router = AcceleratorRouter::new();
    router.register_window(
        WindowId::new("editor"),
        editor.with_renderer(|renderer| renderer.accelerator_bindings().clone()),
    );

    let outcomes = feed(
        &router,
        &[
            KeyEvent::while_typing(chord("Primary+1"), "editor"),
            KeyEvent::while_typing(chord("Primary+Shift+A"), "editor"),
            KeyEvent::plain(chord("Primary+1"), "editor"),
        ],
    );

    assert_eq!(
        outcomes,
        vec![
            AcceleratorOutcome::WithheldForTextInput {
                window: WindowId::new("editor"),
                accelerator: "window-entry".to_owned(),
            },
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("editor"),
                accelerator: "application-entry".to_owned(),
            },
            AcceleratorOutcome::Dispatched {
                window: WindowId::new("editor"),
                accelerator: "window-entry".to_owned(),
            },
        ]
    );
    editor.with_component(|component| {
        assert_eq!(component.fired, vec!["application-entry", "window-entry"]);
    });
}

#[test]
fn duplicate_chords_in_one_scope_are_a_typed_render_diagnostic() {
    struct DuplicateComponent;
    impl Component for DuplicateComponent {
        type Message = ();
        fn update(&mut self, (): Self::Message, _context: &UpdateContext<Self::Message>) {}
        fn view(&self, _dispatch: Dispatch<Self::Message>) -> Element {
            column([label("duplicate").with_key("state")])
                .with_key("root")
                .accelerators([
                    Accelerator::new("first", chord("Primary+S"), || {}),
                    Accelerator::new("second", chord("Primary+S"), || {}),
                ])
        }
    }

    let error = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        DuplicateComponent,
        PlatformServices::default(),
    )
    .expect_err("duplicate chords must not mount");
    assert!(
        matches!(
            &error,
            RenderError::Tree(TreeError::DuplicateAcceleratorChord { chord, scope })
                if chord == "Primary+S" && *scope == AcceleratorScope::Window
        ),
        "unexpected diagnostic: {error:?}"
    );
}

#[test]
fn accelerators_below_the_content_root_are_rejected() {
    struct MisplacedComponent;
    impl Component for MisplacedComponent {
        type Message = ();
        fn update(&mut self, (): Self::Message, _context: &UpdateContext<Self::Message>) {}
        fn view(&self, _dispatch: Dispatch<Self::Message>) -> Element {
            column([label("inner")
                .with_key("inner")
                .accelerators([Accelerator::new("nested", chord("Primary+N"), || {})])])
            .with_key("root")
        }
    }

    let error = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        MisplacedComponent,
        PlatformServices::default(),
    )
    .expect_err("nested accelerator tables must not mount");
    assert!(
        matches!(
            &error,
            RenderError::Tree(TreeError::InvalidAcceleratorTable { path, .. })
                if path == "root/inner"
        ),
        "unexpected diagnostic: {error:?}"
    );
}
