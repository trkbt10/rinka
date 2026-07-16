// In-process diagnostic for the native application menu bar.
//
// `RINKA_APPKIT_MENU_BAR_PROBE=1` drives the real explorer after initial
// layout: it asserts the installed NSApplication.mainMenu structure (the
// synthesized application menu plus File/Edit/View/Window/Help, the Window
// and Help menu designations, the standard edit roles, and the declared key
// equivalents), activates menu items through their native target/action
// pairs and through real key-equivalent events posted to the queue, and
// asserts that checkmark reconciliation, validation-gated dispatch, and the
// zero-consumer-code edit roles all reach the retained native objects. An
// accessibility extract of the complete menu tree is logged before and
// after the state changes. When `RINKA_APPKIT_MENU_BAR_PROBE_CAPTURE_DIR`
// names a directory, the probe photographs its own windows — including the
// opened View menu — through the window server's self-capture path.
//
// `RINKA_APPKIT_MENU_BAR_PROBE_FINISH` selects the native exit path proven
// by the run: `quit` activates the application menu's Quit item and `close`
// activates File > Close Window (terminating through the
// last-window-closed path); anything else terminates directly. Every step
// prints one `Rinka menu-bar probe` line before the process exits.
//
// Following the clipboard probe's precedent, the probe never requires OS
// activation, so it stays deterministic on a busy desktop and never steals
// the user's focus. Menu structure, checkmark reconciliation, native
// validation, and key-equivalent dispatch of posted events all work without
// activation (the main menu matches key equivalents regardless). The one
// thing an inactive application cannot do is resolve a nil-target action
// through the key window — there is none — so responder-chain dispatch is
// then anchored at the primary window's first responder with
// `tryToPerform:with:`, the same chain walk minus the key-window lookup;
// each dispatch logs which mechanism carried it.

/// Selected search-field content the native Edit roles must transfer.
const MENU_BAR_COPY_MARKER: &str = "rinka menu bar edit roles 検証";

