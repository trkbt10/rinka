// In-process diagnostic for the native dock realization
// (`reports/document-tabs-and-splits`).
//
// `RINKA_APPKIT_DOCK_PROBE=1` drives the real explorer's dock scene after
// initial layout, entirely inside this process — no global input injection,
// no focus stealing, nothing that can land in another window:
//
// 1. The tab strip is real AppKit controls: three tab buttons carry titles,
//    toggle state, and accessibility exposure (dumped when
//    `RINKA_APPKIT_DOCK_PROBE_AX_DUMP=<path>` is set).
// 2. Selection through the button's own action path reconciles the model.
// 3. The dirty/close indicator anatomy flips deterministically on the item's
//    own hover handlers.
// 4. The explicit split command grows a real NSSplitView whose weights land
//    as divider positions.
// 5. Tab drops are served protocol-level: a constructed NSDraggingInfo
//    double over a uniquely named pasteboard hits the strip's and the
//    content host's own performDragOperation:, moving a tab across groups
//    and splitting by edge drop.
// 6. Closing a dirty tab is vetoed until the native confirmation sheet
//    answers; closing the last tab of a group collapses its split.
// 7. Per-tab context menus dispatch Close Others through their real targets.
// 8. Save Layout / Restore Layout round-trips the whole arrangement.
//
// Every step prints one `Rinka dock probe` line and the process terminates
// after the summary line.

