//! Consumer-level context-menu contracts: structure, activation, and
//! reconciliation recorded deterministically by the headless host.

use rinka_core::{
    AppRuntime, CollectionPattern, Component, Dispatch, Element, MenuEntry, MenuItem, MenuItemRole,
    PlatformServices, Renderer, Submenu, TableColumn, UpdateContext, label, list, list_row,
};
use rinka_headless::{HeadlessBackend, Operation};
use std::cell::Cell;
use std::rc::Rc;

#[test]
fn context_menu_structure_is_recorded_with_submenu_and_separator() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(
            label("Cargo.toml")
                .context_menu([
                    MenuEntry::item(MenuItem::new("rename", "Rename", || {})),
                    MenuEntry::separator(),
                    MenuEntry::submenu(Submenu::new(
                        "open-with",
                        "Open With",
                        [MenuEntry::item(MenuItem::new("editor", "Editor", || {}))],
                    )),
                    MenuEntry::item(MenuItem::new("delete", "Delete", || {}).destructive()),
                ])
                .with_key("file"),
        )
        .unwrap();

    let handle = renderer.backend().find_by_key("file").unwrap();
    let menu = renderer
        .backend()
        .context_menu_of(handle)
        .expect("context menu recorded on the native object");
    assert_eq!(menu.entries.len(), 4);
    assert!(matches!(
        &menu.entries[0],
        MenuEntry::Item(item) if item.id == "rename" && item.enabled && !item.checked
    ));
    assert!(matches!(&menu.entries[1], MenuEntry::Separator));
    assert!(matches!(
        &menu.entries[2],
        MenuEntry::Submenu(submenu) if submenu.id == "open-with" && submenu.entries.len() == 1
    ));
    assert!(matches!(
        &menu.entries[3],
        MenuEntry::Item(item) if item.role == MenuItemRole::Destructive
    ));
}

#[test]
fn context_menu_attaches_to_a_table_row() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(
            list(
                "Files",
                [
                    list_row("Cargo.toml", None, None, false, false, "Cargo.toml", || {})
                        .table_cells(["Today"])
                        .context_menu([MenuEntry::item(MenuItem::new("rename", "Rename", || {}))])
                        .with_key("cargo"),
                ],
            )
            .table_columns([
                TableColumn::new("name", "Name"),
                TableColumn::new("modified", "Date Modified"),
            ])
            .collection_pattern(CollectionPattern::DataTable)
            .with_key("files"),
        )
        .unwrap();

    let row = renderer.backend().find_by_key("cargo").unwrap();
    let menu = renderer
        .backend()
        .context_menu_of(row)
        .expect("row context menu recorded");
    assert!(matches!(
        &menu.entries[0],
        MenuEntry::Item(item) if item.id == "rename"
    ));
}

struct FileMenu {
    renames: u32,
    deleted: bool,
}

enum FileMessage {
    Rename,
    Delete,
}

impl Component for FileMenu {
    type Message = FileMessage;

    fn update(&mut self, message: Self::Message, _context: &UpdateContext<Self::Message>) {
        match message {
            FileMessage::Rename => self.renames += 1,
            FileMessage::Delete => self.deleted = true,
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let rename = dispatch.clone();
        let delete = dispatch;
        label(if self.deleted {
            "deleted"
        } else {
            "Cargo.toml"
        })
        .context_menu([
            MenuEntry::item(MenuItem::new("rename", "Rename", move || {
                rename.emit(FileMessage::Rename);
            })),
            MenuEntry::item(
                MenuItem::new("delete", "Delete", move || {
                    delete.emit(FileMessage::Delete);
                })
                .destructive()
                .enabled(!self.deleted),
            ),
        ])
        .with_key("file")
    }
}

#[test]
fn activation_dispatches_exactly_one_message_through_the_stable_binding() {
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        FileMenu {
            renames: 0,
            deleted: false,
        },
        PlatformServices::default(),
    )
    .unwrap();
    let events = runtime
        .with_renderer(|renderer| {
            let handle = renderer.backend().find_by_key("file").unwrap();
            renderer.backend().events_of(handle)
        })
        .unwrap();

    assert!(events.emit_context_menu_activation("rename"));
    assert_eq!(runtime.with_component(|component| component.renames), 1);

    // The same stable binding observes the freshly rendered handler.
    assert!(events.emit_context_menu_activation("rename"));
    assert_eq!(runtime.with_component(|component| component.renames), 2);
}