impl ApplicationDelegate {
    fn begin_menu_bar_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_MENU_BAR_PROBE").is_none()
            || self.ivars().menu_bar_probe.borrow().is_some()
        {
            return;
        }
        if std::env::var_os("RINKA_APPKIT_SCENE_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_ACCELERATOR_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_CLIPBOARD_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_CONTEXT_MENU_PROBE").is_some()
        {
            panic!("the menu bar probe must run in its own process");
        }
        *self.ivars().menu_bar_probe.borrow_mut() = Some(MenuBarProbe {
            step: 0,
            attempts: 0,
            passed: true,
        });
        self.schedule_menu_bar_probe();
    }

    fn schedule_menu_bar_probe(&self) {
        // SAFETY: The next main-loop turn runs after posted key events have
        // dispatched and any resulting reconciliation has completed.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runMenuBarProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.05_f64
            ];
        }
    }

    fn fail_menu_bar_probe_step(&self, step: &'static str, detail: &str) {
        eprintln!("Rinka menu-bar probe step={step} {detail} pass=false");
        if let Some(probe) = self.ivars().menu_bar_probe.borrow_mut().as_mut() {
            probe.passed = false;
        }
        self.finish_menu_bar_probe();
    }

    /// Reads the explorer's mounted file-action note, if present.
    fn probe_file_action_note(&self) -> Option<String> {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().and_then(|runtime| {
            runtime.with_renderer(|renderer| {
                renderer
                    .mounted()
                    .and_then(|root| mounted_label_text(root, "file-action-note"))
            })
        })
    }

    fn advance_menu_bar_probe(&self) {
        const MAX_MAIN_LOOP_TURNS: usize = 200;
        // Turns a refused chord is given to prove it changes nothing.
        const REFUSED_QUIET_TURNS: usize = 4;
        let Some((step, attempts)) = self
            .ivars()
            .menu_bar_probe
            .borrow()
            .as_ref()
            .map(|probe| (probe.step, probe.attempts))
        else {
            return;
        };
        let retry = || {
            if let Some(probe) = self.ivars().menu_bar_probe.borrow_mut().as_mut() {
                probe.attempts += 1;
            }
            self.schedule_menu_bar_probe();
        };
        let advance = || {
            if let Some(probe) = self.ivars().menu_bar_probe.borrow_mut().as_mut() {
                probe.step += 1;
                probe.attempts = 0;
            }
            self.schedule_menu_bar_probe();
        };
        match step {
            0 => {
                if self.observed_probe_scene() != Some("ready") {
                    if attempts >= MAX_MAIN_LOOP_TURNS {
                        self.fail_menu_bar_probe_step("initial_scene", "scene_timeout");
                        return;
                    }
                    retry();
                    return;
                }
                eprintln!(
                    "Rinka menu-bar probe step=initial_scene observed_scene=ready active={} pass=true",
                    application_is_active()
                );
                // SAFETY: The structure assertions read only retained menu
                // objects on the main thread.
                let structure = unsafe { probe_menu_bar_structure() };
                if !structure {
                    self.fail_menu_bar_probe_step("structure", "main_menu_mismatch");
                    return;
                }
                // SAFETY: The extract walks retained menu objects on the
                // main thread after forcing native validation.
                unsafe {
                    log_menu_bar_ax("initial");
                    capture_step_windows("menu-bar-baseline");
                }
                // Activate View > Empty through its native target/action
                // pair — the same pair AppKit invokes for a chosen item.
                let activated = unsafe { activate_menu_bar_item("View", "Empty") };
                if !activated {
                    self.fail_menu_bar_probe_step("activate_view_empty", "item_not_activatable");
                    return;
                }
                advance();
            }
            1 => {
                if self.observed_probe_scene() != Some("empty") {
                    if attempts >= MAX_MAIN_LOOP_TURNS {
                        self.fail_menu_bar_probe_step("activate_view_empty", "scene_timeout");
                        return;
                    }
                    retry();
                    return;
                }
                // SAFETY: Checkmark state is read from retained menu items.
                let (empty_state, ready_state) = unsafe {
                    (
                        menu_bar_item_state("View", "Empty"),
                        menu_bar_item_state("View", "Ready"),
                    )
                };
                let passed = empty_state == Some(1) && ready_state == Some(0);
                eprintln!(
                    "Rinka menu-bar probe step=checkmark_reconcile empty_state={empty_state:?} ready_state={ready_state:?} pass={passed}"
                );
                if !passed {
                    if let Some(probe) = self.ivars().menu_bar_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_menu_bar_probe();
                    return;
                }
                // Primary+3 travels the real queue: the monitor defers the
                // menu-owned chord and native key-equivalent dispatch fires
                // View > Error.
                self.post_probe_chord("3", 20, NS_EVENT_MODIFIER_COMMAND);
                advance();
            }
            2 => {
                if self.observed_probe_scene() != Some("error") {
                    if attempts >= MAX_MAIN_LOOP_TURNS {
                        self.fail_menu_bar_probe_step("menu_key_equivalent", "scene_timeout");
                        return;
                    }
                    retry();
                    return;
                }
                // SAFETY: State and validated enabled state are read from
                // retained menu objects on the main thread.
                let (error_state, new_folder_enabled) = unsafe {
                    (
                        menu_bar_item_state("View", "Error"),
                        menu_bar_item_enabled_after_update("File", "New Folder"),
                    )
                };
                let passed = error_state == Some(1) && new_folder_enabled == Some(false);
                eprintln!(
                    "Rinka menu-bar probe step=menu_key_equivalent error_state={error_state:?} new_folder_enabled={new_folder_enabled:?} pass={passed}"
                );
                if !passed {
                    if let Some(probe) = self.ivars().menu_bar_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_menu_bar_probe();
                    return;
                }
                // New Folder is disabled in the error scene; its chord
                // (Primary+Alt+N — Primary+N belongs to New Window) must be
                // refused by native validation.
                self.post_probe_chord("n", 45, NS_EVENT_MODIFIER_COMMAND | NS_EVENT_MODIFIER_OPTION);
                advance();
            }
            3 => {
                if attempts < REFUSED_QUIET_TURNS {
                    retry();
                    return;
                }
                let note = self.probe_file_action_note().unwrap_or_default();
                let passed = !note.contains("New Folder");
                eprintln!(
                    "Rinka menu-bar probe step=disabled_chord_refused note={note:?} pass={passed}"
                );
                if !passed {
                    if let Some(probe) = self.ivars().menu_bar_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_menu_bar_probe();
                    return;
                }
                let activated = unsafe { activate_menu_bar_item("View", "Ready") };
                if !activated {
                    self.fail_menu_bar_probe_step("activate_view_ready", "item_not_activatable");
                    return;
                }
                advance();
            }
            4 => {
                if self.observed_probe_scene() != Some("ready") {
                    if attempts >= MAX_MAIN_LOOP_TURNS {
                        self.fail_menu_bar_probe_step("activate_view_ready", "scene_timeout");
                        return;
                    }
                    retry();
                    return;
                }
                // With the listing live again, the menu-only chord must
                // dispatch New Folder to the focused window's component.
                self.post_probe_chord("n", 45, NS_EVENT_MODIFIER_COMMAND | NS_EVENT_MODIFIER_OPTION);
                advance();
            }
            5 => {
                let note = self.probe_file_action_note().unwrap_or_default();
                if !note.contains("New Folder created in Remote Project") {
                    if attempts >= MAX_MAIN_LOOP_TURNS {
                        self.fail_menu_bar_probe_step(
                            "menu_only_chord",
                            &format!("note={note:?}"),
                        );
                        return;
                    }
                    retry();
                    return;
                }
                eprintln!("Rinka menu-bar probe step=menu_only_chord note={note:?} pass=true");

                // Edit roles against the native search field with no
                // consumer role handling: fill and focus the field, then
                // Select All and Copy purely through the menu bar.
                if !self.prepare_probe_search_field_selection(MENU_BAR_COPY_MARKER) {
                    self.fail_menu_bar_probe_step("edit_roles", "search_field_not_focused");
                    return;
                }
                // SAFETY: The Edit menu items dispatch their nil-target
                // selectors down the responder chain to the field editor;
                // without OS activation the chain is anchored at the primary
                // window's first responder.
                let (mechanism, select_all, copy) = unsafe {
                    let windows = self.ivars().windows.borrow();
                    let window = windows.first();
                    (
                        native_dispatch_mechanism(),
                        dispatch_native_menu_item("Edit", "Select All", window),
                        dispatch_native_menu_item("Edit", "Copy", window),
                    )
                };
                let observed = self.probe_general_pasteboard_text();
                let copy_passed =
                    select_all && copy && observed.as_deref() == Some(MENU_BAR_COPY_MARKER);
                eprintln!(
                    "Rinka menu-bar probe step=edit_roles mechanism={mechanism} select_all={select_all} copy={copy} observed={observed:?} pass={copy_passed}"
                );
                // Cut empties the still-selected field while keeping the
                // marker on the pasteboard, and Paste restores it — both
                // through the same menu items. Undo is expected to be
                // *inert* here: a field editor does not register undo groups
                // (stock Cocoa behavior — the same dispatch against a search
                // field in a vanilla app restores nothing), so the paste
                // must survive the undo dispatch unchanged. A rinka-owned
                // editable surface with working undo belongs to the
                // text-editing tickets.
                let (cut, after_cut, paste, after_paste, undo, after_undo) = unsafe {
                    let windows = self.ivars().windows.borrow();
                    let window = windows.first();
                    let cut = dispatch_native_menu_item("Edit", "Cut", window);
                    let after_cut = self.probe_focused_editor_text();
                    let paste = dispatch_native_menu_item("Edit", "Paste", window);
                    let after_paste = self.probe_focused_editor_text();
                    let undo = dispatch_native_menu_item("Edit", "Undo", window);
                    let after_undo = self.probe_focused_editor_text();
                    (cut, after_cut, paste, after_paste, undo, after_undo)
                };
                let pasteboard_after_cut = self.probe_general_pasteboard_text();
                let cut_passed = cut
                    && after_cut.as_deref() == Some("")
                    && pasteboard_after_cut.as_deref() == Some(MENU_BAR_COPY_MARKER);
                let paste_passed = paste && after_paste.as_deref() == Some(MENU_BAR_COPY_MARKER);
                let undo_inert = after_undo.as_deref() == Some(MENU_BAR_COPY_MARKER);
                eprintln!(
                    "Rinka menu-bar probe step=edit_roles_cut_paste_undo cut={cut_passed} after_cut={after_cut:?} paste={paste_passed} after_paste={after_paste:?} undo_dispatched={undo} undo_inert={undo_inert} after_undo={after_undo:?}"
                );
                let passed = copy_passed && cut_passed && paste_passed && undo_inert;
                self.unfocus_probe_text_input();
                if !passed {
                    if let Some(probe) = self.ivars().menu_bar_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_menu_bar_probe();
                    return;
                }
                advance();
            }
            _ => {
                // About: the synthesized application menu shows the native
                // panel; the panel is a window this delegate never built.
                let registered: Vec<usize> = self
                    .ivars()
                    .windows
                    .borrow()
                    .iter()
                    .map(|window| window.as_ptr() as usize)
                    .collect();
                // SAFETY: All menu and window reads happen on the main
                // thread over retained AppKit objects.
                let about_passed = unsafe { probe_about_panel(&registered) };
                eprintln!("Rinka menu-bar probe step=about_panel pass={about_passed}");
                if let Some(probe) = self.ivars().menu_bar_probe.borrow_mut().as_mut() {
                    probe.passed &= about_passed;
                }
                // SAFETY: The menu pop, extract, and captures read and drive
                // only this process's retained objects.
                unsafe {
                    probe_open_view_menu(self.mtm(), &self.ivars().windows.borrow());
                    log_menu_bar_ax("final");
                    capture_step_windows("menu-bar-final");
                }
                self.finish_menu_bar_probe();
            }
        }
    }

    fn finish_menu_bar_probe(&self) {
        let passed = self
            .ivars()
            .menu_bar_probe
            .borrow()
            .as_ref()
            .is_some_and(|probe| probe.passed);
        eprintln!(
            "Rinka menu-bar probe result={}",
            if passed { "PASS" } else { "FAIL" }
        );
        let finish = std::env::var("RINKA_APPKIT_MENU_BAR_PROBE_FINISH").unwrap_or_default();
        match finish.as_str() {
            // The native quit path: the application menu's Quit item.
            "quit" => {
                // terminate: exits inside the dispatch, so the finish line
                // is printed before the item is performed.
                eprintln!("Rinka menu-bar probe finish=quit dispatching=true");
                // SAFETY: The Quit item dispatches terminate: to the shared
                // application, ending only this probe process.
                let quit = unsafe { perform_application_menu_item_with_prefix("Quit") };
                eprintln!("Rinka menu-bar probe finish=quit not_performed={}", !quit);
                if quit {
                    return;
                }
            }
            // The native close path: File > Close Window closes the last
            // window, and the delegate's last-window-closed policy
            // terminates the application.
            "close" => {
                // SAFETY: performClose: travels the responder chain to the
                // key window of this probe process, or is anchored at the
                // primary window when the desktop denies activation.
                // The last-window-closed policy terminates inside the
                // dispatch, so the finish line is printed before the item
                // is performed.
                eprintln!(
                    "Rinka menu-bar probe finish=close mechanism={} dispatching=true",
                    native_dispatch_mechanism()
                );
                let close = unsafe {
                    let windows = self.ivars().windows.borrow();
                    let window = windows.first();
                    dispatch_native_menu_item("File", "Close Window", window)
                };
                eprintln!("Rinka menu-bar probe finish=close not_performed={}", !close);
                if close {
                    return;
                }
            }
            _ => {}
        }
        // SAFETY: Diagnostic completion terminates only the current test app.
        unsafe {
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let _: () = msg_send![application, terminate: std::ptr::null::<AnyObject>()];
        }
    }
}

