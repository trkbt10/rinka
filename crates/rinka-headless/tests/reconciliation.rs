//! Consumer-level reconciliation and runtime contracts.

use rinka_core::{
    AppRuntime, Component, Dispatch, Element, ListRowRole, ListStyle, Props, Renderer,
    SortDirection, Spacing, TableColumn, TableSort, WindowContent, WindowRuntime, button, column,
    label, list, list_row, workspace,
};
use rinka_headless::{HeadlessBackend, Operation};
use std::cell::Cell;
use std::rc::Rc;

#[test]
fn property_change_preserves_native_identity() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer.render(label("before").with_key("title")).unwrap();
    let before = renderer.backend().find_by_key("title").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer.render(label("after").with_key("title")).unwrap();
    let after = renderer.backend().find_by_key("title").unwrap();

    assert_eq!(before, after);
    assert_eq!(stats.patched, 1);
    assert_eq!(stats.created, 0);
    assert!(renderer.backend().operations().iter().any(
        |operation| matches!(operation, Operation::Patch { handle, .. } if *handle == before)
    ));
}

struct WindowSelection {
    selected: bool,
}

impl Component for WindowSelection {
    type Message = bool;

    fn update(&mut self, selected: Self::Message) {
        self.selected = selected;
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        button(
            if self.selected { "Selected" } else { "Select" },
            "Select item",
            move || dispatch.emit(true),
        )
        .with_key("selection")
    }
}

#[test]
fn window_content_reconciles_component_messages_on_the_same_native_root() {
    let renderer = Renderer::new(HeadlessBackend::new());
    let runtime = WindowRuntime::mount(
        renderer,
        WindowContent::component(WindowSelection { selected: false }),
    )
    .unwrap();
    let (before, events) = runtime.with_renderer(|renderer| {
        let handle = renderer.backend().find_by_key("selection").unwrap();
        (handle, renderer.backend().events_of(handle).unwrap())
    });

    events.emit_activate();

    runtime.with_renderer(|renderer| {
        assert_eq!(renderer.backend().find_by_key("selection"), Some(before));
        assert!(matches!(
            renderer.backend().props_of(before),
            Some(Props::Button { label, .. }) if label == "Selected"
        ));
    });
}

struct WindowRootTransition {
    replaced: bool,
}

impl Component for WindowRootTransition {
    type Message = ();

    fn update(&mut self, (): Self::Message) {
        self.replaced = true;
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        if self.replaced {
            column([label("Detached")]).with_key("replacement")
        } else {
            button("Replace root", "Replace root", move || dispatch.emit(())).with_key("original")
        }
    }
}

#[test]
fn window_root_kind_change_is_rejected_before_native_mutation() {
    let renderer = Renderer::new(HeadlessBackend::new());
    let runtime = WindowRuntime::mount(
        renderer,
        WindowContent::component(WindowRootTransition { replaced: false }),
    )
    .unwrap();
    let (original, events, operation_count) = runtime.with_renderer(|renderer| {
        let handle = renderer.backend().find_by_key("original").unwrap();
        (
            handle,
            renderer.backend().events_of(handle).unwrap(),
            renderer.backend().operations().len(),
        )
    });

    events.emit_activate();

    let error = runtime.take_error().expect("root transition must fail");
    assert!(error.to_string().contains("root kind must remain stable"));
    runtime.with_renderer(|renderer| {
        assert_eq!(renderer.backend().find_by_key("original"), Some(original));
        assert_eq!(renderer.backend().find_by_key("replacement"), None);
        assert_eq!(renderer.backend().operations().len(), operation_count);
    });
}

#[test]
fn keyed_reorder_moves_existing_objects() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(
            column([
                label("A").with_key("a"),
                label("B").with_key("b"),
                label("C").with_key("c"),
            ])
            .with_key("screen"),
        )
        .unwrap();
    let a = renderer.backend().find_by_key("a").unwrap();
    let b = renderer.backend().find_by_key("b").unwrap();
    let c = renderer.backend().find_by_key("c").unwrap();
    let screen = renderer.backend().find_by_key("screen").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer
        .render(
            column([
                label("C").with_key("c"),
                label("A").with_key("a"),
                label("B").with_key("b"),
            ])
            .with_key("screen"),
        )
        .unwrap();

    assert_eq!(
        renderer.backend().children_of(screen),
        Some([c, a, b].as_slice())
    );
    assert_eq!(stats.moved, 1);
    assert_eq!(stats.created, 0);
}