impl ApplicationDelegate {
    fn begin_dock_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_DOCK_PROBE").is_none() {
            return;
        }
        if std::env::var_os("RINKA_APPKIT_SCENE_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_ACCELERATOR_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_CONTEXT_MENU_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_DRAG_DROP_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_DIALOG_PROBE").is_some()
        {
            panic!("the dock probe must run in its own process");
        }
        let content = {
            let windows = self.ivars().windows.borrow();
            windows.first().and_then(|window| {
                // SAFETY: The retained main window is read on the main
                // thread after initial layout.
                let pointer: *mut AnyObject =
                    unsafe { msg_send![window.as_object(), contentView] };
                NonNull::new(pointer)
            })
        };
        let Some(content) = content else {
            eprintln!("Rinka dock probe step=locate-window pass=false");
            eprintln!("Rinka dock probe result=FAIL");
            terminate_dock_probe();
            return;
        };
        // SAFETY: Every step reads and drives retained AppKit objects on the
        // main thread through their public selectors or same-crate methods.
        let passed = unsafe { self.run_dock_probe(content.as_ref()) };
        eprintln!(
            "Rinka dock probe result={}",
            if passed { "PASS" } else { "FAIL" }
        );
        terminate_dock_probe();
    }

    /// Runs the whole dock sequence; each step prints its own verdict.
    ///
    /// # Safety
    ///
    /// `content` must be the live main-window content view on AppKit's main
    /// thread.
    unsafe fn run_dock_probe(&self, content: &AnyObject) -> bool {
        // SAFETY: Guaranteed by the caller; every helper below reads live
        // main-thread views.
        unsafe {
            let dock_view = self.dock_probe_area_view();
            let Some(dock_view) = dock_view else {
                eprintln!("Rinka dock probe step=locate-dock pass=false");
                return false;
            };
            let dock_view = dock_view.as_object();

            // 1. Locate the three tabs with their titles and toggle state.
            let buttons = dock_tab_button_titles(dock_view);
            let located = buttons
                == vec![
                    ("view.rs".to_owned(), 1),
                    ("Test Pattern".to_owned(), 0),
                    ("notes.md".to_owned(), 0),
                ]
                && count_views_of_class(dock_view, DockStripView::class()) == 1;
            eprintln!("Rinka dock probe step=locate pass={located} buttons={buttons:?}");
            if !located {
                return false;
            }
            let dumped = dump_dock_ax_tree(dock_view);
            eprintln!("Rinka dock probe step=ax-dump pass={dumped}");
            if !dumped {
                return false;
            }

            // 2. Select the canvas tab through the button's action path —
            //    the same route the accessibility press takes.
            let selected = self.dock_probe_press_tab(dock_view, "Test Pattern")
                && self.dock_probe_note(content) == "dock: selected canvas"
                && dock_tab_button_titles(dock_view)
                    == vec![
                        ("view.rs".to_owned(), 0),
                        ("Test Pattern".to_owned(), 1),
                        ("notes.md".to_owned(), 0),
                    ];
            eprintln!("Rinka dock probe step=select pass={selected}");
            if !selected {
                return false;
            }

            // 3. Hover anatomy on the dirty notes tab: dot at rest, close on
            //    hover, dot restored on exit.
            let anatomy = self.dock_probe_hover_anatomy(dock_view);
            eprintln!("Rinka dock probe step=hover-anatomy pass={anatomy}");
            if !anatomy {
                return false;
            }

            // 3b. The dirty indicator follows the consumer's flag: marking
            //     the active canvas tab dirty shows its dot at rest, and
            //     clearing the flag removes it.
            let dot_visible = |title: &str| -> Option<bool> {
                let item = find_dock_tab_item(dock_view, title)?;
                let slot = item.as_ref().ivars().dot_label.borrow();
                let view = slot.as_ref()?;
                let hidden: bool = msg_send![view.as_object(), isHidden];
                Some(!hidden)
            };
            let dirty_toggle = self.dock_probe_click_mounted_button("dock-mark-dirty")
                && self.dock_probe_note(content) == "dock: canvas dirty=true"
                && dot_visible("Test Pattern") == Some(true)
                && self.dock_probe_click_mounted_button("dock-mark-dirty")
                && self.dock_probe_note(content) == "dock: canvas dirty=false"
                && dot_visible("Test Pattern") == Some(false);
            eprintln!("Rinka dock probe step=dirty-indicator pass={dirty_toggle}");
            if !dirty_toggle {
                return false;
            }

            // 4. Explicit split command: the active canvas tab moves into a
            //    new trailing group realized by a real NSSplitView.
            let split = self.dock_probe_click_mounted_button("dock-split-right")
                && count_views_of_class(dock_view, objc2::class!(NSSplitView)) == 1
                && count_views_of_class(dock_view, DockStripView::class()) == 2
                && self
                    .dock_probe_note(content)
                    .starts_with("dock: split documents");
            eprintln!("Rinka dock probe step=split-command pass={split}");
            if !split {
                return false;
            }

            // 5. Weights land as divider positions once layout settles.
            self.dock_probe_pump(0.3);
            let fraction = self.dock_probe_first_split_fraction();
            let weights = fraction.is_some_and(|value| (0.3..=0.7).contains(&value));
            eprintln!(
                "Rinka dock probe step=weights pass={weights} fraction={fraction:?}"
            );
            if !weights {
                return false;
            }

            // 6. Protocol-level strip drop: notes moves from the documents
            //    group into group-1 through the strip's own drop selector.
            let moved = self.dock_probe_strip_move(dock_view)
                && self.dock_probe_note(content) == "dock: moved notes to group-1@1";
            eprintln!("Rinka dock probe step=strip-move-drop pass={moved}");
            if !moved {
                return false;
            }

            // 6b. Protocol-level within-group reorder: notes dropped before
            //     the canvas tab in the same strip reorders the group.
            let reordered = self.dock_probe_strip_reorder(dock_view)
                && self.dock_probe_note(content) == "dock: moved notes to group-1@0"
                && strip_tab_titles(dock_view, "group-1")
                    == vec!["notes.md".to_owned(), "Test Pattern".to_owned()];
            eprintln!("Rinka dock probe step=strip-reorder-drop pass={reordered}");
            if !reordered {
                return false;
            }

            // 7. Protocol-level edge drop: notes dropped on the documents
            //    content's bottom band splits it into a third group.
            let edge_split = self.dock_probe_edge_drop(dock_view)
                && count_views_of_class(dock_view, DockStripView::class()) == 3
                && self.dock_probe_note(content) == "dock: split documents Bottom with notes";
            eprintln!("Rinka dock probe step=edge-drop-split pass={edge_split}");
            if !edge_split {
                return false;
            }
            flush_window_rendering(self.ivars());
            self.capture_windows_to_directory("dock-split-");

            // Save the three-group arrangement for the persistence check.
            let saved = self.dock_probe_click_mounted_button("dock-save-layout")
                && self
                    .dock_probe_note(content)
                    .starts_with("dock: saved layout");
            eprintln!("Rinka dock probe step=save-layout pass={saved}");
            if !saved {
                return false;
            }

            // 8. Dirty close veto: cancel keeps the tab, an explicit Close
            //    through the native sheet removes it and collapses group-2.
            let veto = self.dock_probe_close_veto(content, dock_view);
            eprintln!("Rinka dock probe step=close-veto pass={veto}");
            if !veto {
                return false;
            }

            // 9. Closing the last tab of group-1 collapses the whole split.
            let collapsed = self.dock_probe_close_tab(dock_view, "Test Pattern")
                && self.dock_probe_note(content) == "dock: closed canvas"
                && count_views_of_class(dock_view, DockStripView::class()) == 1
                && count_views_of_class(dock_view, objc2::class!(NSSplitView)) == 0;
            eprintln!("Rinka dock probe step=close-last-collapses pass={collapsed}");
            if !collapsed {
                return false;
            }

            // 10. The per-tab context menu dispatches through its real
            //     native items.
            let menu = self.dock_probe_tab_menu(content, dock_view);
            eprintln!("Rinka dock probe step=tab-menu pass={menu}");
            if !menu {
                return false;
            }

            // 11. Restoring the saved layout brings the three-group
            //     arrangement — tabs included — back.
            let restored = self.dock_probe_click_mounted_button("dock-restore-layout")
                && self.dock_probe_note(content) == "dock: restored layout"
                && count_views_of_class(dock_view, DockStripView::class()) == 3
                && dock_tab_button_titles(dock_view).len() == 3;
            eprintln!("Rinka dock probe step=restore-layout pass={restored}");
            if !restored {
                return false;
            }
            self.dock_probe_pump(0.3);
            flush_window_rendering(self.ivars());
            self.capture_windows_to_directory("dock-final-");
            true
        }
    }

    /// Returns the mounted dock element's outer view.
    fn dock_probe_area_view(&self) -> Option<Id> {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().and_then(|runtime| {
            runtime.with_renderer(|renderer| {
                renderer
                    .mounted()
                    .and_then(|root| mounted_handle_for_key(root, "dock-area"))
                    .map(|handle| handle.0.view.clone())
            })
        })
    }

    /// Reads the dock scene's assertable status note.
    fn dock_probe_note(&self, content: &AnyObject) -> String {
        // SAFETY: The traversal reads retained subviews on the main thread.
        unsafe { find_dock_note(content).unwrap_or_default() }
    }

    /// Presses a tab button by its visible title through `performClick:`.
    ///
    /// # Safety
    ///
    /// `dock_view` must be a live main-thread view.
    unsafe fn dock_probe_press_tab(&self, dock_view: &AnyObject, title: &str) -> bool {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            let Some(button) = find_dock_tab_button(dock_view, title) else {
                return false;
            };
            let _: () = msg_send![button.as_ref(), performClick: std::ptr::null::<AnyObject>()];
            flush_window_rendering(self.ivars());
            true
        }
    }

    /// Presses a mounted explorer button (declarative key) natively.
    ///
    /// The view is resolved and every renderer borrow released before the
    /// click, because the button's action re-renders synchronously.
    fn dock_probe_click_mounted_button(&self, key: &str) -> bool {
        let view = {
            let renderers = self.ivars().renderers.borrow();
            renderers.first().and_then(|runtime| {
                runtime.with_renderer(|renderer| {
                    renderer
                        .mounted()
                        .and_then(|root| mounted_handle_for_key(root, key))
                        .map(|handle| handle.0.view.clone())
                })
            })
        };
        let Some(view) = view else {
            return false;
        };
        // SAFETY: The key identifies a live mounted NSButton; performClick
        // drives its ordinary action path with no adapter borrow held.
        unsafe {
            let _: () = msg_send![view.as_object(), performClick: std::ptr::null::<AnyObject>()];
        }
        flush_window_rendering(self.ivars());
        true
    }

    /// Turns the run loop so sheet presentation and deferred weight
    /// application complete, without injecting any input.
    fn dock_probe_pump(&self, seconds: f64) {
        // SAFETY: Running the default mode on the main thread processes the
        // application's own queued work only.
        unsafe {
            let run_loop: *mut AnyObject = msg_send![objc2::class!(NSRunLoop), currentRunLoop];
            let deadline: *mut AnyObject = msg_send![
                objc2::class!(NSDate),
                dateWithTimeIntervalSinceNow: seconds
            ];
            let _: () = msg_send![run_loop, runUntilDate: deadline];
        }
    }

    /// Reads the first split's leading-pane fraction of the total extent.
    fn dock_probe_first_split_fraction(&self) -> Option<f64> {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().and_then(|runtime| {
            runtime.with_renderer(|renderer| {
                let root = renderer.mounted()?;
                let handle = mounted_handle_for_key(root, "dock-area")?;
                let state = handle.0.dock.borrow();
                let split = state.as_ref()?.splits.first().map(|split| split.view.clone())?;
                // SAFETY: The retained split view and its arranged panes are
                // measured on the main thread.
                unsafe {
                    let bounds: Rect = msg_send![split.as_object(), bounds];
                    let subviews: *mut AnyObject = msg_send![split.as_object(), arrangedSubviews];
                    let count: usize = msg_send![subviews, count];
                    if count < 2 || bounds.size.width < 1.0 {
                        return None;
                    }
                    let first: *mut AnyObject = msg_send![subviews, objectAtIndex: 0_usize];
                    let frame: Rect = msg_send![first, frame];
                    Some(frame.size.width / bounds.size.width)
                }
            })
        })
    }

    /// Verifies the dirty-dot / hover-close anatomy on the notes tab.
    ///
    /// # Safety
    ///
    /// `dock_view` must be a live main-thread view.
    unsafe fn dock_probe_hover_anatomy(&self, dock_view: &AnyObject) -> bool {
        // SAFETY: Guaranteed by the caller; the item's hover handlers are
        // same-crate methods driven directly, no synthetic input.
        unsafe {
            let Some(item) = find_dock_tab_item(dock_view, "notes.md") else {
                return false;
            };
            let item = item.as_ref();
            let visible = |slot: &RefCell<Option<Id>>| -> Option<bool> {
                let slot = slot.borrow();
                let view = slot.as_ref()?;
                let hidden: bool = msg_send![view.as_object(), isHidden];
                Some(!hidden)
            };
            let at_rest = visible(&item.ivars().dot_label) == Some(true)
                && visible(&item.ivars().close_button) == Some(false);
            let _: () = msg_send![item, mouseEntered: std::ptr::null::<AnyObject>()];
            let hovered = visible(&item.ivars().dot_label) == Some(false)
                && visible(&item.ivars().close_button) == Some(true);
            let _: () = msg_send![item, mouseExited: std::ptr::null::<AnyObject>()];
            let restored = visible(&item.ivars().dot_label) == Some(true)
                && visible(&item.ivars().close_button) == Some(false);
            at_rest && hovered && restored
        }
    }

    /// Drops the notes tab onto the end of group-1's strip through the
    /// strip's own drag-destination selector.
    ///
    /// # Safety
    ///
    /// `dock_view` must be a live main-thread view.
    unsafe fn dock_probe_strip_move(&self, dock_view: &AnyObject) -> bool {
        // SAFETY: Guaranteed by the caller; the pasteboard is uniquely named
        // and released, and the info double points only at it.
        unsafe {
            let Some(strip) = find_dock_strip(dock_view, "group-1") else {
                return false;
            };
            let strip = strip.as_ref();
            let stack = strip.ivars().stack.borrow().clone();
            let Some(stack) = stack else {
                return false;
            };
            let bounds: Rect = msg_send![stack.as_object(), bounds];
            let end = Point {
                x: bounds.origin.x + bounds.size.width + 24.0,
                y: bounds.origin.y + bounds.size.height / 2.0,
            };
            let location: Point = msg_send![stack.as_object(),
                convertPoint: end,
                toView: std::ptr::null::<AnyObject>()
            ];
            self.dock_probe_perform_drop(strip as &AnyObject as *const AnyObject, "notes", location)
        }
    }

    /// Drops the notes tab before the first item of group-1's own strip:
    /// the within-group reorder gesture.
    ///
    /// # Safety
    ///
    /// `dock_view` must be a live main-thread view.
    unsafe fn dock_probe_strip_reorder(&self, dock_view: &AnyObject) -> bool {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            let Some(strip) = find_dock_strip(dock_view, "group-1") else {
                return false;
            };
            let strip = strip.as_ref();
            let stack = strip.ivars().stack.borrow().clone();
            let Some(stack) = stack else {
                return false;
            };
            let bounds: Rect = msg_send![stack.as_object(), bounds];
            let start = Point {
                x: bounds.origin.x + 1.0,
                y: bounds.origin.y + bounds.size.height / 2.0,
            };
            let location: Point = msg_send![stack.as_object(),
                convertPoint: start,
                toView: std::ptr::null::<AnyObject>()
            ];
            self.dock_probe_perform_drop(strip as &AnyObject as *const AnyObject, "notes", location)
        }
    }

    /// Drops the notes tab onto the bottom edge band of the documents
    /// group's content host.
    ///
    /// # Safety
    ///
    /// `dock_view` must be a live main-thread view.
    unsafe fn dock_probe_edge_drop(&self, dock_view: &AnyObject) -> bool {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            let Some(host) = find_dock_content_host(dock_view, "documents") else {
                return false;
            };
            let host = host.as_ref();
            let bounds: Rect = msg_send![host, bounds];
            let flipped: bool = msg_send![host, isFlipped];
            // The drop helper converts into top-left space; aim at the
            // bottom band in the view's own coordinates.
            let y = if flipped {
                bounds.origin.y + bounds.size.height * 0.9
            } else {
                bounds.origin.y + bounds.size.height * 0.1
            };
            let point = Point {
                x: bounds.origin.x + bounds.size.width / 2.0,
                y,
            };
            let location: Point = msg_send![host,
                convertPoint: point,
                toView: std::ptr::null::<AnyObject>()
            ];
            self.dock_probe_perform_drop(host as &AnyObject as *const AnyObject, "notes", location)
        }
    }

    /// Serves one constructed dock-tab drop through a destination view's own
    /// dragging selectors.
    ///
    /// # Safety
    ///
    /// `destination` must be a live main-thread dragging-destination view.
    unsafe fn dock_probe_perform_drop(
        &self,
        destination: *const AnyObject,
        tab_id: &str,
        location: Point,
    ) -> bool {
        // SAFETY: Guaranteed by the caller; the pasteboard is uniquely named
        // and globally released after the drop.
        unsafe {
            let pasteboard = probe_pasteboard("dock");
            let encoded = format!("{DOCK_TAB_PAYLOAD_TYPE}\n{tab_id}");
            let _: bool = msg_send![pasteboard.as_object(),
                setString: ns_string(&encoded).as_object(),
                forType: payload_pasteboard_type().as_object()
            ];
            let mtm = MainThreadMarker::new().expect("probe runs on the main thread");
            let info = DragInfoDouble::new(mtm, pasteboard.clone(), location);
            let entered: usize = msg_send![&*destination, draggingEntered: &*info];
            let performed: bool = if entered == DRAG_OPERATION_MOVE {
                msg_send![&*destination, performDragOperation: &*info]
            } else {
                false
            };
            release_probe_pasteboard(&pasteboard);
            flush_window_rendering(self.ivars());
            performed
        }
    }

    /// Presses a tab's close affordance: hover reveals the real close
    /// button, then its action path runs.
    ///
    /// # Safety
    ///
    /// `dock_view` must be a live main-thread view.
    unsafe fn dock_probe_close_tab(&self, dock_view: &AnyObject, title: &str) -> bool {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            let Some(item) = find_dock_tab_item(dock_view, title) else {
                return false;
            };
            let item = item.as_ref();
            let _: () = msg_send![item, mouseEntered: std::ptr::null::<AnyObject>()];
            let close = item.ivars().close_button.borrow().clone();
            let Some(close) = close else {
                return false;
            };
            let hidden: bool = msg_send![close.as_object(), isHidden];
            if hidden {
                return false;
            }
            let _: () = msg_send![close.as_object(), performClick: std::ptr::null::<AnyObject>()];
            flush_window_rendering(self.ivars());
            true
        }
    }

    /// Runs the dirty-close veto round trip on the notes tab: Cancel keeps
    /// it, Close through the native sheet removes it and collapses group-2.
    ///
    /// # Safety
    ///
    /// `content` and `dock_view` must be live main-thread views.
    unsafe fn dock_probe_close_veto(&self, content: &AnyObject, dock_view: &AnyObject) -> bool {
        // SAFETY: Guaranteed by the caller; sheet buttons are pressed
        // through their own performClick.
        unsafe {
            if !self.dock_probe_close_tab(dock_view, "notes.md") {
                eprintln!("Rinka dock probe detail=veto-close-press pass=false");
                return false;
            }
            if self.dock_probe_note(content) != "dock: close requested for dirty notes" {
                eprintln!("Rinka dock probe detail=veto-note pass=false");
                return false;
            }
            if !self.dock_probe_answer_sheet("Cancel") {
                eprintln!("Rinka dock probe detail=veto-cancel pass=false");
                return false;
            }
            if self.dock_probe_note(content) != "dock: close cancelled"
                || count_views_of_class(dock_view, DockStripView::class()) != 3
            {
                eprintln!("Rinka dock probe detail=veto-cancel-kept pass=false");
                return false;
            }
            if !self.dock_probe_close_tab(dock_view, "notes.md") {
                eprintln!("Rinka dock probe detail=veto-second-press pass=false");
                return false;
            }
            if !self.dock_probe_answer_sheet("Close") {
                eprintln!("Rinka dock probe detail=veto-confirm pass=false");
                return false;
            }
            let closed = self.dock_probe_note(content) == "dock: closed notes"
                && count_views_of_class(dock_view, DockStripView::class()) == 2;
            if !closed {
                eprintln!("Rinka dock probe detail=veto-collapse pass=false");
            }
            closed
        }
    }

    /// Waits for the native confirmation sheet and presses one button.
    fn dock_probe_answer_sheet(&self, title: &str) -> bool {
        for _ in 0..40 {
            let sheet = {
                let windows = self.ivars().windows.borrow();
                let Some(window) = windows.first() else {
                    return false;
                };
                // SAFETY: The retained window's attached sheet is read on
                // the main thread.
                unsafe {
                    let sheet: *mut AnyObject = msg_send![window.as_object(), attachedSheet];
                    NonNull::new(sheet)
                }
            };
            if let Some(sheet) = sheet {
                // SAFETY: The sheet's content view and its buttons are live;
                // performClick runs the alert's ordinary completion path.
                unsafe {
                    let sheet_content: *mut AnyObject =
                        msg_send![sheet.as_ref(), contentView];
                    let Some(sheet_content) = NonNull::new(sheet_content) else {
                        return false;
                    };
                    let Some(button) = find_button_titled(sheet_content.as_ref(), title) else {
                        return false;
                    };
                    let _: () =
                        msg_send![button.as_ref(), performClick: std::ptr::null::<AnyObject>()];
                }
                // Let the sheet dismissal and the queued outcome message
                // reconcile before asserting.
                self.dock_probe_pump(0.1);
                flush_window_rendering(self.ivars());
                return true;
            }
            self.dock_probe_pump(0.05);
        }
        false
    }

    /// Activates "Close Others" on the editor tab through the real native
    /// menu item's target.
    ///
    /// # Safety
    ///
    /// `content` and `dock_view` must be live main-thread views.
    unsafe fn dock_probe_tab_menu(&self, content: &AnyObject, dock_view: &AnyObject) -> bool {
        // SAFETY: Guaranteed by the caller; the retained menu items carry
        // their own retained targets.
        unsafe {
            let Some(item) = find_dock_tab_item(dock_view, "view.rs") else {
                return false;
            };
            let menu: *mut AnyObject = msg_send![item.as_ref(), menu];
            let Some(menu) = NonNull::new(menu) else {
                return false;
            };
            let count: isize = msg_send![menu.as_ref(), numberOfItems];
            if count != 2 {
                return false;
            }
            let first: *mut AnyObject = msg_send![menu.as_ref(), itemAtIndex: 0_isize];
            let second: *mut AnyObject = msg_send![menu.as_ref(), itemAtIndex: 1_isize];
            let first_title: *mut AnyObject = msg_send![first, title];
            let second_title: *mut AnyObject = msg_send![second, title];
            if rust_string(first_title) != "Close Others"
                || rust_string(second_title) != "Close to the Right"
            {
                return false;
            }
            let target: *mut AnyObject = msg_send![first, target];
            let Some(target) = NonNull::new(target) else {
                return false;
            };
            let _: () = msg_send![target.as_ref(), performAction: first];
            flush_window_rendering(self.ivars());
            self.dock_probe_note(content) == "dock: closed others of editor"
        }
    }
}