/// Returns the installed main menu.
unsafe fn probe_main_menu() -> Option<Id> {
    // SAFETY: mainMenu is a main-thread NSApplication read.
    unsafe {
        let application: *mut AnyObject =
            msg_send![objc2::class!(NSApplication), sharedApplication];
        let menu: *mut AnyObject = msg_send![application, mainMenu];
        NonNull::new(menu).map(|menu| Id::from_borrowed(menu.as_ptr()))
    }
}

/// Returns the submenu of the top-level menu with this title.
unsafe fn menu_bar_submenu(title: &str) -> Option<Id> {
    // SAFETY: All receivers are retained menu objects on the main thread.
    unsafe {
        let main = probe_main_menu()?;
        let item = menu_item_titled(main.as_object(), title)?;
        let submenu: *mut AnyObject = msg_send![item.as_object(), submenu];
        NonNull::new(submenu).map(|submenu| Id::from_borrowed(submenu.as_ptr()))
    }
}

/// Asserts the installed menu bar matches the explorer's declaration.
unsafe fn probe_menu_bar_structure() -> bool {
    // SAFETY: All receivers are retained menu objects on the main thread.
    unsafe {
        let Some(main) = probe_main_menu() else {
            eprintln!("Rinka menu-bar probe step=structure error=no_main_menu pass=false");
            return false;
        };
        let count: isize = msg_send![main.as_object(), numberOfItems];
        let mut titles = Vec::new();
        for index in 0..count {
            let item: *mut AnyObject = msg_send![main.as_object(), itemAtIndex: index];
            let title: *mut AnyObject = msg_send![item, title];
            titles.push(rust_string(title));
        }
        let titles_pass = count == 6
            && titles[1..] == ["File", "Edit", "View", "Window", "Help"].map(str::to_owned);

        let application: *mut AnyObject =
            msg_send![objc2::class!(NSApplication), sharedApplication];
        let windows_menu: *mut AnyObject = msg_send![application, windowsMenu];
        let help_menu: *mut AnyObject = msg_send![application, helpMenu];
        let windows_pass = menu_bar_submenu("Window")
            .is_some_and(|submenu| submenu.as_ptr() == windows_menu);
        let help_pass =
            menu_bar_submenu("Help").is_some_and(|submenu| submenu.as_ptr() == help_menu);

        // Edit > Copy: a nil-target copy: selector carrying the canonical
        // key equivalent — the pass-through contract for standard roles.
        let copy_pass = menu_bar_submenu("Edit").is_some_and(|edit| {
            menu_item_titled(edit.as_object(), "Copy").is_some_and(|item| {
                let action: Option<objc2::runtime::Sel> = msg_send![item.as_object(), action];
                let target: *mut AnyObject = msg_send![item.as_object(), target];
                let key: *mut AnyObject = msg_send![item.as_object(), keyEquivalent];
                let mask: usize = msg_send![item.as_object(), keyEquivalentModifierMask];
                action == Some(sel!(copy:))
                    && target.is_null()
                    && rust_string(key) == "c"
                    && mask == NS_EVENT_MODIFIER_COMMAND
            })
        });
        let new_folder_pass = menu_bar_submenu("File").is_some_and(|file| {
            menu_item_titled(file.as_object(), "New Folder").is_some_and(|item| {
                let key: *mut AnyObject = msg_send![item.as_object(), keyEquivalent];
                let mask: usize = msg_send![item.as_object(), keyEquivalentModifierMask];
                rust_string(key) == "n"
                    && mask == NS_EVENT_MODIFIER_COMMAND | NS_EVENT_MODIFIER_OPTION
            })
        });
        let new_window_pass = menu_bar_submenu("File").is_some_and(|file| {
            menu_item_titled(file.as_object(), "New Window").is_some_and(|item| {
                let key: *mut AnyObject = msg_send![item.as_object(), keyEquivalent];
                let mask: usize = msg_send![item.as_object(), keyEquivalentModifierMask];
                rust_string(key) == "n" && mask == NS_EVENT_MODIFIER_COMMAND
            })
        });
        let checkmark_pass = menu_bar_item_state("View", "Ready") == Some(1)
            && menu_bar_item_state("View", "Empty") == Some(0);

        let passed = titles_pass
            && windows_pass
            && help_pass
            && copy_pass
            && new_folder_pass
            && new_window_pass
            && checkmark_pass;
        eprintln!(
            "Rinka menu-bar probe step=structure items={count} titles={titles:?} titles_pass={titles_pass} windows_menu={windows_pass} help_menu={help_pass} copy_role={copy_pass} new_folder_chord={new_folder_pass} new_window_chord={new_window_pass} checkmarks={checkmark_pass} pass={passed}"
        );
        passed
    }
}

