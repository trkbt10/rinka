// Live verification of window-modal dialogs on the AppKit host.
//
// The probe drives the explorer's real transfer and delete flows entirely
// in-process: mounted buttons are pressed through `performClick:`, panel and
// alert facts are read from the attached sheet through public AppKit API,
// and no global pointer or keyboard event ever leaves this process.

const DIALOG_PROBE_MAX_TURNS: usize = 200;

/// Compacts a path to its final two components, mirroring the explorer's
/// narrow-pane display so mounted label text can be asserted exactly.
fn dialog_probe_compact_path(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    match path.parent().and_then(std::path::Path::file_name) {
        Some(parent) => format!("{}/{name}", parent.to_string_lossy()),
        None => name,
    }
}

/// Compares filesystem paths ignoring the `/private` automount alias, which
/// AppKit panels may add to temporary directories.
fn dialog_probe_paths_match(observed: &str, expected: &str) -> bool {
    observed.trim_start_matches("/private") == expected.trim_start_matches("/private")
}

fn dialog_probe_panel_directory() -> Option<std::path::PathBuf> {
    std::env::var_os("RINKA_EXPLORER_PANEL_DIR").map(std::path::PathBuf::from)
}

fn mounted_node_for_key<'a>(
    node: &'a MountedNode<AppKitHandle>,
    key: &str,
) -> Option<&'a MountedNode<AppKitHandle>> {
    if node
        .element()
        .key()
        .is_some_and(|candidate| candidate.as_str() == key)
    {
        return Some(node);
    }
    node.children()
        .iter()
        .find_map(|child| mounted_node_for_key(child, key))
}

/// Finds an NSButton titled `title` inside one view hierarchy.
///
/// # Safety
///
/// `view` must be a live NSView used on AppKit's main thread.
unsafe fn find_button_titled(view: &AnyObject, title: &str) -> Option<NonNull<AnyObject>> {
    // SAFETY: The traversal reads retained subviews and public NSButton
    // properties on the main thread.
    unsafe {
        let is_button: bool = msg_send![view, isKindOfClass: objc2::class!(NSButton)];
        if is_button {
            let native_title: *mut AnyObject = msg_send![view, title];
            if rust_string(native_title) == title {
                return Some(NonNull::from(view));
            }
        }
        let subviews: *mut AnyObject = msg_send![view, subviews];
        let subviews = NonNull::new(subviews)?;
        let count: usize = msg_send![subviews.as_ref(), count];
        (0..count).find_map(|index| {
            let child: *mut AnyObject = msg_send![subviews.as_ref(), objectAtIndex: index];
            NonNull::new(child).and_then(|child| find_button_titled(child.as_ref(), title))
        })
    }
}

