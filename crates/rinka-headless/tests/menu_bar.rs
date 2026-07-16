//! Deterministic application menu bar reconciliation and routing over the
//! headless native host.
//!
//! These tests mount real components on [`rinka_headless::HeadlessBackend`],
//! let their content roots declare menu bars, and prove through
//! [`rinka_core::MenuBarRouter`] that an app-defined item's activation is
//! delivered as a message to the focused window's component, that switching
//! focus redirects delivery, and that runtime add/remove/relabel/enable/check
//! changes produce the expected recorded update plans without touching native
//! tree structure.

use rinka_core::{
    AppRuntime, Component, Dispatch, Element, MenuBar, MenuBarActivation, MenuBarBindings,
    MenuBarEntry, MenuBarMenu, MenuBarMenuRole, MenuBarRouter, MenuBarUpdate, MenuItem,
    PlatformServices, RenderError, Renderer, StandardItem, TreeError, UpdateContext, WindowId,
    column, label,
};
use rinka_headless::{HeadlessBackend, Operation};

fn chord(text: &str) -> rinka_core::KeyChord {
    text.parse().expect("test chord")
}

/// Component whose menu bar declaration derives from reconciled state.
///
/// Every state transition is itself a declared menu item, so the tests drive
/// the component exclusively through routed menu activations — the same
/// queued delivery a platform host's menu target uses.
struct MenuComponent {
    window: &'static str,
    received: Vec<String>,
    new_folder_enabled: bool,
    hidden_checked: bool,
    declare_extra_item: bool,
}

enum MenuMessage {
    Command(String),
    LockNewFolder,
    ToggleHidden,
    DeclareExtraItem,
}

impl MenuComponent {
    const fn new(window: &'static str) -> Self {
        Self {
            window,
            received: Vec::new(),
            new_folder_enabled: true,
            hidden_checked: false,
            declare_extra_item: false,
        }
    }
}

impl Component for MenuComponent {
    type Message = MenuMessage;

    fn update(&mut self, message: Self::Message, _context: &UpdateContext<Self::Message>) {
        match message {
            MenuMessage::Command(id) => self.received.push(id),
            MenuMessage::LockNewFolder => self.new_folder_enabled = false,
            MenuMessage::ToggleHidden => self.hidden_checked = !self.hidden_checked,
            MenuMessage::DeclareExtraItem => self.declare_extra_item = true,
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let window = self.window;
        let new_folder = dispatch.clone();
        let lock = dispatch.clone();
        let hidden = dispatch.clone();
        let declare = dispatch.clone();
        let mut file_entries = vec![
            MenuBarEntry::item(
                MenuItem::new("new-folder", "New Folder", move || {
                    new_folder.emit(MenuMessage::Command(format!("{window}:new-folder")));
                })
                .enabled(self.new_folder_enabled)
                .chord(chord("Primary+N")),
            ),
            MenuBarEntry::item(MenuItem::new(
                "lock-new-folder",
                "Lock New Folder",
                move || {
                    lock.emit(MenuMessage::LockNewFolder);
                },
            )),
            MenuBarEntry::item(MenuItem::new("declare-extra", "Declare Extra", move || {
                declare.emit(MenuMessage::DeclareExtraItem);
            })),
            MenuBarEntry::separator(),
            MenuBarEntry::standard(StandardItem::CloseWindow),
        ];
        if self.declare_extra_item {
            let extra = dispatch.clone();
            file_entries.insert(
                1,
                MenuBarEntry::item(MenuItem::new("extra", "Extra", move || {
                    extra.emit(MenuMessage::Command(format!("{window}:extra")));
                })),
            );
        }
        let hidden_label = if self.hidden_checked {
            "Hide Hidden Files"
        } else {
            "Show Hidden Files"
        };
        column([label(format!(
            "window={window} received={} new_folder_enabled={} hidden={}",
            self.received.len(),
            self.new_folder_enabled,
            self.hidden_checked,
        ))
        .with_key("state")])
        .with_key("root")
        .menu_bar(MenuBar::new([
            MenuBarMenu::new("file", "File", file_entries),
            MenuBarMenu::new(
                "view",
                "View",
                [MenuBarEntry::item(
                    MenuItem::new("toggle-hidden", hidden_label, move || {
                        hidden.emit(MenuMessage::ToggleHidden);
                    })
                    .checked(self.hidden_checked),
                )],
            ),
            MenuBarMenu::new(
                "window",
                "Window",
                [MenuBarEntry::standard(StandardItem::Minimize)],
            )
            .role(MenuBarMenuRole::Window),
        ]))
    }
}

fn mount(component: MenuComponent) -> AppRuntime<HeadlessBackend, MenuComponent> {
    AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        component,
        PlatformServices::default(),
    )
    .expect("initial render succeeds")
}