/// Reads one item's checkmark state along a menu-bar path.
unsafe fn menu_bar_item_state(menu_title: &str, item_title: &str) -> Option<isize> {
    // SAFETY: All receivers are retained menu objects on the main thread.
    unsafe {
        let submenu = menu_bar_submenu(menu_title)?;
        let item = menu_item_titled(submenu.as_object(), item_title)?;
        let state: isize = msg_send![item.as_object(), state];
        Some(state)
    }
}

/// Reads one item's enabled state after forcing native menu validation.
unsafe fn menu_bar_item_enabled_after_update(
    menu_title: &str,
    item_title: &str,
) -> Option<bool> {
    // SAFETY: update runs the same validation AppKit performs before
    // displaying the menu; all receivers are retained menu objects.
    unsafe {
        let submenu = menu_bar_submenu(menu_title)?;
        let _: () = msg_send![submenu.as_object(), update];
        let item = menu_item_titled(submenu.as_object(), item_title)?;
        let enabled: bool = msg_send![item.as_object(), isEnabled];
        Some(enabled)
    }
}

/// Activates an app-defined item through its native target/action pair.
unsafe fn activate_menu_bar_item(menu_title: &str, item_title: &str) -> bool {
    // SAFETY: The target/action pair is the one AppKit invokes for a chosen
    // menu item; all receivers are retained menu objects.
    unsafe {
        let Some(submenu) = menu_bar_submenu(menu_title) else {
            return false;
        };
        let Some(item) = menu_item_titled(submenu.as_object(), item_title) else {
            return false;
        };
        let target: *mut AnyObject = msg_send![item.as_object(), target];
        let Some(target) = NonNull::new(target) else {
            return false;
        };
        let _: () = msg_send![target.as_ref(), performMenuBarAction: item.as_object()];
        true
    }
}