unsafe extern "C" {
    /// Resolves the assistive element AppKit actually exposes for a view.
    fn NSAccessibilityUnignoredDescendant(element: *mut AnyObject) -> *mut AnyObject;
}

fn terminate_dock_probe() {
    if std::env::var_os("RINKA_APPKIT_DOCK_PROBE_HOLD").is_some() {
        return;
    }
    // SAFETY: Diagnostic completion terminates only the current test app.
    unsafe {
        let application: *mut AnyObject =
            msg_send![objc2::class!(NSApplication), sharedApplication];
        let _: () = msg_send![application, terminate: std::ptr::null::<AnyObject>()];
    }
}

/// Collects every view of `class` under `view` in traversal order.
///
/// # Safety
///
/// `view` must be a live NSView used on AppKit's main thread.
unsafe fn collect_views_of_class(
    view: &AnyObject,
    class: &objc2::runtime::AnyClass,
    found: &mut Vec<NonNull<AnyObject>>,
) {
    // SAFETY: The traversal reads retained subviews on the main thread.
    unsafe {
        let is_match: bool = msg_send![view, isKindOfClass: class];
        if is_match {
            found.push(NonNull::from(view));
        }
        let subviews: *mut AnyObject = msg_send![view, subviews];
        let Some(subviews) = NonNull::new(subviews) else {
            return;
        };
        let count: usize = msg_send![subviews.as_ref(), count];
        for index in 0..count {
            let child: *mut AnyObject = msg_send![subviews.as_ref(), objectAtIndex: index];
            if let Some(child) = NonNull::new(child) {
                collect_views_of_class(child.as_ref(), class, found);
            }
        }
    }
}

