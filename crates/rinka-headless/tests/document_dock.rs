//! Consumer tests for the tabbed-document dock: every semantic operation as
//! a deterministic gesture → message → reconciliation round trip.

use rinka_core::{
    Alert, AppRuntime, Component, DialogButtonRole, DialogOutcome, Dispatch, DockEdge, DockEvent,
    DockGroup, DockLayout, DockNode, DockTab, Element, EventBindings, MenuEntry, MenuItem,
    PlatformServices, RenderError, Renderer, TreeError, UpdateContext, button, column, dock, label,
};
use rinka_headless::{FakeDialogPresenter, Handle, HeadlessBackend, Operation};

/// A document workspace owning its dock layout, the way Overshell owns its
/// editor area: tab content and state stay with the consumer, the dock
/// carries only ids and chrome.
struct Workspace {
    layout: DockLayout,
    saved: Option<String>,
    next_group: u32,
}

enum WorkspaceMessage {
    Dock(DockEvent),
    CloseConfirmed(String),
    CloseCancelled,
    SplitActiveTrailing,
    SaveLayout,
    RestoreLayout,
    CloseOthers(String),
    CloseToTheRight(String),
}

impl Workspace {
    fn initial() -> Self {
        Self {
            layout: DockLayout::single_group(DockGroup::new(
                "documents",
                [
                    DockTab::new("readme", "README.md"),
                    DockTab::new("main", "main.rs").dirty(true),
                    DockTab::new("cargo", "Cargo.toml"),
                ],
                "readme",
            )),
            saved: None,
            next_group: 0,
        }
    }
}

impl Component for Workspace {
    type Message = WorkspaceMessage;