/// Dispatches a native (nil-target) item exactly as the menu would, through
/// `performActionForItemAtIndex:`, which validates before sending.
unsafe fn perform_native_menu_item(menu_title: &str, item_title: &str) -> bool {
    // SAFETY: All receivers are retained menu objects on the main thread.
    unsafe {
        let Some(submenu) = menu_bar_submenu(menu_title) else {
            return false;
        };
        let Some(item) = menu_item_titled(submenu.as_object(), item_title) else {
            return false;
        };
        let index: isize = msg_send![submenu.as_object(), indexOfItem: item.as_object()];
        if index < 0 {
            return false;
        }
        let _: () = msg_send![submenu.as_object(), performActionForItemAtIndex: index];
        true
    }
}

impl ApplicationDelegate {
    /// Reads the focused field editor's text, if native text has focus.
    fn probe_focused_editor_text(&self) -> Option<String> {
        let editor = self.probe_focused_field_editor()?;
        // SAFETY: The field editor's string is copied on the main thread.
        unsafe {
            let value: *mut AnyObject = msg_send![editor.as_object(), string];
            Some(rust_string(value))
        }
    }
}

/// Returns whether the shared application is active.
fn application_is_active() -> bool {
    // SAFETY: isActive is a main-thread NSApplication read.
    unsafe {
        let application: *mut AnyObject =
            msg_send![objc2::class!(NSApplication), sharedApplication];
        msg_send![application, isActive]
    }
}