#[test]
fn a_disabled_item_does_not_dispatch() {
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        FileMenu {
            renames: 0,
            deleted: false,
        },
        PlatformServices::default(),
    )
    .unwrap();
    let events = runtime
        .with_renderer(|renderer| {
            let handle = renderer.backend().find_by_key("file").unwrap();
            renderer.backend().events_of(handle)
        })
        .unwrap();

    assert!(events.emit_context_menu_activation("delete"));
    assert!(runtime.with_component(|component| component.deleted));

    // The reconciled model disables the item, so activation is refused.
    assert!(!events.emit_context_menu_activation("delete"));
    assert!(runtime.with_component(|component| component.deleted));
}

#[test]
fn a_disabled_submenu_disables_every_entry_inside_it() {
    let activations = Rc::new(Cell::new(0_u32));
    let direct = Rc::clone(&activations);
    let nested = Rc::clone(&activations);
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(
            label("Cargo.toml")
                .context_menu([
                    MenuEntry::item(
                        MenuItem::new("disabled", "Disabled", move || {
                            direct.set(direct.get() + 1);
                        })
                        .enabled(false),
                    ),
                    MenuEntry::submenu(
                        Submenu::new(
                            "more",
                            "More",
                            [MenuEntry::item(MenuItem::new(
                                "nested",
                                "Nested",
                                move || {
                                    nested.set(nested.get() + 1);
                                },
                            ))],
                        )
                        .enabled(false),
                    ),
                ])
                .with_key("file"),
        )
        .unwrap();
    let handle = renderer.backend().find_by_key("file").unwrap();
    let events = renderer.backend().events_of(handle).unwrap();

    assert!(!events.emit_context_menu_activation("disabled"));
    assert!(!events.emit_context_menu_activation("nested"));
    assert!(!events.emit_context_menu_activation("missing"));
    assert_eq!(activations.get(), 0);
}

#[test]
fn enabled_and_checkmark_state_reconcile_without_replacing_native_identity() {
    let tree = |enabled: bool, checked: bool| {
        label("Cargo.toml")
            .context_menu([
                MenuEntry::item(MenuItem::new("duplicate", "Duplicate", || {}).enabled(enabled)),
                MenuEntry::item(MenuItem::new("favorite", "Favorite", || {}).checked(checked)),
            ])
            .with_key("file")
    };
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer.render(tree(true, false)).unwrap();
    let handle = renderer.backend().find_by_key("file").unwrap();
    renderer.backend_mut().clear_operations();

    // Identical declarative state issues no patch even though the handler
    // closures are fresh instances.
    let stats = renderer.render(tree(true, false)).unwrap();
    assert_eq!(stats.patched, 0);

    let stats = renderer.render(tree(false, true)).unwrap();
    assert_eq!(stats.patched, 1);
    assert_eq!(renderer.backend().find_by_key("file"), Some(handle));
    let menu = renderer.backend().context_menu_of(handle).unwrap();
    assert!(matches!(
        &menu.entries[0],
        MenuEntry::Item(item) if item.id == "duplicate" && !item.enabled
    ));
    assert!(matches!(
        &menu.entries[1],
        MenuEntry::Item(item) if item.id == "favorite" && item.checked
    ));
    assert!(renderer.backend().operations().iter().any(|operation| {
        matches!(
            operation,
            Operation::Patch { handle: patched, patch }
                if *patched == handle && patch.context_menu().is_some()
        )
    }));
}

#[test]
fn removing_the_context_menu_patches_the_native_object() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(
            label("Cargo.toml")
                .context_menu([MenuEntry::item(MenuItem::new("rename", "Rename", || {}))])
                .with_key("file"),
        )
        .unwrap();
    let handle = renderer.backend().find_by_key("file").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer
        .render(label("Cargo.toml").with_key("file"))
        .unwrap();

    assert_eq!(stats.patched, 1);
    assert_eq!(renderer.backend().find_by_key("file"), Some(handle));
    assert!(renderer.backend().context_menu_of(handle).is_none());
}

#[test]
fn duplicate_menu_identities_are_rejected_before_native_mutation() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let error = renderer
        .render(label("Cargo.toml").context_menu([
            MenuEntry::item(MenuItem::new("same", "First", || {})),
            MenuEntry::submenu(Submenu::new(
                "more",
                "More",
                [MenuEntry::item(MenuItem::new("same", "Second", || {}))],
            )),
        ]))
        .unwrap_err();

    assert!(error.to_string().contains("same"));
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn an_empty_menu_identity_is_rejected_before_native_mutation() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let error = renderer
        .render(
            label("Cargo.toml").context_menu([MenuEntry::item(MenuItem::new(
                "",
                "Unnamed",
                || {},
            ))]),
        )
        .unwrap_err();

    assert!(error.to_string().contains("empty"));
    assert!(renderer.backend().operations().is_empty());
}