fn register(
    router: &mut MenuBarRouter,
    id: &str,
    runtime: &AppRuntime<HeadlessBackend, MenuComponent>,
) {
    let bindings = runtime.with_renderer(|renderer| renderer.menu_bar_bindings().clone());
    router.register_window(WindowId::new(id), bindings);
}

fn declared_model(runtime: &AppRuntime<HeadlessBackend, MenuComponent>) -> MenuBar {
    runtime
        .with_renderer(|renderer| renderer.menu_bar_bindings().model())
        .expect("the component root declares a menu bar")
}

#[test]
fn activation_reaches_the_focused_window_and_focus_switch_redirects_delivery() {
    let first = mount(MenuComponent::new("first"));
    let second = mount(MenuComponent::new("second"));
    let mut router = MenuBarRouter::new(MenuBar::default());
    register(&mut router, "first", &first);
    register(&mut router, "second", &second);

    // The same item identity dispatches into whichever window is focused.
    assert_eq!(
        router.activate(Some(&WindowId::new("first")), "new-folder"),
        MenuBarActivation::Dispatched {
            owner: Some(WindowId::new("first")),
        }
    );
    assert_eq!(
        router.activate(Some(&WindowId::new("second")), "new-folder"),
        MenuBarActivation::Dispatched {
            owner: Some(WindowId::new("second")),
        }
    );

    first.with_component(|component| {
        assert_eq!(component.received, vec!["first:new-folder".to_owned()]);
    });
    second.with_component(|component| {
        assert_eq!(component.received, vec!["second:new-folder".to_owned()]);
    });
    assert!(first.take_error().is_none());
    assert!(second.take_error().is_none());
}

#[test]
fn a_focused_window_without_a_declaration_delegates_to_the_first_declaring_window() {
    let main = mount(MenuComponent::new("main"));
    let mut router = MenuBarRouter::new(MenuBar::default());
    register(&mut router, "main", &main);
    // The activity panel declares no bar of its own.
    router.register_window(WindowId::new("panel"), MenuBarBindings::default());

    let (owner, model) = router
        .effective_model(Some(&WindowId::new("panel")))
        .expect("fallback bar exists");
    assert_eq!(owner, Some(WindowId::new("main")));
    assert_eq!(model.menus.len(), 3);

    assert_eq!(
        router.activate(Some(&WindowId::new("panel")), "new-folder"),
        MenuBarActivation::Dispatched {
            owner: Some(WindowId::new("main")),
        }
    );
    main.with_component(|component| {
        assert_eq!(component.received, vec!["main:new-folder".to_owned()]);
    });
}

#[test]
fn runtime_disable_refreshes_in_place_and_refuses_the_item_without_native_churn() {
    let runtime = mount(MenuComponent::new("main"));
    let mut router = MenuBarRouter::new(MenuBar::default());
    register(&mut router, "main", &runtime);
    let key = Some(WindowId::new("main"));
    runtime.with_renderer_mut(|renderer| renderer.backend_mut().clear_operations());

    let before = declared_model(&runtime);
    assert!(router.item_enabled(key.as_ref(), "new-folder"));

    // A routed activation disables the item through a real component message.
    assert_eq!(
        router.activate(key.as_ref(), "lock-new-folder"),
        MenuBarActivation::Dispatched {
            owner: Some(WindowId::new("main")),
        }
    );

    let after = declared_model(&runtime);
    assert_eq!(
        MenuBar::plan_update(&before, &after),
        MenuBarUpdate::RefreshInPlace
    );
    assert!(!router.item_enabled(key.as_ref(), "new-folder"));
    assert_eq!(
        router.activate(key.as_ref(), "new-folder"),
        MenuBarActivation::Refused
    );
    runtime.with_component(|component| {
        assert!(component.received.is_empty());
        assert!(!component.new_folder_enabled);
    });

    // The re-render patched the state label; changing the menu bar itself
    // required no native tree mutation.
    runtime.with_renderer(|renderer| {
        assert!(
            renderer
                .backend()
                .operations()
                .iter()
                .all(|operation| matches!(operation, Operation::Patch { .. })),
            "menu bar changes must not touch native tree structure: {:?}",
            renderer.backend().operations()
        );
    });
}