/// Names the responder-chain mechanism the current activation state allows.
fn native_dispatch_mechanism() -> &'static str {
    if application_is_active() {
        "native-chain"
    } else {
        "anchored-chain"
    }
}

/// Dispatches a nil-target item through the responder chain: natively via
/// `performActionForItemAtIndex:` while the application is active, otherwise
/// anchored at the primary window's first responder with `tryToPerform:` —
/// the same chain walk minus the key-window lookup an inactive application
/// cannot answer.
unsafe fn dispatch_native_menu_item(
    menu_title: &str,
    item_title: &str,
    window: Option<&Id>,
) -> bool {
    // SAFETY: All receivers are retained AppKit objects on the main thread.
    unsafe {
        if application_is_active() {
            return perform_native_menu_item(menu_title, item_title);
        }
        let Some(submenu) = menu_bar_submenu(menu_title) else {
            return false;
        };
        let Some(item) = menu_item_titled(submenu.as_object(), item_title) else {
            return false;
        };
        let target: *mut AnyObject = msg_send![item.as_object(), target];
        let action: Option<objc2::runtime::Sel> = msg_send![item.as_object(), action];
        let (Some(action), true) = (action, target.is_null()) else {
            return false;
        };
        let Some(window) = window else {
            return false;
        };
        let responder: *mut AnyObject = msg_send![window.as_object(), firstResponder];
        let responder = if responder.is_null() {
            window.as_ptr()
        } else {
            responder
        };
        msg_send![responder, tryToPerform: action, with: item.as_object()]
    }
}