/// Counts views of `class` under `view`.
///
/// # Safety
///
/// `view` must be a live NSView used on AppKit's main thread.
unsafe fn count_views_of_class(view: &AnyObject, class: &objc2::runtime::AnyClass) -> usize {
    let mut found = Vec::new();
    // SAFETY: Guaranteed by the caller.
    unsafe {
        collect_views_of_class(view, class, &mut found);
    }
    found.len()
}

/// Returns every dock tab button's `(title, state)` in traversal order.
///
/// # Safety
///
/// `dock_view` must be a live NSView used on AppKit's main thread.
unsafe fn dock_tab_button_titles(dock_view: &AnyObject) -> Vec<(String, isize)> {
    let mut found = Vec::new();
    // SAFETY: Guaranteed by the caller; title and state are public NSButton
    // properties.
    unsafe {
        collect_views_of_class(dock_view, DockTabButton::class(), &mut found);
        found
            .into_iter()
            .map(|button| {
                let title: *mut AnyObject = msg_send![button.as_ref(), title];
                let state: isize = msg_send![button.as_ref(), state];
                (rust_string(title), state)
            })
            .collect()
    }
}

/// Finds one dock tab button by its visible title.
///
/// # Safety
///
/// `dock_view` must be a live NSView used on AppKit's main thread.
unsafe fn find_dock_tab_button(dock_view: &AnyObject, title: &str) -> Option<NonNull<AnyObject>> {
    let mut found = Vec::new();
    // SAFETY: Guaranteed by the caller.
    unsafe {
        collect_views_of_class(dock_view, DockTabButton::class(), &mut found);
        found.into_iter().find(|button| {
            let native: *mut AnyObject = msg_send![button.as_ref(), title];
            rust_string(native) == title
        })
    }
}