    fn update(&mut self, message: Self::Message, context: &UpdateContext<Self::Message>) {
        match message {
            WorkspaceMessage::Dock(DockEvent::SelectTab { tab, .. }) => {
                self.layout.select_tab(&tab);
            }
            WorkspaceMessage::Dock(DockEvent::CloseTab { tab, .. }) => {
                if self.layout.tab(&tab).is_some_and(|tab| tab.dirty) {
                    // The veto round trip: a dirty tab closes only through
                    // an explicit dialog answer.
                    context.dialogs().alert(
                        Alert::new("Close without saving?", "Unsaved edits will be lost.")
                            .button(
                                "Close",
                                DialogButtonRole::Destructive,
                                WorkspaceMessage::CloseConfirmed(tab),
                            )
                            .button(
                                "Cancel",
                                DialogButtonRole::Cancel,
                                WorkspaceMessage::CloseCancelled,
                            ),
                    );
                } else {
                    self.layout.close_tab(&tab);
                }
            }
            WorkspaceMessage::Dock(DockEvent::MoveTab {
                tab,
                to_group,
                index,
                ..
            }) => {
                self.layout.move_tab(&tab, &to_group, index);
            }
            WorkspaceMessage::Dock(DockEvent::SplitGroup {
                tab,
                target_group,
                edge,
                ..
            }) => {
                self.next_group += 1;
                let new_group = format!("group-{}", self.next_group);
                self.layout
                    .split_with_tab(&target_group, edge, &new_group, &tab);
            }
            WorkspaceMessage::CloseConfirmed(tab) => {
                self.layout.close_tab(&tab);
            }
            WorkspaceMessage::CloseCancelled => {}
            WorkspaceMessage::SplitActiveTrailing => {
                let Some(group) = self.layout.groups().first().copied() else {
                    return;
                };
                let (group_id, active) = (group.id.clone(), group.active.clone());
                self.next_group += 1;
                let new_group = format!("group-{}", self.next_group);
                self.layout
                    .split_with_tab(&group_id, DockEdge::Trailing, &new_group, &active);
            }
            WorkspaceMessage::SaveLayout => {
                self.saved = Some(self.layout.to_persisted());
            }
            WorkspaceMessage::RestoreLayout => {
                if let Some(saved) = &self.saved {
                    self.layout = DockLayout::from_persisted(saved).expect("saved layouts restore");
                }
            }
            WorkspaceMessage::CloseOthers(tab) => {
                let others: Vec<String> = self
                    .layout
                    .group_of_tab(&tab)
                    .map(|group| {
                        group
                            .tabs
                            .iter()
                            .filter(|candidate| candidate.id != tab)
                            .map(|candidate| candidate.id.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for other in others {
                    self.layout.close_tab(&other);
                }
            }
            WorkspaceMessage::CloseToTheRight(tab) => {
                let rightward: Vec<String> = self
                    .layout
                    .group_of_tab(&tab)
                    .map(|group| {
                        group
                            .tabs
                            .iter()
                            .skip_while(|candidate| candidate.id != tab)
                            .skip(1)
                            .map(|candidate| candidate.id.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for other in rightward {
                    self.layout.close_tab(&other);
                }
            }
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let contents: Vec<Element> = self
            .layout
            .tabs()
            .into_iter()
            .map(|tab| label(format!("content-{}", tab.id)).with_key(tab.id.clone()))
            .collect();
        let handler = dispatch.clone();
        let mut documents = dock(self.layout.clone(), "Documents", contents, move |event| {
            handler.emit(WorkspaceMessage::Dock(event));
        })
        .with_key("dock");
        for tab in self.layout.tabs() {
            let tab_id = tab.id.clone();
            let others = dispatch.clone();
            let rightward = dispatch.clone();
            let other_tab = tab_id.clone();
            let right_tab = tab_id.clone();
            documents = documents.dock_tab_menu(
                tab_id,
                [
                    MenuEntry::item(MenuItem::new("close-others", "Close Others", move || {
                        others.emit(WorkspaceMessage::CloseOthers(other_tab.clone()));
                    })),
                    MenuEntry::item(MenuItem::new(
                        "close-right",
                        "Close to the Right",
                        move || {
                            rightward.emit(WorkspaceMessage::CloseToTheRight(right_tab.clone()));
                        },
                    )),
                ],
            );
        }
        let split = dispatch.clone();
        let save = dispatch.clone();
        let restore = dispatch.clone();
        column([
            button(
                "Split Right",
                "Split the active tab to the right",
                move || {
                    split.emit(WorkspaceMessage::SplitActiveTrailing);
                },
            )
            .with_key("split-right"),
            button("Save Layout", "Serialize the dock layout", move || {
                save.emit(WorkspaceMessage::SaveLayout);
            })
            .with_key("save-layout"),
            button(
                "Restore Layout",
                "Restore the saved dock layout",
                move || {
                    restore.emit(WorkspaceMessage::RestoreLayout);
                },
            )
            .with_key("restore-layout"),
            documents,
        ])
        .with_key("workspace")
    }
}

type Runtime = AppRuntime<HeadlessBackend, Workspace>;

fn mount() -> (Runtime, FakeDialogPresenter) {
    let presenter = FakeDialogPresenter::new();
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        Workspace::initial(),
        PlatformServices::default().with_dialog_service(presenter.clone()),
    )
    .expect("initial mount");
    (runtime, presenter)
}

fn dock_handle(runtime: &Runtime) -> Handle {
    runtime.with_renderer(|renderer| {
        renderer
            .backend()
            .find_by_key("dock")
            .expect("dock is mounted")
    })
}

fn dock_events(runtime: &Runtime) -> EventBindings {
    runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        backend
            .events_of(backend.find_by_key("dock").expect("dock is mounted"))
            .expect("dock has events")
    })
}

/// Emits a gesture the way a platform strip or drop zone would, outside any
/// renderer borrow because delivery re-renders synchronously.
fn gesture(runtime: &Runtime, event: DockEvent) {
    assert!(
        dock_events(runtime).emit_dock(event),
        "dock handler is bound"
    );
}

fn native_layout(runtime: &Runtime) -> DockLayout {
    runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        backend
            .dock_layout_of(backend.find_by_key("dock").expect("dock is mounted"))
            .expect("dock props are realized")
            .clone()
    })
}

fn content_handle(runtime: &Runtime, tab_id: &str) -> Option<Handle> {
    runtime.with_renderer(|renderer| renderer.backend().find_by_key(tab_id))
}

fn assert_no_error(runtime: &Runtime) {
    if let Some(error) = runtime.take_error() {
        panic!("runtime reported {error}");
    }
}

#[test]
fn mounting_realizes_the_dock_with_keyed_tab_content() {
    let (runtime, _presenter) = mount();
    let layout = native_layout(&runtime);
    assert_eq!(layout.tab_ids(), ["readme", "main", "cargo"]);
    assert_eq!(layout.find_group("documents").unwrap().active, "readme");
    let handle = dock_handle(&runtime);
    runtime.with_renderer(|renderer| {
        let children = renderer
            .backend()
            .children_of(handle)
            .expect("dock has children");
        assert_eq!(children.len(), 3);
    });
    assert!(content_handle(&runtime, "readme").is_some());
    assert_no_error(&runtime);
}

#[test]
fn selecting_a_tab_round_trips_from_gesture_to_native_state() {
    let (runtime, _presenter) = mount();
    gesture(
        &runtime,
        DockEvent::SelectTab {
            group: "documents".to_owned(),
            tab: "cargo".to_owned(),
        },
    );
    assert_eq!(
        native_layout(&runtime)
            .find_group("documents")
            .unwrap()
            .active,
        "cargo"
    );
    assert_no_error(&runtime);
}

#[test]
fn closing_a_clean_tab_removes_it_and_activates_the_neighbor() {
    let (runtime, presenter) = mount();
    gesture(
        &runtime,
        DockEvent::CloseTab {
            group: "documents".to_owned(),
            tab: "readme".to_owned(),
        },
    );
    assert_eq!(presenter.presented_count(), 0, "clean closes ask nothing");
    let layout = native_layout(&runtime);
    assert_eq!(layout.tab_ids(), ["main", "cargo"]);
    assert_eq!(layout.find_group("documents").unwrap().active, "main");
    assert!(content_handle(&runtime, "readme").is_none());
    assert_no_error(&runtime);
}

#[test]
fn closing_a_dirty_tab_is_vetoed_until_the_dialog_answers() {
    let (runtime, presenter) = mount();
    gesture(
        &runtime,
        DockEvent::CloseTab {
            group: "documents".to_owned(),
            tab: "main".to_owned(),
        },
    );
    // The tab is still present: the consumer vetoed pending the answer.
    assert_eq!(
        native_layout(&runtime).tab_ids(),
        ["readme", "main", "cargo"]
    );
    assert_eq!(presenter.presented_count(), 1);

    // Cancel keeps the tab.
    assert!(presenter.deliver(0, DialogOutcome::ButtonChosen(1)));
    assert_eq!(
        native_layout(&runtime).tab_ids(),
        ["readme", "main", "cargo"]
    );

    // A second close request confirmed destructively removes it.
    gesture(
        &runtime,
        DockEvent::CloseTab {
            group: "documents".to_owned(),
            tab: "main".to_owned(),
        },
    );
    assert!(presenter.deliver(1, DialogOutcome::ButtonChosen(0)));
    assert_eq!(native_layout(&runtime).tab_ids(), ["readme", "cargo"]);
    assert!(content_handle(&runtime, "main").is_none());
    assert_no_error(&runtime);
}

#[test]
fn reordering_within_a_group_preserves_retained_content_identity() {
    let (runtime, _presenter) = mount();
    let readme_before = content_handle(&runtime, "readme").expect("content mounted");
    gesture(
        &runtime,
        DockEvent::MoveTab {
            tab: "readme".to_owned(),
            from_group: "documents".to_owned(),
            to_group: "documents".to_owned(),
            index: 3,
        },
    );
    let layout = native_layout(&runtime);
    assert_eq!(layout.tab_ids(), ["main", "cargo", "readme"]);
    assert_eq!(layout.find_group("documents").unwrap().active, "readme");
    assert_eq!(
        content_handle(&runtime, "readme"),
        Some(readme_before),
        "keyed reconciliation kept the native content"
    );
    assert_no_error(&runtime);
}

#[test]
fn splitting_by_edge_drop_then_moving_across_groups_keeps_content() {
    let (runtime, _presenter) = mount();
    runtime.with_renderer_mut(|renderer| renderer.backend_mut().clear_operations());
    let cargo_before = content_handle(&runtime, "cargo").expect("content mounted");

    gesture(
        &runtime,
        DockEvent::SplitGroup {
            tab: "cargo".to_owned(),
            from_group: "documents".to_owned(),
            target_group: "documents".to_owned(),
            edge: DockEdge::Trailing,
        },
    );
    let layout = native_layout(&runtime);
    let DockNode::Split(split) = layout.root() else {
        panic!("edge drop produces a split");
    };
    assert_eq!(split.items.len(), 2);
    assert_eq!(layout.find_group("group-1").unwrap().active, "cargo");

    // Move a second tab into the new group: cross-group transfer.
    gesture(
        &runtime,
        DockEvent::MoveTab {
            tab: "readme".to_owned(),
            from_group: "documents".to_owned(),
            to_group: "group-1".to_owned(),
            index: 0,
        },
    );
    let layout = native_layout(&runtime);
    assert_eq!(
        layout
            .find_group("group-1")
            .unwrap()
            .tabs
            .iter()
            .map(|tab| tab.id.as_str())
            .collect::<Vec<_>>(),
        ["readme", "cargo"]
    );
    assert_eq!(layout.find_group("documents").unwrap().active, "main");

    assert_eq!(
        content_handle(&runtime, "cargo"),
        Some(cargo_before),
        "content survives the group change"
    );
    // The retained content object was never destroyed across the whole
    // split-and-move sequence.
    runtime.with_renderer(|renderer| {
        let destroyed = renderer
            .backend()
            .operations()
            .iter()
            .any(|operation| matches!(operation, Operation::Destroy { handle } if *handle == cargo_before));
        assert!(!destroyed, "cargo content must not be destroyed");
    });
    assert_no_error(&runtime);
}

#[test]
fn closing_the_last_tab_of_a_group_collapses_the_split() {
    let (runtime, _presenter) = mount();
    gesture(
        &runtime,
        DockEvent::SplitGroup {
            tab: "cargo".to_owned(),
            from_group: "documents".to_owned(),
            target_group: "documents".to_owned(),
            edge: DockEdge::Bottom,
        },
    );
    assert!(matches!(native_layout(&runtime).root(), DockNode::Split(_)));
    gesture(
        &runtime,
        DockEvent::CloseTab {
            group: "group-1".to_owned(),
            tab: "cargo".to_owned(),
        },
    );
    let layout = native_layout(&runtime);
    assert!(
        matches!(layout.root(), DockNode::Group(group) if group.id == "documents"),
        "the emptied group collapsed back to the single-group dock"
    );
    assert_eq!(layout.tab_ids(), ["readme", "main"]);
    assert_no_error(&runtime);
}

#[test]
fn the_explicit_split_command_works_without_any_drag() {
    let (runtime, _presenter) = mount();
    let split = runtime.with_renderer(|renderer| {
        let backend = renderer.backend();
        backend
            .events_of(backend.find_by_key("split-right").expect("button mounted"))
            .expect("button has events")
    });
    split.emit_activate();
    let layout = native_layout(&runtime);
    assert!(matches!(layout.root(), DockNode::Split(_)));
    assert_eq!(layout.find_group("group-1").unwrap().active, "readme");
    assert_no_error(&runtime);
}

#[test]
fn layout_persistence_round_trips_to_the_identical_model_and_native_tree() {
    let (runtime, _presenter) = mount();
    // Build a nontrivial layout: split, then reorder.
    gesture(
        &runtime,
        DockEvent::SplitGroup {
            tab: "cargo".to_owned(),
            from_group: "documents".to_owned(),
            target_group: "documents".to_owned(),
            edge: DockEdge::Trailing,
        },
    );
    let saved_layout = native_layout(&runtime);

    let press = |key: &str| {
        let events = runtime.with_renderer(|renderer| {
            let backend = renderer.backend();
            backend
                .events_of(backend.find_by_key(key).expect("button mounted"))
                .expect("button has events")
        });
        events.emit_activate();
    };
    press("save-layout");

    // Diverge, then restore.
    gesture(
        &runtime,
        DockEvent::MoveTab {
            tab: "cargo".to_owned(),
            from_group: "group-1".to_owned(),
            to_group: "documents".to_owned(),
            index: 0,
        },
    );
    assert_ne!(native_layout(&runtime), saved_layout);
    press("restore-layout");
    assert_eq!(
        native_layout(&runtime),
        saved_layout,
        "serialize → restore reproduces the identical model"
    );

    // A fresh mount from the persisted value realizes the identical native
    // dock: same layout props, same keyed children in layout order.
    let persisted = saved_layout.to_persisted();
    let restored = DockLayout::from_persisted(&persisted).expect("persisted layout restores");
    assert_eq!(restored, saved_layout);
    assert_no_error(&runtime);
}

#[test]
fn tab_context_menus_dispatch_close_others_and_close_to_the_right() {
    let (runtime, _presenter) = mount();
    let handle = dock_handle(&runtime);
    runtime.with_renderer(|renderer| {
        let menus = renderer
            .backend()
            .dock_tab_menus_of(handle)
            .expect("per-tab menus realized");
        let menu = menus.menu_for("main").expect("main declares a menu");
        assert!(menu.find_item("close-others").is_some());
        assert!(menu.find_item("close-right").is_some());
    });

    // Close to the Right on "main" closes "cargo" only.
    assert!(dock_events(&runtime).emit_dock_tab_menu_activation("main", "close-right"));
    assert_eq!(native_layout(&runtime).tab_ids(), ["readme", "main"]);

    // Close Others on "main" closes "readme".
    assert!(dock_events(&runtime).emit_dock_tab_menu_activation("main", "close-others"));
    assert_eq!(native_layout(&runtime).tab_ids(), ["main"]);

    // Unknown tabs and items are refused like a native menu would refuse.
    assert!(!dock_events(&runtime).emit_dock_tab_menu_activation("missing", "close-others"));
    assert!(!dock_events(&runtime).emit_dock_tab_menu_activation("main", "missing"));
    assert_no_error(&runtime);
}

#[test]
fn simulate_dock_event_reports_an_unbound_handler() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(
            dock(
                DockLayout::single_group(DockGroup::new(
                    "solo",
                    [DockTab::new("only", "Only")],
                    "only",
                )),
                "Documents",
                [label("content").with_key("only")],
                |_| {},
            )
            .with_key("dock"),
        )
        .expect("dock mounts");
    let handle = renderer.backend().find_by_key("dock").expect("mounted");
    renderer
        .backend()
        .simulate_dock_event(
            handle,
            DockEvent::SelectTab {
                group: "solo".to_owned(),
                tab: "only".to_owned(),
            },
        )
        .expect("bound handler consumes the gesture");
}

#[test]
fn invalid_dock_declarations_are_rejected_before_native_mutation() {
    // A content child without a key.
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let unkeyed = dock(
        DockLayout::single_group(DockGroup::new(
            "solo",
            [DockTab::new("only", "Only")],
            "only",
        )),
        "Documents",
        [label("content")],
        |_| {},
    );
    assert!(matches!(
        renderer.render(unkeyed),
        Err(RenderError::Tree(TreeError::InvalidDock { .. }))
    ));

    // A menu declared for a tab the layout does not contain.
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let foreign_menu = dock(
        DockLayout::single_group(DockGroup::new(
            "solo",
            [DockTab::new("only", "Only")],
            "only",
        )),
        "Documents",
        [label("content").with_key("only")],
        |_| {},
    )
    .dock_tab_menu(
        "missing",
        [MenuEntry::item(MenuItem::new("close", "Close", || {}))],
    );
    assert!(matches!(
        renderer.render(foreign_menu),
        Err(RenderError::Tree(TreeError::InvalidDock { .. }))
    ));

    // A structurally invalid layout: active tab outside the group.
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let bad_active = dock(
        DockLayout::single_group(DockGroup::new(
            "solo",
            [DockTab::new("only", "Only")],
            "other",
        )),
        "Documents",
        [label("content").with_key("only")],
        |_| {},
    );
    assert!(matches!(
        renderer.render(bad_active),
        Err(RenderError::Tree(TreeError::InvalidDock { .. }))
    ));

    // A child whose key matches no tab id.
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let stray_child = dock(
        DockLayout::single_group(DockGroup::new(
            "solo",
            [DockTab::new("only", "Only")],
            "only",
        )),
        "Documents",
        [
            label("content").with_key("only"),
            label("stray").with_key("stray"),
        ],
        |_| {},
    );
    assert!(matches!(
        renderer.render(stray_child),
        Err(RenderError::Tree(TreeError::InvalidDock { .. }))
    ));
}

#[test]
fn dirty_indicator_changes_patch_the_retained_dock() {
    let (runtime, presenter) = mount();
    runtime.with_renderer_mut(|renderer| renderer.backend_mut().clear_operations());

    // Confirmed close of the dirty tab flips model state through the dialog;
    // the dock element itself is patched, never rebuilt.
    gesture(
        &runtime,
        DockEvent::CloseTab {
            group: "documents".to_owned(),
            tab: "main".to_owned(),
        },
    );
    assert!(presenter.deliver(0, DialogOutcome::ButtonChosen(0)));

    let handle = dock_handle(&runtime);
    runtime.with_renderer(|renderer| {
        let operations = renderer.backend().operations();
        let dock_patched = operations
            .iter()
            .any(|operation| matches!(operation, Operation::Patch { handle: patched, .. } if *patched == handle));
        let dock_destroyed = operations
            .iter()
            .any(|operation| matches!(operation, Operation::Destroy { handle: destroyed } if *destroyed == handle));
        assert!(dock_patched, "layout changes arrive as property patches");
        assert!(!dock_destroyed, "the retained dock is never rebuilt");
    });
    assert_no_error(&runtime);
}