/// Dispatches the application-menu item whose title starts with `prefix`
/// (About/Hide/Quit carry the application name in their titles).
unsafe fn perform_application_menu_item_with_prefix(prefix: &str) -> bool {
    // SAFETY: All receivers are retained menu objects on the main thread.
    unsafe {
        let Some(main) = probe_main_menu() else {
            return false;
        };
        let app_item: *mut AnyObject = msg_send![main.as_object(), itemAtIndex: 0_isize];
        let Some(app_item) = NonNull::new(app_item) else {
            return false;
        };
        let submenu: *mut AnyObject = msg_send![app_item.as_ref(), submenu];
        let Some(submenu) = NonNull::new(submenu) else {
            return false;
        };
        let count: isize = msg_send![submenu.as_ref(), numberOfItems];
        for index in 0..count {
            let item: *mut AnyObject = msg_send![submenu.as_ref(), itemAtIndex: index];
            let title: *mut AnyObject = msg_send![item, title];
            if rust_string(title).starts_with(prefix) {
                let _: () = msg_send![submenu.as_ref(), performActionForItemAtIndex: index];
                return true;
            }
        }
        false
    }
}

/// Opens the native About panel from the application menu, asserts a window
/// this delegate never built appeared, and closes it again.
unsafe fn probe_about_panel(registered_windows: &[usize]) -> bool {
    // SAFETY: All receivers are retained AppKit objects on the main thread;
    // the panel belongs to this probe process.
    unsafe {
        if !perform_application_menu_item_with_prefix("About") {
            return false;
        }
        let application: *mut AnyObject =
            msg_send![objc2::class!(NSApplication), sharedApplication];
        let windows: *mut AnyObject = msg_send![application, windows];
        let count: usize = msg_send![windows, count];
        for index in 0..count {
            let window: *mut AnyObject = msg_send![windows, objectAtIndex: index];
            if registered_windows.contains(&(window as usize)) {
                continue;
            }
            let visible: bool = msg_send![window, isVisible];
            if visible {
                let _: () = msg_send![window, close];
                return true;
            }
        }
        false
    }
}