/// Finds one tab item container by its button title.
///
/// # Safety
///
/// `dock_view` must be a live NSView used on AppKit's main thread.
unsafe fn find_dock_tab_item(
    dock_view: &AnyObject,
    title: &str,
) -> Option<NonNull<DockTabItemView>> {
    // SAFETY: Guaranteed by the caller; the cast target's class is checked
    // by the superview walk.
    unsafe {
        let button = find_dock_tab_button(dock_view, title)?;
        let mut view: *mut AnyObject = msg_send![button.as_ref(), superview];
        while let Some(current) = NonNull::new(view) {
            let is_item: bool =
                msg_send![current.as_ref(), isKindOfClass: DockTabItemView::class()];
            if is_item {
                return Some(current.cast::<DockTabItemView>());
            }
            view = msg_send![current.as_ref(), superview];
        }
        None
    }
}

/// Returns the tab titles of one group's strip in strip order.
///
/// # Safety
///
/// `dock_view` must be a live NSView used on AppKit's main thread.
unsafe fn strip_tab_titles(dock_view: &AnyObject, group_id: &str) -> Vec<String> {
    // SAFETY: Guaranteed by the caller; arrangedSubviews is the strip's
    // authoritative visual order (plain subview order can drift after
    // arranged reinsertion).
    unsafe {
        let Some(strip) = find_dock_strip(dock_view, group_id) else {
            return Vec::new();
        };
        let stack = strip.as_ref().ivars().stack.borrow().clone();
        let Some(stack) = stack else {
            return Vec::new();
        };
        let arranged: *mut AnyObject = msg_send![stack.as_object(), arrangedSubviews];
        let count: usize = msg_send![arranged, count];
        (0..count)
            .filter_map(|index| {
                let item: *mut AnyObject = msg_send![arranged, objectAtIndex: index];
                let item = NonNull::new(item)?;
                let mut buttons = Vec::new();
                collect_views_of_class(item.as_ref(), DockTabButton::class(), &mut buttons);
                buttons.first().map(|button| {
                    let title: *mut AnyObject = msg_send![button.as_ref(), title];
                    rust_string(title)
                })
            })
            .collect()
    }
}