impl ApplicationDelegate {
    fn begin_dialog_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_DIALOG_PROBE").is_none()
            || self.ivars().dialog_probe.borrow().is_some()
        {
            return;
        }
        if std::env::var_os("RINKA_APPKIT_SCENE_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_ACCELERATOR_PROBE").is_some()
        {
            panic!("the dialog probe must run in its own process");
        }
        *self.ivars().dialog_probe.borrow_mut() = Some(DialogProbe {
            step: 0,
            attempts: 0,
            passed: true,
        });
        self.schedule_dialog_probe();
    }

    fn schedule_dialog_probe(&self) {
        // SAFETY: The next main-loop turn observes sheet attachment and
        // reconciliation results after AppKit finishes the queued work.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runDialogProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.05_f64
            ];
        }
    }

    fn fail_dialog_probe(&self, step: &'static str, detail: &str) {
        eprintln!("Rinka dialog probe step={step} {detail} pass=false");
        if let Some(probe) = self.ivars().dialog_probe.borrow_mut().as_mut() {
            probe.passed = false;
        }
        self.finish_dialog_probe();
    }

    /// Retries the current step on the next turn; `false` means the bounded
    /// turn budget is exhausted and the probe has failed.
    fn dialog_probe_retry(&self, step: &'static str) -> bool {
        let attempts = {
            let mut probe = self.ivars().dialog_probe.borrow_mut();
            let Some(probe) = probe.as_mut() else {
                return false;
            };
            probe.attempts += 1;
            probe.attempts
        };
        if attempts < DIALOG_PROBE_MAX_TURNS {
            self.schedule_dialog_probe();
            return true;
        }
        self.fail_dialog_probe(step, "settlement_timeout");
        false
    }

    fn advance_dialog_probe_step(&self) {
        if let Some(probe) = self.ivars().dialog_probe.borrow_mut().as_mut() {
            probe.step += 1;
            probe.attempts = 0;
        }
        self.schedule_dialog_probe();
    }

    /// Presses the mounted element named by `key` through the AppKit action
    /// pipeline. The mounted handle is cloned before the click so no runtime
    /// borrow is held while the resulting update reconciles.
    fn dialog_probe_click_mounted(&self, key: &str) -> bool {
        let handle = {
            let renderers = self.ivars().renderers.borrow();
            renderers.first().and_then(|runtime| {
                runtime.with_renderer(|renderer| {
                    renderer
                        .mounted()
                        .and_then(|root| mounted_handle_for_key(root, key))
                        .cloned()
                })
            })
        };
        let Some(handle) = handle else {
            return false;
        };
        // SAFETY: The key identifies a live mounted NSButton; performClick
        // runs the ordinary target/action dispatch on the main thread.
        unsafe {
            let _: () = msg_send![handle.view(), performClick: std::ptr::null::<AnyObject>()];
        }
        true
    }

    fn dialog_probe_label_text(&self, key: &str) -> Option<String> {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().and_then(|runtime| {
            runtime.with_renderer(|renderer| {
                renderer
                    .mounted()
                    .and_then(|root| mounted_node_for_key(root, key))
                    .and_then(|node| match node.element().props() {
                        Props::Label { text, .. } => Some(text.clone()),
                        _ => None,
                    })
            })
        })
    }

    fn dialog_probe_mounted_exists(&self, key: &str) -> bool {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().is_some_and(|runtime| {
            runtime.with_renderer(|renderer| {
                renderer
                    .mounted()
                    .is_some_and(|root| mounted_handle_for_key(root, key).is_some())
            })
        })
    }

    /// Writes the attached sheet's content view into the capture directory.
    ///
    /// # Safety
    ///
    /// `sheet` must be a live NSWindow used on AppKit's main thread.
    unsafe fn capture_dialog_sheet(&self, sheet: &AnyObject, name: &str) {
        let Some(directory) = std::env::var_os("RINKA_APPKIT_WINDOW_CAPTURE_DIR") else {
            return;
        };
        let path = std::path::PathBuf::from(directory).join(name);
        // SAFETY: The sheet's content view renders itself on the main thread.
        let written = unsafe {
            let content: *mut AnyObject = msg_send![sheet, contentView];
            NonNull::new(content).is_some_and(|content| write_view_png(content.as_ref(), &path))
        };
        eprintln!(
            "Rinka dialog probe capture name={name} written={written} path={}",
            path.display()
        );
    }

    fn advance_dialog_probe(&self) {
        let Some(step) = self
            .ivars()
            .dialog_probe
            .borrow()
            .as_ref()
            .map(|probe| probe.step)
        else {
            return;
        };
        let Some(window) = self.ivars().windows.borrow().first().cloned() else {
            return;
        };
        // SAFETY: The retained primary window's attached sheet is read on the
        // main thread; a null sheet is represented as None.
        let sheet = unsafe {
            let sheet: *mut AnyObject = msg_send![window.as_object(), attachedSheet];
            NonNull::new(sheet)
        };
        match step {
            // Establish key status, then open the upload panel.
            0 => {
                if !self.probe_window_is_key() {
                    self.dialog_probe_retry("activation");
                    return;
                }
                if !self.dialog_probe_click_mounted("upload-files") {
                    self.fail_dialog_probe("click_upload", "element-not-mounted");
                    return;
                }
                eprintln!("Rinka dialog probe step=click_upload pass=true");
                self.advance_dialog_probe_step();
            }
            // The open panel is a sheet honoring the declared options.
            1 => {
                let Some(sheet) = sheet else {
                    self.dialog_probe_retry("open_panel_sheet");
                    return;
                };
                let expected_directory = dialog_probe_panel_directory()
                    .map_or_else(String::new, |path| path.display().to_string());
                // SAFETY: The attached sheet is the live NSOpenPanel; only
                // public panel properties are read.
                let (is_open_panel, choose_files, choose_directories, multiple, directory) = unsafe {
                    let is_open_panel: bool =
                        msg_send![sheet.as_ref(), isKindOfClass: objc2::class!(NSOpenPanel)];
                    let choose_files: bool = msg_send![sheet.as_ref(), canChooseFiles];
                    let choose_directories: bool = msg_send![sheet.as_ref(), canChooseDirectories];
                    let multiple: bool = msg_send![sheet.as_ref(), allowsMultipleSelection];
                    let url: *mut AnyObject = msg_send![sheet.as_ref(), directoryURL];
                    let path: *mut AnyObject = msg_send![url, path];
                    (
                        is_open_panel,
                        choose_files,
                        choose_directories,
                        multiple,
                        rust_string(path),
                    )
                };
                let directory_matches = dialog_probe_paths_match(&directory, &expected_directory);
                let passed = is_open_panel
                    && choose_files
                    && choose_directories
                    && multiple
                    && directory_matches;
                eprintln!(
                    "Rinka dialog probe step=open_panel_sheet is_open_panel={is_open_panel} can_choose_files={choose_files} can_choose_directories={choose_directories} allows_multiple={multiple} directory={directory} expected_directory={expected_directory} directory_matches={directory_matches} pass={passed}"
                );
                if !passed {
                    if let Some(probe) = self.ivars().dialog_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_dialog_probe();
                    return;
                }
                // SAFETY: The live panel renders and confirms on main.
                unsafe {
                    self.capture_dialog_sheet(sheet.as_ref(), "dialog-open-panel.png");
                    let _: () = msg_send![sheet.as_ref(), ok: std::ptr::null::<AnyObject>()];
                }
                self.advance_dialog_probe_step();
            }
            // The confirmed paths arrived as a message and re-rendered.
            2 => {
                if sheet.is_some() {
                    self.dialog_probe_retry("open_panel_round_trip");
                    return;
                }
                let Some(text) = self.dialog_probe_label_text("upload-first-path") else {
                    self.dialog_probe_retry("open_panel_round_trip");
                    return;
                };
                let expected = dialog_probe_panel_directory()
                    .map_or_else(String::new, |path| dialog_probe_compact_path(&path));
                let passed = text == expected;
                eprintln!(
                    "Rinka dialog probe step=open_panel_round_trip observed={text} expected={expected} pass={passed}"
                );
                if !passed {
                    if let Some(probe) = self.ivars().dialog_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_dialog_probe();
                    return;
                }
                if !self.dialog_probe_click_mounted("download-file") {
                    self.fail_dialog_probe("click_download", "element-not-mounted");
                    return;
                }
                self.advance_dialog_probe_step();
            }
            // The save panel is a sheet carrying the suggested filename.
            3 => {
                let Some(sheet) = sheet else {
                    self.dialog_probe_retry("save_panel_sheet");
                    return;
                };
                let expected_directory = dialog_probe_panel_directory()
                    .map_or_else(String::new, |path| path.display().to_string());
                // SAFETY: The attached sheet is the live NSSavePanel; only
                // public panel properties are read.
                let (is_save_panel, is_open_panel, filename, directory) = unsafe {
                    let is_save_panel: bool =
                        msg_send![sheet.as_ref(), isKindOfClass: objc2::class!(NSSavePanel)];
                    let is_open_panel: bool =
                        msg_send![sheet.as_ref(), isKindOfClass: objc2::class!(NSOpenPanel)];
                    let filename: *mut AnyObject = msg_send![sheet.as_ref(), nameFieldStringValue];
                    let url: *mut AnyObject = msg_send![sheet.as_ref(), directoryURL];
                    let path: *mut AnyObject = msg_send![url, path];
                    (
                        is_save_panel,
                        is_open_panel,
                        rust_string(filename),
                        rust_string(path),
                    )
                };
                let directory_matches = dialog_probe_paths_match(&directory, &expected_directory);
                let passed = is_save_panel
                    && !is_open_panel
                    && filename == "Cargo.toml"
                    && directory_matches;
                eprintln!(
                    "Rinka dialog probe step=save_panel_sheet is_save_panel={is_save_panel} is_open_panel={is_open_panel} suggested_filename={filename} directory={directory} expected_directory={expected_directory} directory_matches={directory_matches} pass={passed}"
                );
                if !passed {
                    if let Some(probe) = self.ivars().dialog_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_dialog_probe();
                    return;
                }
                // SAFETY: The live panel renders and confirms on main.
                unsafe {
                    self.capture_dialog_sheet(sheet.as_ref(), "dialog-save-panel.png");
                    let _: () = msg_send![sheet.as_ref(), ok: std::ptr::null::<AnyObject>()];
                }
                self.advance_dialog_probe_step();
            }
            // The confirmed save destination arrived as a message.
            4 => {
                if sheet.is_some() {
                    self.dialog_probe_retry("save_panel_round_trip");
                    return;
                }
                let Some(text) = self.dialog_probe_label_text("download-path") else {
                    self.dialog_probe_retry("save_panel_round_trip");
                    return;
                };
                let expected = dialog_probe_panel_directory()
                    .map_or_else(String::new, |path| {
                        dialog_probe_compact_path(&path.join("Cargo.toml"))
                    });
                let passed = text == expected;
                eprintln!(
                    "Rinka dialog probe step=save_panel_round_trip observed={text} expected={expected} pass={passed}"
                );
                if !passed {
                    if let Some(probe) = self.ivars().dialog_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_dialog_probe();
                    return;
                }
                if !self.dialog_probe_click_mounted("delete-file") {
                    self.fail_dialog_probe("click_delete", "element-not-mounted");
                    return;
                }
                self.advance_dialog_probe_step();
            }
            // The confirm alert is a window-modal sheet whose destructive
            // button never owns the return key.
            5 => {
                let Some(sheet) = sheet else {
                    self.dialog_probe_retry("confirm_sheet");
                    return;
                };
                // SAFETY: Sheet relationship, modal state, and button facts
                // are public AppKit API read from live objects on main.
                let passed = unsafe {
                    let application: *mut AnyObject =
                        msg_send![objc2::class!(NSApplication), sharedApplication];
                    let modal_window: *mut AnyObject = msg_send![application, modalWindow];
                    let app_modal = !modal_window.is_null();
                    let is_sheet: bool = msg_send![sheet.as_ref(), isSheet];
                    let parent: *mut AnyObject = msg_send![sheet.as_ref(), sheetParent];
                    let parent_is_window = parent == window.as_ptr();
                    let content: *mut AnyObject = msg_send![sheet.as_ref(), contentView];
                    let Some(content) = NonNull::new(content) else {
                        self.fail_dialog_probe("confirm_sheet", "sheet-has-no-content");
                        return;
                    };
                    let Some(delete) = find_button_titled(content.as_ref(), "Delete") else {
                        self.fail_dialog_probe("confirm_sheet", "delete-button-missing");
                        return;
                    };
                    let Some(cancel) = find_button_titled(content.as_ref(), "Cancel") else {
                        self.fail_dialog_probe("confirm_sheet", "cancel-button-missing");
                        return;
                    };
                    let delete_destructive: bool =
                        msg_send![delete.as_ref(), hasDestructiveAction];
                    let delete_key: *mut AnyObject = msg_send![delete.as_ref(), keyEquivalent];
                    let delete_key = rust_string(delete_key);
                    let cancel_key: *mut AnyObject = msg_send![cancel.as_ref(), keyEquivalent];
                    let cancel_key = rust_string(cancel_key);
                    for (name, button) in [("Delete", delete), ("Cancel", cancel)] {
                        let role: *mut AnyObject = msg_send![button.as_ref(), accessibilityRole];
                        let title: *mut AnyObject = msg_send![button.as_ref(), accessibilityTitle];
                        let destructive: bool = msg_send![button.as_ref(), hasDestructiveAction];
                        let key: *mut AnyObject = msg_send![button.as_ref(), keyEquivalent];
                        eprintln!(
                            "Rinka dialog probe ax button={name} role={} title={} destructive={destructive} key_equivalent={:?}",
                            rust_string(role),
                            rust_string(title),
                            rust_string(key)
                        );
                    }
                    let passed = !app_modal
                        && is_sheet
                        && parent_is_window
                        && delete_destructive
                        && delete_key != "\r"
                        && cancel_key == "\r";
                    eprintln!(
                        "Rinka dialog probe step=confirm_sheet app_modal={app_modal} is_sheet={is_sheet} parent_is_window={parent_is_window} delete_destructive={delete_destructive} delete_key_equivalent={delete_key:?} cancel_key_equivalent={cancel_key:?} pass={passed}"
                    );
                    self.capture_dialog_sheet(sheet.as_ref(), "dialog-confirm-sheet.png");
                    if passed {
                        // The destructive choice completes the round trip.
                        let _: () =
                            msg_send![delete.as_ref(), performClick: std::ptr::null::<AnyObject>()];
                    }
                    passed
                };
                if !passed {
                    if let Some(probe) = self.ivars().dialog_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_dialog_probe();
                    return;
                }
                self.advance_dialog_probe_step();
            }
            // The confirmed delete removed the row and cleared the selection.
            _ => {
                if sheet.is_some() {
                    self.dialog_probe_retry("delete_round_trip");
                    return;
                }
                let file_removed = !self.dialog_probe_mounted_exists("file-Cargo");
                let selection_cleared = self.dialog_probe_mounted_exists("inspector-state");
                if !file_removed || !selection_cleared {
                    self.dialog_probe_retry("delete_round_trip");
                    return;
                }
                eprintln!(
                    "Rinka dialog probe step=delete_round_trip file_removed={file_removed} selection_cleared={selection_cleared} pass=true"
                );
                self.capture_windows_to_directory("dialog-after-delete-");
                self.finish_dialog_probe();
            }
        }
    }

    fn finish_dialog_probe(&self) {
        let passed = self
            .ivars()
            .dialog_probe
            .borrow()
            .as_ref()
            .is_some_and(|probe| probe.passed);
        eprintln!(
            "Rinka dialog probe result={}",
            if passed { "PASS" } else { "FAIL" }
        );
        if std::env::var_os("RINKA_APPKIT_DIALOG_PROBE_HOLD").is_none() {
            // SAFETY: Diagnostic completion terminates only the current test app.
            unsafe {
                let application: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let _: () = msg_send![application, terminate: std::ptr::null::<AnyObject>()];
            }
        }
    }
}