/// Pops the View menu as a real tracking menu (photographed and closed by
/// the shared tracking observer) so the capture shows the live items.
unsafe fn probe_open_view_menu(mtm: MainThreadMarker, windows: &[Id]) {
    // SAFETY: popUpMenuPositioningItem runs a native tracking session on the
    // main thread; the observer's timer cancels it without user input.
    unsafe {
        let Some(view_menu) = menu_bar_submenu("View") else {
            eprintln!("Rinka menu-bar probe step=open_view_menu opened=false pass=false");
            return;
        };
        let Some(window) = windows.first() else {
            return;
        };
        let frame: Rect = msg_send![window.as_object(), frame];
        let location = Point {
            x: frame.origin.x + 60.0,
            y: frame.origin.y + frame.size.height - 20.0,
        };
        let (performed, opened) =
            drive_menu_open(mtm, view_menu.as_object(), "menu-bar-view", || {
                let shown: bool = msg_send![view_menu.as_object(),
                    popUpMenuPositioningItem: std::ptr::null::<AnyObject>(),
                    atLocation: location,
                    inView: std::ptr::null::<AnyObject>()
                ];
                shown
            });
        eprintln!(
            "Rinka menu-bar probe step=open_view_menu performed={performed} opened={opened} pass={opened}"
        );
    }
}

/// Logs the accessibility extract of the complete installed menu tree:
/// every item's accessible title, enabled state, checkmark, and key
/// equivalent, after forcing native validation per menu.
unsafe fn log_menu_bar_ax(label: &str) {
    // SAFETY: All receivers are retained menu objects on the main thread.
    unsafe {
        let Some(main) = probe_main_menu() else {
            eprintln!("Rinka menu-bar ax label={label} error=no_main_menu");
            return;
        };
        eprintln!("Rinka menu-bar ax label={label} begin");
        log_menu_ax_items(main.as_object(), 0);
        eprintln!("Rinka menu-bar ax label={label} end");
    }
}

/// # Safety
///
/// `menu` must be a live NSMenu read on the main thread.
unsafe fn log_menu_ax_items(menu: &AnyObject, depth: usize) {
    // SAFETY: update runs native validation so the logged enabled state is
    // the one assistive clients observe; item reads are public API.
    unsafe {
        let _: () = msg_send![menu, update];
        let count: isize = msg_send![menu, numberOfItems];
        for index in 0..count {
            let item: *mut AnyObject = msg_send![menu, itemAtIndex: index];
            let Some(item) = NonNull::new(item) else {
                continue;
            };
            let separator: bool = msg_send![item.as_ref(), isSeparatorItem];
            if separator {
                eprintln!("Rinka menu-bar ax {}separator", "  ".repeat(depth));
                continue;
            }
            // NSMenuItem's accessibility title is its title; read the AX
            // getter where the runtime provides it and fall back to title.
            let responds: bool = msg_send![
                item.as_ref(),
                respondsToSelector: sel!(accessibilityTitle)
            ];
            let title: *mut AnyObject = if responds {
                msg_send![item.as_ref(), accessibilityTitle]
            } else {
                msg_send![item.as_ref(), title]
            };
            let mut title = rust_string(title);
            if title.is_empty() {
                let fallback: *mut AnyObject = msg_send![item.as_ref(), title];
                title = rust_string(fallback);
            }
            let enabled: bool = msg_send![item.as_ref(), isEnabled];
            let state: isize = msg_send![item.as_ref(), state];
            let key: *mut AnyObject = msg_send![item.as_ref(), keyEquivalent];
            let key = rust_string(key);
            eprintln!(
                "Rinka menu-bar ax {}item title={title:?} enabled={enabled} state={state} key={key:?}",
                "  ".repeat(depth)
            );
            let submenu: *mut AnyObject = msg_send![item.as_ref(), submenu];
            if let Some(submenu) = NonNull::new(submenu) {
                log_menu_ax_items(submenu.as_ref(), depth + 1);
            }
        }
    }
}
