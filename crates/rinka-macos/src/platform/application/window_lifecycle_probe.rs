// Live verification of the runtime window lifecycle on the AppKit host.
//
// The probe drives the explorer's real multi-window flows entirely
// in-process: File > New Window through a real Primary+N key event posted to
// its own queue, live retitling through the scene chords, a programmatic
// close through the window-scoped Primary+Alt+W accelerator, and the
// close-interception protocol through performClose: — the native
// close-button path — answered through the real confirmation sheet's
// buttons. No global input is injected and only this probe's own windows are
// touched.

const WINDOW_LIFECYCLE_PROBE_MAX_TURNS: usize = 200;
/// Turns the vetoed window is watched to prove the veto actually held.
const WINDOW_LIFECYCLE_VETO_QUIET_TURNS: usize = 4;

impl ApplicationDelegate {
    fn begin_window_lifecycle_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_WINDOW_LIFECYCLE_PROBE").is_none()
            || self.ivars().window_lifecycle_probe.borrow().is_some()
        {
            return;
        }
        for other in [
            "RINKA_APPKIT_SCENE_PROBE",
            "RINKA_APPKIT_TRANSITION_PROBE",
            "RINKA_APPKIT_ACCELERATOR_PROBE",
            "RINKA_APPKIT_CLIPBOARD_PROBE",
            "RINKA_APPKIT_DIALOG_PROBE",
            "RINKA_APPKIT_MENU_BAR_PROBE",
        ] {
            if std::env::var_os(other).is_some() {
                panic!("the window-lifecycle probe must run in its own process");
            }
        }
        *self.ivars().window_lifecycle_probe.borrow_mut() = Some(WindowLifecycleProbe {
            step: 0,
            attempts: 0,
            passed: true,
            first_secondary: None,
        });
        self.schedule_window_lifecycle_probe();
    }

    fn schedule_window_lifecycle_probe(&self) {
        // SAFETY: The next main-loop turn observes window-set changes after
        // posted events dispatched and reconciliation completed.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runWindowLifecycleProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.05_f64
            ];
        }
    }

    fn fail_window_lifecycle_probe(&self, step: &'static str, detail: &str) {
        eprintln!("Rinka window-lifecycle probe step={step} {detail} pass=false");
        if let Some(probe) = self.ivars().window_lifecycle_probe.borrow_mut().as_mut() {
            probe.passed = false;
        }
        self.finish_window_lifecycle_probe();
    }

    /// Resolves the key window's declared identity text, if any.
    fn probe_key_window_id(&self) -> Option<String> {
        // SAFETY: keyWindow is a main-thread NSApplication read; the
        // identity registry pairs retained pointers with declared ids.
        unsafe {
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let key: *mut AnyObject = msg_send![application, keyWindow];
            let key = NonNull::new(key)?;
            self.window_declared_id(key.as_ref())
                .map(|id| id.as_str().to_owned())
        }
    }

    /// Finds the open runtime-opened explorer window, if one exists.
    fn probe_secondary_window(&self) -> Option<(WindowId, Id)> {
        let id = self
            .ivars()
            .window_identities
            .borrow()
            .iter()
            .find_map(|(_, id)| id.as_str().starts_with("explorer-secondary-").then(|| id.clone()))?;
        let window = self.native_window_for(&id)?;
        Some((id, window))
    }

    /// Reads one retained window's native title.
    fn probe_window_title(&self, window: &AnyObject) -> String {
        // SAFETY: title is a main-thread read of the retained window.
        unsafe {
            let title: *mut AnyObject = msg_send![window, title];
            rust_string(title)
        }
    }

    /// Reads the main explorer window's mounted window-note label.
    fn probe_window_note(&self) -> Option<String> {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().and_then(|runtime| {
            runtime.with_renderer(|renderer| {
                renderer
                    .mounted()
                    .and_then(|root| mounted_label_text(root, "window-note"))
            })
        })
    }

    /// Reads the secondary window's attached sheet, if one is presented.
    fn probe_secondary_sheet(&self) -> Option<NonNull<AnyObject>> {
        let (_, window) = self.probe_secondary_window()?;
        // SAFETY: attachedSheet is a main-thread read of the retained
        // window; a null sheet is represented as None.
        unsafe {
            let sheet: *mut AnyObject = msg_send![window.as_object(), attachedSheet];
            NonNull::new(sheet)
        }
    }

    /// Presses the button titled `title` inside the secondary's sheet.
    fn press_secondary_sheet_button(&self, title: &str) -> bool {
        let Some(sheet) = self.probe_secondary_sheet() else {
            return false;
        };
        // SAFETY: The sheet and its content view are live main-thread
        // objects; performClick: drives the ordinary target/action dispatch.
        unsafe {
            let content: *mut AnyObject = msg_send![sheet.as_ref(), contentView];
            let Some(content) = NonNull::new(content) else {
                return false;
            };
            let Some(button) = find_button_titled(content.as_ref(), title) else {
                return false;
            };
            let _: () = msg_send![button.as_ref(), performClick: std::ptr::null::<AnyObject>()];
        }
        true
    }

    /// Sends the native close-button path to the secondary window.
    fn perform_close_secondary(&self) -> bool {
        let Some((_, window)) = self.probe_secondary_window() else {
            return false;
        };
        // SAFETY: performClose: is the user-gesture close path — it travels
        // windowShouldClose:, which is exactly the interception under test —
        // sent to this probe's own retained window on the main thread.
        unsafe {
            let _: () = msg_send![window.as_object(), performClose: std::ptr::null::<AnyObject>()];
        }
        true
    }

    fn window_lifecycle_retry(&self, step: &'static str) {
        let attempts = {
            let mut probe = self.ivars().window_lifecycle_probe.borrow_mut();
            let Some(probe) = probe.as_mut() else {
                return;
            };
            probe.attempts += 1;
            probe.attempts
        };
        if attempts < WINDOW_LIFECYCLE_PROBE_MAX_TURNS {
            self.schedule_window_lifecycle_probe();
        } else {
            // The mounted action note distinguishes "the message never
            // dispatched" from "the service call failed silently".
            let note = {
                let renderers = self.ivars().renderers.borrow();
                renderers.first().and_then(|runtime| {
                    runtime.with_renderer(|renderer| {
                        renderer
                            .mounted()
                            .and_then(|root| mounted_label_text(root, "file-action-note"))
                    })
                })
            };
            self.fail_window_lifecycle_probe(
                step,
                &format!("settlement_timeout file_action_note={note:?}"),
            );
        }
    }

    fn advance_window_lifecycle_probe_step(&self) {
        if let Some(probe) = self.ivars().window_lifecycle_probe.borrow_mut().as_mut() {
            probe.step += 1;
            probe.attempts = 0;
        }
        self.schedule_window_lifecycle_probe();
    }

    #[allow(clippy::too_many_lines)]
    fn advance_window_lifecycle_probe(&self) {
        let Some((step, attempts)) = self
            .ivars()
            .window_lifecycle_probe
            .borrow()
            .as_ref()
            .map(|probe| (probe.step, probe.attempts))
        else {
            return;
        };
        let window_count = self.ivars().windows.borrow().len();
        match step {
            // Establish activation and the ready scene, then open a second
            // window through the real File > New Window key equivalent.
            0 => {
                if !self.probe_window_is_key() {
                    if attempts >= WINDOW_LIFECYCLE_PROBE_MAX_TURNS {
                        self.fail_window_lifecycle_probe("initial_scene", "activation_timeout");
                        return;
                    }
                    self.window_lifecycle_retry("initial_scene");
                    return;
                }
                if self.observed_probe_scene() != Some("ready") {
                    self.fail_window_lifecycle_probe("initial_scene", "expected_scene=ready");
                    return;
                }
                eprintln!(
                    "Rinka window-lifecycle probe step=initial_scene observed_scene=ready windows={window_count} pass=true"
                );
                self.post_probe_chord("n", 45, NS_EVENT_MODIFIER_COMMAND);
                self.advance_window_lifecycle_probe_step();
            }
            // The second window opened, took key focus, and titled itself
            // from its component state.
            1 => {
                if window_count != 2 {
                    self.window_lifecycle_retry("open_second_window");
                    return;
                }
                let Some((id, window)) = self.probe_secondary_window() else {
                    self.window_lifecycle_retry("open_second_window");
                    return;
                };
                let key = self.probe_key_window_id();
                let title = self.probe_window_title(window.as_object());
                let note = self.probe_window_note().unwrap_or_default();
                let key_is_secondary = key.as_deref() == Some(id.as_str());
                let title_pass = title == "Rinka Explorer — Ready";
                let resigned_pass = note == "window resigned: explorer-main";
                if !(key_is_secondary && title_pass && resigned_pass) {
                    if attempts < WINDOW_LIFECYCLE_PROBE_MAX_TURNS {
                        self.window_lifecycle_retry("open_second_window");
                        return;
                    }
                    self.fail_window_lifecycle_probe(
                        "open_second_window",
                        &format!("key={key:?} title={title:?} note={note:?}"),
                    );
                    return;
                }
                eprintln!(
                    "Rinka window-lifecycle probe step=open_second_window id={} key_is_secondary={key_is_secondary} title={title:?} main_note={note:?} pass=true",
                    id.as_str()
                );
                if let Some(probe) = self.ivars().window_lifecycle_probe.borrow_mut().as_mut() {
                    probe.first_secondary = Some(id);
                }
                // The menu-owned Primary+2 routes to the key window: the
                // secondary's own component switches its scene.
                self.post_probe_chord("2", 19, NS_EVENT_MODIFIER_COMMAND);
                self.advance_window_lifecycle_probe_step();
            }
            // The declared title reconciled from the changed scene state
            // without rebuilding the window.
            2 => {
                let Some((_, window)) = self.probe_secondary_window() else {
                    self.fail_window_lifecycle_probe("live_title", "secondary_missing");
                    return;
                };
                let title = self.probe_window_title(window.as_object());
                if title != "Rinka Explorer — Empty" {
                    self.window_lifecycle_retry("live_title");
                    return;
                }
                eprintln!(
                    "Rinka window-lifecycle probe step=live_title title={title:?} windows={window_count} pass=true"
                );
                // The table-owned Primary+4 switches the secondary into the
                // editor scene — the dirty-ish state that intercepts closes.
                self.post_probe_chord("4", 21, NS_EVENT_MODIFIER_COMMAND);
                self.advance_window_lifecycle_probe_step();
            }
            3 => {
                let Some((_, window)) = self.probe_secondary_window() else {
                    self.fail_window_lifecycle_probe("editor_scene", "secondary_missing");
                    return;
                };
                let title = self.probe_window_title(window.as_object());
                if title != "Rinka Explorer — Editor" {
                    self.window_lifecycle_retry("editor_scene");
                    return;
                }
                eprintln!(
                    "Rinka window-lifecycle probe step=editor_scene title={title:?} pass=true"
                );
                // Programmatic close from a component message: Primary+Alt+W
                // dispatches CloseThisWindow in the key window, which closes
                // unconditionally — no sheet, even in the editor scene.
                self.post_probe_chord(
                    "w",
                    13,
                    NS_EVENT_MODIFIER_COMMAND | NS_EVENT_MODIFIER_OPTION,
                );
                self.advance_window_lifecycle_probe_step();
            }
            4 => {
                if window_count != 1 {
                    self.window_lifecycle_retry("programmatic_close");
                    return;
                }
                let key = self.probe_key_window_id();
                let note = self.probe_window_note().unwrap_or_default();
                let key_is_main = key.as_deref() == Some("explorer-main");
                let focused_pass = note == "window focused: explorer-main";
                if !(key_is_main && focused_pass) {
                    if attempts < WINDOW_LIFECYCLE_PROBE_MAX_TURNS {
                        self.window_lifecycle_retry("programmatic_close");
                        return;
                    }
                    self.fail_window_lifecycle_probe(
                        "programmatic_close",
                        &format!("key={key:?} note={note:?}"),
                    );
                    return;
                }
                eprintln!(
                    "Rinka window-lifecycle probe step=programmatic_close windows={window_count} key_is_main={key_is_main} main_note={note:?} pass=true"
                );
                // Reopen for the interception flow.
                self.post_probe_chord("n", 45, NS_EVENT_MODIFIER_COMMAND);
                self.advance_window_lifecycle_probe_step();
            }
            // The reopened window carries a fresh identity: identity is
            // stable per window, never recycled across opens.
            5 => {
                if window_count != 2 {
                    self.window_lifecycle_retry("reopen_second_window");
                    return;
                }
                let Some((id, _)) = self.probe_secondary_window() else {
                    self.window_lifecycle_retry("reopen_second_window");
                    return;
                };
                let first = self
                    .ivars()
                    .window_lifecycle_probe
                    .borrow()
                    .as_ref()
                    .and_then(|probe| probe.first_secondary.clone());
                let fresh_identity = first.as_ref() != Some(&id);
                if !fresh_identity {
                    self.fail_window_lifecycle_probe(
                        "reopen_second_window",
                        &format!("identity_reused={}", id.as_str()),
                    );
                    return;
                }
                eprintln!(
                    "Rinka window-lifecycle probe step=reopen_second_window id={} first={:?} fresh_identity={fresh_identity} pass=true",
                    id.as_str(),
                    first.as_ref().map(WindowId::as_str)
                );
                self.post_probe_chord("4", 21, NS_EVENT_MODIFIER_COMMAND);
                self.advance_window_lifecycle_probe_step();
            }
            6 => {
                let Some((_, window)) = self.probe_secondary_window() else {
                    self.fail_window_lifecycle_probe("editor_scene_again", "secondary_missing");
                    return;
                };
                let title = self.probe_window_title(window.as_object());
                if title != "Rinka Explorer — Editor" {
                    self.window_lifecycle_retry("editor_scene_again");
                    return;
                }
                eprintln!(
                    "Rinka window-lifecycle probe step=editor_scene_again title={title:?} pass=true"
                );
                // The native close-button path: windowShouldClose: defers it
                // behind a pending-close token and the component presents
                // the confirmation sheet.
                if !self.perform_close_secondary() {
                    self.fail_window_lifecycle_probe("perform_close", "secondary_missing");
                    return;
                }
                self.advance_window_lifecycle_probe_step();
            }
            7 => {
                let Some(sheet) = self.probe_secondary_sheet() else {
                    self.window_lifecycle_retry("close_deferred_sheet");
                    return;
                };
                let still_open = window_count == 2;
                eprintln!(
                    "Rinka window-lifecycle probe step=close_deferred_sheet sheet=true windows={window_count} still_open={still_open} pass={still_open}"
                );
                if !still_open {
                    if let Some(probe) =
                        self.ivars().window_lifecycle_probe.borrow_mut().as_mut()
                    {
                        probe.passed = false;
                    }
                    self.finish_window_lifecycle_probe();
                    return;
                }
                // SAFETY: The attached sheet's content renders on main.
                unsafe {
                    self.capture_dialog_sheet(sheet.as_ref(), "window-lifecycle-confirm-sheet.png");
                }
                if !self.press_secondary_sheet_button("Cancel") {
                    self.fail_window_lifecycle_probe("veto", "cancel_button_missing");
                    return;
                }
                self.advance_window_lifecycle_probe_step();
            }
            // The veto held: the sheet is gone and the window stayed open
            // across several quiet turns.
            8 => {
                if self.probe_secondary_sheet().is_some() {
                    self.window_lifecycle_retry("veto_holds");
                    return;
                }
                if attempts < WINDOW_LIFECYCLE_VETO_QUIET_TURNS {
                    self.window_lifecycle_retry("veto_holds");
                    return;
                }
                let pending = self.ivars().pending_closes.borrow().len();
                let passed = window_count == 2 && pending == 0;
                eprintln!(
                    "Rinka window-lifecycle probe step=veto_holds windows={window_count} pending_tokens={pending} pass={passed}"
                );
                if !passed {
                    if let Some(probe) =
                        self.ivars().window_lifecycle_probe.borrow_mut().as_mut()
                    {
                        probe.passed = false;
                    }
                    self.finish_window_lifecycle_probe();
                    return;
                }
                if !self.perform_close_secondary() {
                    self.fail_window_lifecycle_probe("second_close", "secondary_missing");
                    return;
                }
                self.advance_window_lifecycle_probe_step();
            }
            9 => {
                if self.probe_secondary_sheet().is_none() {
                    self.window_lifecycle_retry("confirm_sheet");
                    return;
                }
                eprintln!("Rinka window-lifecycle probe step=confirm_sheet sheet=true pass=true");
                if !self.press_secondary_sheet_button("Close") {
                    self.fail_window_lifecycle_probe("confirm", "close_button_missing");
                    return;
                }
                self.advance_window_lifecycle_probe_step();
            }
            // Only the explicit confirmation closed the window.
            _ => {
                if window_count != 1 {
                    self.window_lifecycle_retry("confirmed_close");
                    return;
                }
                let key = self.probe_key_window_id();
                let pending = self.ivars().pending_closes.borrow().len();
                let key_is_main = key.as_deref() == Some("explorer-main");
                let passed = key_is_main && pending == 0;
                eprintln!(
                    "Rinka window-lifecycle probe step=confirmed_close windows={window_count} key_is_main={key_is_main} pending_tokens={pending} pass={passed}"
                );
                if let Some(probe) = self.ivars().window_lifecycle_probe.borrow_mut().as_mut() {
                    probe.passed &= passed;
                }
                self.capture_windows_to_directory("window-lifecycle-");
                self.finish_window_lifecycle_probe();
            }
        }
    }

    fn finish_window_lifecycle_probe(&self) {
        let passed = self
            .ivars()
            .window_lifecycle_probe
            .borrow()
            .as_ref()
            .is_some_and(|probe| probe.passed);
        eprintln!(
            "Rinka window-lifecycle probe result={}",
            if passed { "PASS" } else { "FAIL" }
        );
        if std::env::var_os("RINKA_APPKIT_WINDOW_LIFECYCLE_PROBE_HOLD").is_none() {
            // SAFETY: Diagnostic completion terminates only the current test app.
            unsafe {
                let application: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let _: () = msg_send![application, terminate: std::ptr::null::<AnyObject>()];
            }
        }
    }
}