/// Finds one group's strip by its group id.
///
/// # Safety
///
/// `dock_view` must be a live NSView used on AppKit's main thread.
unsafe fn find_dock_strip(dock_view: &AnyObject, group_id: &str) -> Option<NonNull<DockStripView>> {
    let mut found = Vec::new();
    // SAFETY: Guaranteed by the caller; the ivars read belongs to this
    // crate's own class.
    unsafe {
        collect_views_of_class(dock_view, DockStripView::class(), &mut found);
        found
            .into_iter()
            .map(|view| view.cast::<DockStripView>())
            .find(|strip| *strip.as_ref().ivars().group_id.borrow() == group_id)
    }
}

/// Finds one group's content host by its group id.
///
/// # Safety
///
/// `dock_view` must be a live NSView used on AppKit's main thread.
unsafe fn find_dock_content_host(
    dock_view: &AnyObject,
    group_id: &str,
) -> Option<NonNull<DockContentHostView>> {
    let mut found = Vec::new();
    // SAFETY: Guaranteed by the caller; the ivars read belongs to this
    // crate's own class.
    unsafe {
        collect_views_of_class(dock_view, DockContentHostView::class(), &mut found);
        found
            .into_iter()
            .map(|view| view.cast::<DockContentHostView>())
            .find(|host| *host.as_ref().ivars().group_id.borrow() == group_id)
    }
}