#[test]
fn relabel_and_checkmark_changes_refresh_while_add_remove_rebuilds() {
    let runtime = mount(MenuComponent::new("main"));
    let mut router = MenuBarRouter::new(MenuBar::default());
    register(&mut router, "main", &runtime);
    let key = Some(WindowId::new("main"));

    let base = declared_model(&runtime);
    assert_eq!(
        base.find_item("toggle-hidden")
            .expect("declared item")
            .label,
        "Show Hidden Files"
    );

    // Relabel plus checkmark: same structure, refresh in place.
    router.activate(key.as_ref(), "toggle-hidden");
    let checked = declared_model(&runtime);
    assert_eq!(
        MenuBar::plan_update(&base, &checked),
        MenuBarUpdate::RefreshInPlace
    );
    let hidden_item = checked.find_item("toggle-hidden").expect("declared item");
    assert!(hidden_item.checked);
    assert_eq!(hidden_item.label, "Hide Hidden Files");

    // Structural change: an item added at runtime rebuilds.
    router.activate(key.as_ref(), "declare-extra");
    let grown = declared_model(&runtime);
    assert_eq!(
        MenuBar::plan_update(&checked, &grown),
        MenuBarUpdate::Rebuild
    );
    assert!(grown.find_item("extra").is_some());

    // A render that changes only handlers and unrelated content is no menu
    // bar update at all.
    router.activate(key.as_ref(), "new-folder");
    let after_command = declared_model(&runtime);
    assert_eq!(
        MenuBar::plan_update(&grown, &after_command),
        MenuBarUpdate::Unchanged
    );

    // The freshly declared item dispatches through the same registration.
    assert_eq!(
        router.activate(key.as_ref(), "extra"),
        MenuBarActivation::Dispatched {
            owner: Some(WindowId::new("main")),
        }
    );
    runtime.with_component(|component| {
        assert_eq!(
            component.received,
            vec!["main:new-folder".to_owned(), "main:extra".to_owned()]
        );
    });
}

#[test]
fn the_effective_bar_claims_menu_owned_chords_for_the_platform_monitor() {
    let runtime = mount(MenuComponent::new("main"));
    let mut router = MenuBarRouter::new(MenuBar::default());
    register(&mut router, "main", &runtime);
    let key = Some(WindowId::new("main"));

    // The app-defined item's declared chord and the standard items' canonical
    // chords are menu-owned; anything else falls through to the accelerator
    // tables.
    assert!(router.claims_chord(key.as_ref(), chord("Primary+N")));
    assert!(router.claims_chord(key.as_ref(), chord("Primary+W")));
    assert!(router.claims_chord(key.as_ref(), chord("Primary+M")));
    assert!(!router.claims_chord(key.as_ref(), chord("Primary+Shift+H")));
    assert!(!router.claims_chord(None, chord("Primary+Shift+H")));

    assert_eq!(
        router.activate(key.as_ref(), "unknown-item"),
        MenuBarActivation::Unknown
    );
}

#[test]
fn a_menu_bar_below_the_content_root_is_a_typed_render_diagnostic() {
    struct BelowRoot;
    impl Component for BelowRoot {
        type Message = ();

        fn update(&mut self, (): Self::Message, _context: &UpdateContext<Self::Message>) {}

        fn view(&self, _dispatch: Dispatch<Self::Message>) -> Element {
            column([column([label("inner").with_key("inner")])
                .with_key("nested")
                .menu_bar(MenuBar::new([MenuBarMenu::new(
                    "file",
                    "File",
                    [MenuBarEntry::standard(StandardItem::CloseWindow)],
                )]))])
            .with_key("root")
        }
    }

    let error = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        BelowRoot,
        PlatformServices::default(),
    )
    .expect_err("a nested menu bar must be rejected");
    assert!(matches!(
        error,
        RenderError::Tree(TreeError::InvalidMenuBar { .. })
    ));
}

#[test]
fn a_shadowed_standard_chord_is_a_typed_render_diagnostic() {
    struct DuplicateChord;
    impl Component for DuplicateChord {
        type Message = ();

        fn update(&mut self, (): Self::Message, _context: &UpdateContext<Self::Message>) {}

        fn view(&self, _dispatch: Dispatch<Self::Message>) -> Element {
            column([label("state").with_key("state")])
                .with_key("root")
                .menu_bar(MenuBar::new([MenuBarMenu::new(
                    "edit",
                    "Edit",
                    [
                        MenuBarEntry::standard(StandardItem::Copy),
                        // Primary+C is the canonical Copy chord; binding it on
                        // an app-defined item would shadow the standard role.
                        MenuBarEntry::item(
                            MenuItem::new("shadow", "Shadow Copy", || {}).chord(chord("Primary+C")),
                        ),
                    ],
                )]))
        }
    }

    let error = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        DuplicateChord,
        PlatformServices::default(),
    )
    .expect_err("a shadowed standard chord must be rejected");
    assert!(matches!(
        error,
        RenderError::Tree(TreeError::InvalidMenuBar { .. })
    ));
}