#[test]
fn duplicate_keys_are_a_release_error() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let error = renderer
        .render(column([
            label("A").with_key("same"),
            label("B").with_key("same"),
        ]))
        .unwrap_err();
    assert!(error.to_string().contains("duplicate sibling key"));
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn invalid_tree_is_rejected_before_native_mutation() {
    let invalid = rinka_core::list("Files", [label("not a row")]);
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let error = renderer.render(invalid).unwrap_err();
    assert!(error.to_string().contains("is not a list row"));
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn invalid_table_schema_is_rejected_before_native_mutation() {
    let invalid = list(
        "Files",
        [
            list_row("Cargo.toml", None, None, false, false, "Cargo.toml", || {})
                .table_cells(["Today"]),
        ],
    )
    .table_columns([
        TableColumn::new("name", "Name"),
        TableColumn::new("name", "Duplicate Name"),
    ])
    .list_style(ListStyle::Table);
    let mut renderer = Renderer::new(HeadlessBackend::new());

    let error = renderer.render(invalid).unwrap_err();

    assert!(error.to_string().contains("empty or duplicated"));
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn secondary_cells_without_declared_table_columns_are_rejected() {
    let invalid = list(
        "Files",
        [
            list_row("Cargo.toml", None, None, false, false, "Cargo.toml", || {})
                .table_cells(["Today"]),
        ],
    )
    .list_style(ListStyle::Table);
    let mut renderer = Renderer::new(HeadlessBackend::new());

    let error = renderer.render(invalid).unwrap_err();

    assert!(error.to_string().contains("secondary columns"));
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn secondary_cells_on_a_source_list_are_rejected() {
    let invalid = list(
        "Locations",
        [list_row("Home", None, None, false, false, "Home", || {}).table_cells(["Today"])],
    )
    .list_style(ListStyle::Source);
    let mut renderer = Renderer::new(HeadlessBackend::new());

    let error = renderer.render(invalid).unwrap_err();

    assert!(error.to_string().contains("require table presentation"));
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn changing_between_source_and_table_replaces_the_native_container() {
    let row = || list_row("Home", None, None, true, false, "Home", || {}).with_key("home");
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(
            list("Locations", [row()])
                .list_style(ListStyle::Source)
                .with_key("locations"),
        )
        .unwrap();
    let source = renderer.backend().find_by_key("locations").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer
        .render(
            list("Locations", [row()])
                .table_columns([TableColumn::new("name", "Name")])
                .list_style(ListStyle::Table)
                .with_key("locations"),
        )
        .unwrap();

    assert_eq!(stats.replaced, 1);
    assert_ne!(renderer.backend().find_by_key("locations"), Some(source));
}

#[test]
fn changing_between_table_and_non_outline_lists_replaces_the_native_container() {
    let row = || list_row("Home", None, None, true, false, "Home", || {}).with_key("home");
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(
            list("Files", [row()])
                .list_style(ListStyle::Content)
                .with_key("files"),
        )
        .unwrap();
    let content = renderer.backend().find_by_key("files").unwrap();

    let stats = renderer
        .render(
            list("Files", [row()])
                .table_columns([TableColumn::new("name", "Name")])
                .list_style(ListStyle::Table)
                .with_key("files"),
        )
        .unwrap();
    assert_eq!(stats.replaced, 1);
    let table = renderer.backend().find_by_key("files").unwrap();
    assert_ne!(table, content);

    let stats = renderer
        .render(
            list("Files", [row()])
                .list_style(ListStyle::Plain)
                .with_key("files"),
        )
        .unwrap();
    assert_eq!(stats.replaced, 1);
    assert_ne!(renderer.backend().find_by_key("files"), Some(table));
}

#[test]
fn source_sections_and_nested_rows_preserve_declarative_hierarchy() {
    let source = list(
        "Locations",
        [
            list_row("Favorites", None, None, false, false, "Favorites", || {})
                .source_section()
                .expanded(true)
                .source_children([
                    list_row("Home", None, None, true, false, "Home", || {}).with_key("home"),
                    list_row("Downloads", None, None, false, false, "Downloads", || {})
                        .with_key("downloads"),
                ])
                .with_key("favorites"),
        ],
    )
    .list_style(ListStyle::Source)
    .with_key("locations");
    let mut renderer = Renderer::new(HeadlessBackend::new());

    renderer.render(source).unwrap();

    let section = renderer.backend().find_by_key("favorites").unwrap();
    let home = renderer.backend().find_by_key("home").unwrap();
    let downloads = renderer.backend().find_by_key("downloads").unwrap();
    assert_eq!(
        renderer.backend().children_of(section),
        Some([home, downloads].as_slice())
    );
    assert!(matches!(
        renderer.backend().props_of(section),
        Some(Props::ListRow {
            role: ListRowRole::Section,
            expanded: true,
            ..
        })
    ));
}

#[test]
fn hierarchy_outside_source_presentation_is_rejected() {
    let invalid = list(
        "Files",
        [
            list_row("Folder", None, None, false, false, "Folder", || {})
                .source_children([list_row("Child", None, None, false, false, "Child", || {})]),
        ],
    );
    let mut renderer = Renderer::new(HeadlessBackend::new());

    let error = renderer.render(invalid).unwrap_err();

    assert!(error.to_string().contains("require source presentation"));
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn multiple_active_sort_columns_are_rejected() {
    let invalid = list(
        "Files",
        [
            list_row("Cargo.toml", None, None, false, false, "Cargo.toml", || {})
                .table_cells(["Today"]),
        ],
    )
    .table_columns([
        TableColumn::new("name", "Name").sorted(SortDirection::Ascending),
        TableColumn::new("modified", "Date Modified").sorted(SortDirection::Descending),
    ])
    .list_style(ListStyle::Table);
    let mut renderer = Renderer::new(HeadlessBackend::new());

    let error = renderer.render(invalid).unwrap_err();

    assert!(error.to_string().contains("only one active sort column"));
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn table_sort_handler_is_replaced_without_reconnecting_native_identity() {
    let observed = Rc::new(std::cell::RefCell::new(Vec::<TableSort>::new()));
    let table = |observed: Rc<std::cell::RefCell<Vec<TableSort>>>| {
        list(
            "Files",
            [list_row(
                "Cargo.toml",
                None,
                None,
                false,
                false,
                "Cargo.toml",
                || {},
            )],
        )
        .table_columns([TableColumn::new("name", "Name").sortable(true)])
        .list_style(ListStyle::Table)
        .on_sort_change(move |sort| observed.borrow_mut().push(sort))
        .with_key("files")
    };
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer.render(table(observed.clone())).unwrap();
    let handle = renderer.backend().find_by_key("files").unwrap();
    let events = renderer.backend().events_of(handle).unwrap();

    renderer.render(table(observed.clone())).unwrap();
    events.emit_sort(TableSort {
        column_id: "name".to_owned(),
        direction: SortDirection::Descending,
    });

    assert_eq!(
        observed.borrow().as_slice(),
        [TableSort {
            column_id: "name".to_owned(),
            direction: SortDirection::Descending,
        }]
    );
}

#[test]
fn workspace_keeps_three_semantic_regions_and_patches_collapse_policy() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(
            workspace(
                true,
                false,
                label("Sidebar").with_key("sidebar"),
                label("Content").with_key("content"),
                label("Inspector").with_key("inspector"),
            )
            .with_key("workspace"),
        )
        .unwrap();

    let workspace_handle = renderer.backend().find_by_key("workspace").unwrap();
    let sidebar = renderer.backend().find_by_key("sidebar").unwrap();
    let content = renderer.backend().find_by_key("content").unwrap();
    let inspector = renderer.backend().find_by_key("inspector").unwrap();
    assert_eq!(
        renderer.backend().children_of(workspace_handle),
        Some([sidebar, content, inspector].as_slice())
    );
    renderer.backend_mut().clear_operations();

    let stats = renderer
        .render(
            workspace(
                false,
                true,
                label("Sidebar").with_key("sidebar"),
                label("Content").with_key("content"),
                label("Inspector").with_key("inspector"),
            )
            .with_key("workspace"),
        )
        .unwrap();

    assert_eq!(stats.patched, 1);
    assert_eq!(stats.created, 0);
    assert!(matches!(
        renderer.backend().props_of(workspace_handle),
        Some(Props::Workspace {
            sidebar_collapsible: false,
            inspector_collapsible: true,
        })
    ));
}

#[test]
fn table_columns_and_row_cells_patch_without_replacing_native_identity() {
    let table = |kind: &str| {
        list(
            "Files",
            [
                list_row("Cargo.toml", None, None, true, false, "Cargo.toml", || {})
                    .table_cells(["Today", kind])
                    .with_key("cargo"),
            ],
        )
        .table_columns([
            TableColumn::new("name", "Name"),
            TableColumn::new("modified", "Date Modified"),
            TableColumn::new("kind", "Kind"),
        ])
        .list_style(ListStyle::Table)
        .with_key("files")
    };
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer.render(table("TOML")).unwrap();
    let table_handle = renderer.backend().find_by_key("files").unwrap();
    let row_handle = renderer.backend().find_by_key("cargo").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer.render(table("Configuration")).unwrap();

    assert_eq!(stats.patched, 1);
    assert_eq!(renderer.backend().find_by_key("files"), Some(table_handle));
    assert_eq!(renderer.backend().find_by_key("cargo"), Some(row_handle));
    assert!(matches!(
        renderer.backend().props_of(row_handle),
        Some(Props::ListRow { cells, .. }) if cells == &["Today", "Configuration"]
    ));
}

#[test]
fn stable_signal_observes_current_handler() {
    let total = Rc::new(Cell::new(0));
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let first = Rc::clone(&total);
    renderer
        .render(button("Run", "Run", move || first.set(first.get() + 1)).with_key("run"))
        .unwrap();
    let handle = renderer.backend().find_by_key("run").unwrap();
    let events = renderer.backend().events_of(handle).unwrap();
    events.emit_activate();

    let second = Rc::clone(&total);
    renderer
        .render(button("Run", "Run", move || second.set(second.get() + 10)).with_key("run"))
        .unwrap();
    events.emit_activate();

    assert_eq!(total.get(), 11);
    assert_eq!(renderer.backend().find_by_key("run"), Some(handle));
}

#[derive(Default)]
struct Counter {
    count: u32,
}

enum Message {
    Increment,
}

impl Component for Counter {
    type Message = Message;

    fn update(&mut self, message: Self::Message) {
        match message {
            Message::Increment => self.count += 1,
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        column([
            label(format!("count={}", self.count)).with_key("count"),
            button("Increment", "Increment counter", move || {
                dispatch.emit(Message::Increment);
            })
            .with_key("increment"),
        ])
        .spacing(Spacing::Section)
        .with_key("counter")
    }
}

#[test]
fn native_event_updates_component_and_patches_tree() {
    let runtime =
        AppRuntime::mount(Renderer::new(HeadlessBackend::new()), Counter::default()).unwrap();
    let handle = runtime
        .with_renderer(|renderer| renderer.backend().find_by_key("increment"))
        .unwrap();
    let events = runtime
        .with_renderer(|renderer| renderer.backend().events_of(handle))
        .unwrap();
    events.emit_activate();
    events.emit_activate();

    assert_eq!(runtime.with_component(|state| state.count), 2);
    let label = runtime
        .with_renderer(|renderer| renderer.backend().find_by_key("count"))
        .unwrap();
    assert!(matches!(
        runtime.with_renderer(|renderer| renderer.backend().props_of(label).cloned()),
        Some(Props::Label { text, .. }) if text == "count=2"
    ));
}