/// Finds the dock scene's status note label text.
///
/// # Safety
///
/// `view` must be a live NSView used on AppKit's main thread.
unsafe fn find_dock_note(view: &AnyObject) -> Option<String> {
    // SAFETY: Guaranteed by the caller; stringValue is public NSTextField
    // API and the traversal reads retained subviews.
    unsafe {
        let is_field: bool = msg_send![view, isKindOfClass: objc2::class!(NSTextField)];
        if is_field {
            let value: *mut AnyObject = msg_send![view, stringValue];
            let text = rust_string(value);
            if text.starts_with("dock:") {
                return Some(text);
            }
        }
        let subviews: *mut AnyObject = msg_send![view, subviews];
        let subviews = NonNull::new(subviews)?;
        let count: usize = msg_send![subviews.as_ref(), count];
        (0..count).find_map(|index| {
            let child: *mut AnyObject = msg_send![subviews.as_ref(), objectAtIndex: index];
            NonNull::new(child).and_then(|child| find_dock_note(child.as_ref()))
        })
    }
}

/// Writes the deterministic accessibility extract of the tab strip: one
/// sorted line per tab button carrying class, role, label, and value.
///
/// # Safety
///
/// `dock_view` must be a live NSView used on AppKit's main thread.
unsafe fn dump_dock_ax_tree(dock_view: &AnyObject) -> bool {
    let Some(path) = std::env::var_os("RINKA_APPKIT_DOCK_PROBE_AX_DUMP") else {
        return true;
    };
    let mut found = Vec::new();
    // SAFETY: Guaranteed by the caller; the accessibility getters are
    // public NSAccessibility protocol methods.
    let mut lines: Vec<String> = unsafe {
        collect_views_of_class(dock_view, DockTabButton::class(), &mut found);
        found
            .into_iter()
            .map(|button| {
                let class_name: *mut AnyObject = msg_send![button.as_ref(), className];
                // AppKit's view/cell accessibility split: the assistive
                // element for an NSButton is its unignored descendant (the
                // cell), which carries the effective role.
                let element = NSAccessibilityUnignoredDescendant(button.as_ptr());
                let element = NonNull::new(element).unwrap_or(button);
                let role: *mut AnyObject = msg_send![element.as_ref(), accessibilityRole];
                let label: *mut AnyObject = msg_send![button.as_ref(), accessibilityLabel];
                let state: isize = msg_send![button.as_ref(), state];
                format!(
                    "class={} role={} label={} selected={}",
                    rust_string(class_name),
                    rust_string(role),
                    rust_string(label),
                    state
                )
            })
            .collect()
    };
    lines.sort();
    std::fs::write(std::path::PathBuf::from(path), lines.join("\n") + "\n").is_ok()
}
