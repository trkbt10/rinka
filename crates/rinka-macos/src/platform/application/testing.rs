// In-process AppKit test host.
//
// This file generalizes the adapter's probe machinery (the settlement-wait
// discipline of the transition probe, `performClick:` and field-editor
// driving from the clipboard probe, synthetic `NSEvent` delivery from the
// pointer and accelerator probes, native menu dispatch from the menu-bar
// probe, and the in-process render capture of `write_view_capture`) into a
// reusable surface the `rinka-test` crate exposes to consumers.
//
// The host mounts the real adapter WITHOUT `-[NSApplication run]` taking
// over the process: the test owns the loop and pumps it in bounded bursts
// through `nextEventMatchingMask:untilDate:inMode:dequeue:` — the same
// dequeue path the native run loop uses — so queued synthetic events reach
// local monitors, `performSelector:afterDelay:` timers fire, and display
// updates run, while control returns to the test between bursts.

unsafe extern "C" {
    #[link_name = "NSDefaultRunLoopMode"]
    static DEFAULT_RUN_LOOP_MODE: *mut AnyObject;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    /// Returns the window-server session dictionary, or null when the
    /// process has no window-server session (headless CI, SSH daemons).
    fn CGSessionCopyCurrentDictionary() -> *const std::ffi::c_void;
}

/// `NSEventTypeKeyDown`.
const EVENT_KEY_DOWN: usize = 10;
/// The Return key's virtual key code on every Apple keyboard layout.
const KEY_CODE_RETURN: u16 = 36;

/// Reports whether this process can reach a window-server session.
///
/// AppKit cannot host windows without one; a consumer harness checks this
/// before mounting and surfaces the skip as a typed, logged reason instead
/// of silence.
pub fn window_server_session_available() -> bool {
    // SAFETY: The call takes no arguments and returns an owned CF object or
    // null; the owned dictionary is released before returning.
    unsafe {
        let session = CGSessionCopyCurrentDictionary();
        if session.is_null() {
            return false;
        }
        CFRelease(session);
        true
    }
}

/// Named settlement conditions observed on one main-loop turn.
///
/// The set is the one the transition probe's settlement wait already
/// enumerates (`delegate.rs`): a pending split restore, unsettled controlled
/// outline expansion, and unresolved semantic source widths each block
/// settlement, and the split-resize epoch must stay quiet across turns.
#[derive(Clone, Copy, Debug)]
pub struct SettleObservation {
    /// No split-view restore transaction is pending.
    pub split_restore_idle: bool,
    /// Every controlled outline's native expansion matches its declaration.
    pub outline_state_settled: bool,
    /// Every visible semantic source list has a resolved fitting width.
    pub source_widths_resolved: bool,
    /// Current split-resize transaction counter; settlement requires it to
    /// stay unchanged across quiet turns.
    pub split_epoch: u64,
}

impl SettleObservation {
    /// Returns the names of the conditions this observation leaves unmet,
    /// excluding epoch quiescence (which only spans multiple turns).
    pub fn unmet_conditions(&self) -> Vec<&'static str> {
        let mut unmet = Vec::new();
        if !self.split_restore_idle {
            unmet.push("split-restore-idle");
        }
        if !self.outline_state_settled {
            unmet.push("outline-state-settled");
        }
        if !self.source_widths_resolved {
            unmet.push("source-widths-resolved");
        }
        unmet
    }
}

/// Drivable in-process AppKit application host for consumer tests.
///
/// One host owns one mounted [`ApplicationSpec`] on the process main thread.
/// The test pumps the run loop explicitly through [`Self::pump_turn`] (or a
/// settlement wait built on it) and drives mounted elements through the
/// element verbs; nothing here posts events to any other process or moves
/// the user's pointer.
pub struct AppKitTestHost {
    delegate: Retained<ApplicationDelegate>,
    unmounted: Cell<bool>,
}

impl fmt::Debug for AppKitTestHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppKitTestHost")
            .field("windows", &self.window_count())
            .finish()
    }
}

impl AppKitTestHost {
    /// Mounts the application against the live AppKit adapter without
    /// entering `-[NSApplication run]`.
    ///
    /// Must be called on the process main thread of a session that can
    /// reach the window server (see [`window_server_session_available`]).
    /// The initial layout passes the adapter schedules through the run loop
    /// are pumped before returning, so the mounted tree is complete.
    pub fn mount(application: ApplicationSpec) -> Result<Self, AppKitError> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            AppKitError("the AppKit test host must be mounted on the process main thread".into())
        })?;
        if !window_server_session_available() {
            return Err(AppKitError(
                "no window-server session is available to host AppKit windows".into(),
            ));
        }
        let expected_windows = application.windows.len();
        // SAFETY: sharedApplication is the AppKit singleton on the main
        // thread; launch completion runs exactly once per process, exactly
        // as `run` performs it before showing windows.
        let app: *mut AnyObject =
            unsafe { msg_send![objc2::class!(NSApplication), sharedApplication] };
        let delegate = ApplicationDelegate::new(mtm, application);
        // SAFETY: NSApplication keeps a non-owning delegate pointer; the
        // host retains the delegate until unmount.
        unsafe {
            let _: () = msg_send![app, setDelegate: &*delegate];
        }
        static FINISH_LAUNCHING: std::sync::Once = std::sync::Once::new();
        FINISH_LAUNCHING.call_once(|| {
            // SAFETY: finishLaunching completes AppKit launch on the main
            // thread; it must run before application windows are built.
            unsafe {
                let _: () = msg_send![app, finishLaunching];
            }
        });
        // `finishLaunching` delivers applicationDidFinishLaunching, whose
        // handler consumes the retained spec; on later mounts in the same
        // process the notification no longer fires, so the explicit call
        // shows the windows. The spec take makes the pair idempotent.
        if let Err(exception) = objc2::exception::catch(AssertUnwindSafe(|| {
            delegate.show_initial_windows();
        })) {
            return Err(AppKitError(format!(
                "AppKit rejected the native view tree: {exception:?}"
            )));
        }
        let host = Self {
            delegate,
            unmounted: Cell::new(false),
        };
        // Initial layout is finished through the run loop
        // (`refreshInitialLayout:` / `restoreInitialWindowSizes:` are
        // delayed selectors); give it bounded turns instead of one blind
        // wait.
        for _ in 0..50 {
            host.pump_turn();
            if host.window_count() == expected_windows {
                break;
            }
        }
        if host.window_count() != expected_windows {
            host.unmount_windows();
            return Err(AppKitError(format!(
                "mounted {} of {expected_windows} declared windows",
                host.window_count()
            )));
        }
        if let Some(error) = host.take_render_error() {
            host.unmount_windows();
            return Err(AppKitError(error));
        }
        Ok(host)
    }

    /// Runs the AppKit loop for one short burst (about 20 milliseconds),
    /// dequeuing and dispatching every pending event exactly as
    /// `-[NSApplication run]` would.
    pub fn pump_turn(&self) {
        self.pump(0.02);
    }

    /// Runs the AppKit loop until `seconds` elapse, dispatching pending
    /// events through `sendEvent:` and firing default-mode timers.
    pub fn pump(&self, seconds: f64) {
        autoreleasepool(|_| {
            // SAFETY: All receivers are main-thread AppKit objects. The
            // dequeue call runs the default-mode run loop while waiting, so
            // delayed selectors and queued notifications are serviced; every
            // dequeued event goes through ordinary application dispatch.
            unsafe {
                let app: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let deadline: *mut AnyObject = msg_send![
                    objc2::class!(NSDate),
                    dateWithTimeIntervalSinceNow: seconds
                ];
                loop {
                    let event: *mut AnyObject = msg_send![app,
                        nextEventMatchingMask: usize::MAX,
                        untilDate: deadline,
                        inMode: DEFAULT_RUN_LOOP_MODE,
                        dequeue: true
                    ];
                    let Some(event) = NonNull::new(event) else {
                        break;
                    };
                    let _: () = msg_send![app, sendEvent: event.as_ref()];
                    let remaining: f64 = msg_send![deadline, timeIntervalSinceNow];
                    if remaining <= 0.0 {
                        break;
                    }
                }
                let _: () = msg_send![app, updateWindows];
            }
        });
    }

    /// Returns how many declared windows are currently hosted.
    pub fn window_count(&self) -> usize {
        self.delegate.ivars().windows.borrow().len()
    }

    /// Reads the mounted element tree of the window at `index`.
    ///
    /// The closure runs while the renderer is borrowed: it must only read
    /// (finding handles, snapshotting props), never dispatch events that
    /// would re-render.
    pub fn with_mounted<R>(
        &self,
        index: usize,
        read: impl FnOnce(&MountedNode<AppKitHandle>) -> R,
    ) -> Option<R> {
        let renderers = self.delegate.ivars().renderers.borrow();
        let runtime = renderers.get(index)?;
        runtime.with_renderer(|renderer| renderer.mounted().map(read))
    }

    /// Takes the most recent asynchronous render error from any window.
    pub fn take_render_error(&self) -> Option<String> {
        let renderers = self.delegate.ivars().renderers.borrow();
        renderers
            .iter()
            .find_map(|runtime| runtime.take_error().map(|error| error.to_string()))
    }

    /// Observes the adapter's named settlement conditions for this turn.
    pub fn observe_settlement(&self) -> SettleObservation {
        let ivars = self.delegate.ivars();
        SettleObservation {
            split_restore_idle: !ivars.split_restore_pending.get(),
            outline_state_settled: registered_outline_state_is_settled(&ivars.list_registries),
            source_widths_resolved: registered_visible_source_widths(&ivars.list_registries)
                .all_widths_resolved,
            split_epoch: ivars.split_resize_epoch.get(),
        }
    }

    /// Presses a mounted native button or checkbox through `performClick:`,
    /// driving its connected target/action synchronously — the same dispatch
    /// a user click performs (clipboard-probe precedent).
    pub fn press(&self, handle: &AppKitHandle) -> Result<(), AppKitError> {
        if !matches!(
            handle.element_kind(),
            Some(ElementKind::Button | ElementKind::Toggle)
        ) {
            return Err(AppKitError(format!(
                "press expects a button or toggle, found {:?}",
                handle.element_kind()
            )));
        }
        let view = handle.0.view.clone();
        // SAFETY: The retained view is the mounted NSButton; performClick:
        // runs its target/action on AppKit's main thread. The renderer is
        // not borrowed here, so the resulting update may re-render freely.
        unsafe {
            let _: () = msg_send![view.as_object(), performClick: std::ptr::null::<AnyObject>()];
        }
        Ok(())
    }

    /// Sends one primary-button click through `NSWindow sendEvent:` at the
    /// center of the mounted element — real hit testing and responder
    /// dispatch, confined to this process (pointer-probe precedent).
    pub fn click_center(&self, handle: &AppKitHandle) -> Result<(), AppKitError> {
        let view = handle.0.view.clone();
        // SAFETY: The mounted view, its window, and NSEvent construction are
        // used on AppKit's main thread; sendEvent: performs ordinary event
        // dispatch confined to this application's window.
        let delivered = unsafe {
            let window: *mut AnyObject = msg_send![view.as_object(), window];
            if window.is_null() {
                false
            } else {
                let bounds: Rect = msg_send![view.as_object(), bounds];
                let center = Point {
                    x: bounds.origin.x + bounds.size.width / 2.0,
                    y: bounds.origin.y + bounds.size.height / 2.0,
                };
                let in_window: Point = msg_send![
                    view.as_object(),
                    convertPoint: center,
                    toView: std::ptr::null::<AnyObject>()
                ];
                let window_number: isize = msg_send![window, windowNumber];
                // NSEventTypeLeftMouseDown = 1, NSEventTypeLeftMouseUp = 2.
                for event_type in [1_usize, 2_usize] {
                    let event: *mut AnyObject = msg_send![objc2::class!(NSEvent),
                        mouseEventWithType: event_type,
                        location: in_window,
                        modifierFlags: 0_usize,
                        timestamp: 0.0_f64,
                        windowNumber: window_number,
                        context: std::ptr::null::<AnyObject>(),
                        eventNumber: 0_isize,
                        clickCount: 1_isize,
                        pressure: 1.0_f32
                    ];
                    let _: () = msg_send![window, sendEvent: event];
                }
                true
            }
        };
        if delivered {
            Ok(())
        } else {
            Err(AppKitError("the element's view is not in a window".into()))
        }
    }

    /// Focuses a mounted native text field and inserts `text` through its
    /// field editor — a real editing session, the same path the clipboard
    /// probe proved for native-field editing.
    pub fn type_text(&self, handle: &AppKitHandle, text: &str) -> Result<(), AppKitError> {
        if handle.element_kind() != Some(ElementKind::Input) {
            return Err(AppKitError(format!(
                "type_text expects an input, found {:?}",
                handle.element_kind()
            )));
        }
        let view = handle.0.view.clone();
        // SAFETY: makeFirstResponder begins a field editor session on the
        // main thread; the focused NSText inserts the string exactly as
        // typed characters arrive from the keyboard path.
        unsafe {
            let window: *mut AnyObject = msg_send![view.as_object(), window];
            let Some(window) = NonNull::new(window) else {
                return Err(AppKitError("the input's view is not in a window".into()));
            };
            let accepted: bool =
                msg_send![window.as_ref(), makeFirstResponder: view.as_object()];
            if !accepted || !first_responder_is_text_input(window.as_ref()) {
                return Err(AppKitError(
                    "the input refused first-responder status".into(),
                ));
            }
            let responder: *mut AnyObject = msg_send![window.as_ref(), firstResponder];
            let value = ns_string(text);
            let _: () = msg_send![responder, insertText: value.as_object()];
        }
        Ok(())
    }

    /// Commits the focused text field by sending a Return key-down through
    /// `NSWindow sendEvent:`, so the field editor's `insertNewline:` fires
    /// the control's action exactly as the keyboard would.
    pub fn commit_text(&self, handle: &AppKitHandle) -> Result<(), AppKitError> {
        let view = handle.0.view.clone();
        // SAFETY: The synthesized key event carries this window's own number
        // and dispatches through ordinary responder routing on the main
        // thread.
        unsafe {
            let window: *mut AnyObject = msg_send![view.as_object(), window];
            let Some(window) = NonNull::new(window) else {
                return Err(AppKitError("the input's view is not in a window".into()));
            };
            let window_number: isize = msg_send![window.as_ref(), windowNumber];
            let characters = ns_string("\r");
            let event: *mut AnyObject = msg_send![objc2::class!(NSEvent),
                keyEventWithType: EVENT_KEY_DOWN,
                location: Point::default(),
                modifierFlags: 0_usize,
                timestamp: 0.0_f64,
                windowNumber: window_number,
                context: std::ptr::null::<AnyObject>(),
                characters: characters.as_object(),
                charactersIgnoringModifiers: characters.as_object(),
                isARepeat: false,
                keyCode: KEY_CODE_RETURN
            ];
            let _: () = msg_send![window.as_ref(), sendEvent: event];
        }
        Ok(())
    }

    /// Reads the element's value off the native control's accessibility
    /// surface (`accessibilityValue`), falling back to `stringValue`.
    ///
    /// This is the in-process read of the same attribute assistive
    /// technology sees; no external AX API (and therefore no TCC grant) is
    /// involved.
    pub fn read_value(&self, handle: &AppKitHandle) -> Option<String> {
        let view = handle.0.view.clone();
        // SAFETY: Both reads are main-thread queries of the retained view;
        // string conversion copies before anything releases.
        unsafe {
            let value: *mut AnyObject = msg_send![view.as_object(), accessibilityValue];
            if let Some(value) = NonNull::new(value) {
                let is_string: bool =
                    msg_send![value.as_ref(), isKindOfClass: objc2::class!(NSString)];
                if is_string {
                    return Some(rust_string(value.as_ptr()));
                }
                let described: *mut AnyObject = msg_send![value.as_ref(), description];
                return Some(rust_string(described));
            }
            let responds: bool =
                msg_send![view.as_object(), respondsToSelector: sel!(stringValue)];
            if responds {
                let value: *mut AnyObject = msg_send![view.as_object(), stringValue];
                return Some(rust_string(value));
            }
        }
        None
    }

    /// Reads the native control's accessibility label.
    pub fn read_accessibility_label(&self, handle: &AppKitHandle) -> Option<String> {
        let view = handle.0.view.clone();
        // SAFETY: accessibilityLabel is a main-thread read of the retained view.
        unsafe {
            let label: *mut AnyObject = msg_send![view.as_object(), accessibilityLabel];
            NonNull::new(label).map(|label| rust_string(label.as_ptr()))
        }
    }

    /// Reads the native enabled state of a control, or `None` when the view
    /// has no enabled concept.
    pub fn is_enabled(&self, handle: &AppKitHandle) -> Option<bool> {
        let view = handle.0.view.clone();
        // SAFETY: The selector check guards the read; both run on the main
        // thread against the retained view.
        unsafe {
            let responds: bool = msg_send![view.as_object(), respondsToSelector: sel!(isEnabled)];
            responds.then(|| msg_send![view.as_object(), isEnabled])
        }
    }

    /// Reads the native checked state of a toggle, or `None` for other
    /// elements.
    pub fn is_checked(&self, handle: &AppKitHandle) -> Option<bool> {
        if handle.element_kind() != Some(ElementKind::Toggle) {
            return None;
        }
        let view = handle.0.view.clone();
        // SAFETY: The retained view is the mounted NSButton checkbox.
        let state: isize = unsafe { msg_send![view.as_object(), state] };
        Some(state != 0)
    }

    /// Selects a mounted list row in its native table, driving the same
    /// consumer path a user click takes.
    ///
    /// `selectRowIndexes:byExtendingSelection:` posts
    /// `NSTableViewSelectionDidChangeNotification` synchronously; rinka's
    /// table delegate translates it into the row's stable activate binding.
    /// The reconciler's own programmatic reloads stay silent through the
    /// `suppress_selection` guard — which this verb never sets — so this
    /// selection is observed exactly like user input. This closes the
    /// recorded gap that the pointer probe cannot drive collection rows.
    pub fn select_row(&self, handle: &AppKitHandle) -> Result<(), AppKitError> {
        if handle.element_kind() != Some(ElementKind::ListRow) {
            return Err(AppKitError(format!(
                "select_row expects a list row, found {:?}",
                handle.element_kind()
            )));
        }
        // Resolve the native table and row index first, then release every
        // borrow: the selection notification dispatches consumer state
        // synchronously, and the resulting reconciliation mutates the same
        // row records.
        let record = handle
            .0
            .list_row
            .borrow()
            .clone()
            .ok_or_else(|| AppKitError("the list row has no native record".into()))?;
        let table = record
            .borrow()
            .table
            .borrow()
            .clone()
            .ok_or_else(|| AppKitError("the list row is not attached to a table".into()))?;
        // SAFETY: The retained table is the row's mounted NSTableView or
        // NSOutlineView; rowForItem: resolves the visible row index for
        // outline-backed patterns, and a flat table's index is the record's
        // position in its rinka delegate.
        let row_index: isize = unsafe {
            let is_outline: bool =
                msg_send![table.as_object(), isKindOfClass: objc2::class!(NSOutlineView)];
            if is_outline {
                let identity = record.borrow().outline_identity.clone();
                msg_send![table.as_object(), rowForItem: identity.as_object()]
            } else {
                let delegate: *mut AnyObject = msg_send![table.as_object(), delegate];
                let Some(delegate) = NonNull::new(delegate) else {
                    return Err(AppKitError("the native table has no delegate".into()));
                };
                let is_rinka: bool =
                    msg_send![delegate.as_ref(), isKindOfClass: TableDelegate::class()];
                if !is_rinka {
                    return Err(AppKitError(
                        "the native table's delegate is not rinka's".into(),
                    ));
                }
                // SAFETY: The class check above proves the delegate is the
                // adapter's TableDelegate, so the reference cast is sound.
                let delegate: &TableDelegate = &*delegate.as_ptr().cast::<TableDelegate>();
                delegate
                    .ivars()
                    .rows
                    .borrow()
                    .iter()
                    .position(|candidate| Rc::ptr_eq(candidate, &record))
                    .and_then(|index| isize::try_from(index).ok())
                    .unwrap_or(-1)
            }
        };
        let Ok(row_index) = usize::try_from(row_index) else {
            return Err(AppKitError(
                "the list row is not visible in its native table (a collapsed \
                 ancestor hides it)"
                    .into(),
            ));
        };
        // SAFETY: The selection runs on the main thread with no renderer or
        // record borrow held; the delegate notification may re-render.
        unsafe {
            let indexes: *mut AnyObject = msg_send![objc2::class!(NSIndexSet),
                indexSetWithIndex: row_index
            ];
            let _: () = msg_send![table.as_object(),
                selectRowIndexes: indexes,
                byExtendingSelection: false
            ];
        }
        Ok(())
    }

    /// Posts one synthetic key-down through the real event queue, exactly
    /// like the accelerator probe: the next pump dequeues it through the
    /// same path hardware input takes, so local monitors and accelerator
    /// routing observe it.
    pub fn post_key(&self, characters: &str, key_code: u16, modifier_flags: usize) {
        self.delegate
            .post_probe_key(characters, characters, key_code, modifier_flags, false);
    }

    /// Activates an item of the installed application menu bar through
    /// native menu dispatch (menu-bar-probe precedent): app-defined items
    /// fire their retained target's `performMenuBarAction:`, native
    /// nil-target items dispatch through the responder chain.
    pub fn activate_menu_item(
        &self,
        menu_title: &str,
        item_title: &str,
    ) -> Result<(), AppKitError> {
        let window = self.delegate.ivars().windows.borrow().first().cloned();
        // SAFETY: All receivers are retained menu objects on the main
        // thread; both dispatch paths are the ones AppKit itself uses for a
        // chosen item.
        let activated = unsafe {
            activate_menu_bar_item(menu_title, item_title)
                || dispatch_native_menu_item(menu_title, item_title, window.as_ref())
        };
        if activated {
            Ok(())
        } else {
            Err(AppKitError(format!(
                "no menu item titled '{item_title}' in menu '{menu_title}' could be activated"
            )))
        }
    }

    /// Opens the element's retained context menu through the accessibility
    /// show-menu action, cancelled by a timer so the tracking session never
    /// waits on a pointer (context-menu-probe precedent). Returns whether
    /// the native menu actually began tracking.
    pub fn open_context_menu(&self, handle: &AppKitHandle) -> Result<bool, AppKitError> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            AppKitError("context menus open on the process main thread".into())
        })?;
        let view = handle.0.view.clone();
        // SAFETY: The retained view's menu and the accessibility action are
        // main-thread calls; drive_menu_open schedules the cancel timer
        // before the blocking tracking session begins.
        unsafe {
            let menu: *mut AnyObject = msg_send![view.as_object(), menu];
            let Some(menu) = NonNull::new(menu) else {
                return Err(AppKitError("the element retains no context menu".into()));
            };
            let (performed, opened) =
                drive_menu_open(mtm, menu.as_ref(), "harness-context-menu", || {
                    let performed: bool =
                        msg_send![view.as_object(), accessibilityPerformShowMenu];
                    performed
                });
            Ok(performed && opened)
        }
    }

    /// Activates one item of the element's retained context menu through the
    /// item's native target/action pair — the dispatch AppKit performs for a
    /// chosen item — without opening a tracking session.
    pub fn activate_context_menu_item(
        &self,
        handle: &AppKitHandle,
        item_title: &str,
    ) -> Result<(), AppKitError> {
        let view = handle.0.view.clone();
        // SAFETY: The retained view's menu and its items are read on the
        // main thread; performActionForItemAtIndex: validates and dispatches
        // exactly as a chosen menu item does.
        unsafe {
            let menu: *mut AnyObject = msg_send![view.as_object(), menu];
            let Some(menu) = NonNull::new(menu) else {
                return Err(AppKitError("the element retains no context menu".into()));
            };
            let Some(item) = menu_item_titled(menu.as_ref(), item_title) else {
                return Err(AppKitError(format!(
                    "the context menu has no item titled '{item_title}'"
                )));
            };
            let index: isize = msg_send![menu.as_ref(), indexOfItem: item.as_object()];
            if index < 0 {
                return Err(AppKitError(format!(
                    "the context menu no longer contains '{item_title}'"
                )));
            }
            let _: () = msg_send![menu.as_ref(), performActionForItemAtIndex: index];
        }
        Ok(())
    }

    /// Renders the window at `index` into a PNG at its backing scale using
    /// the in-process capture path (no screen-recording permission).
    pub fn capture_window_png(
        &self,
        index: usize,
        path: &std::path::Path,
    ) -> Result<(), AppKitError> {
        let window = self
            .delegate
            .ivars()
            .windows
            .borrow()
            .get(index)
            .cloned()
            .ok_or_else(|| AppKitError(format!("no window at index {index}")))?;
        // SAFETY: The retained NSWindow's content view renders itself on
        // AppKit's main thread inside this call.
        let written = unsafe { write_window_content_png(window.as_object(), path) };
        if written {
            Ok(())
        } else {
            Err(AppKitError(format!(
                "window capture failed for {}",
                path.display()
            )))
        }
    }

    /// Renders one mounted element's view into a PNG at the window's
    /// backing scale (the `write_view_capture` path the keyed-view capture
    /// evidence already uses).
    pub fn capture_element_png(
        &self,
        handle: &AppKitHandle,
        path: &std::path::Path,
    ) -> Result<(), AppKitError> {
        let view = handle.0.view.clone();
        // SAFETY: The mounted handle owns a live NSView rendered on AppKit's
        // main thread.
        let written = unsafe { write_view_capture(view.as_object(), path) };
        if written {
            Ok(())
        } else {
            Err(AppKitError(format!(
                "element capture failed for {}",
                path.display()
            )))
        }
    }

    fn unmount_windows(&self) {
        if self.unmounted.replace(true) {
            return;
        }
        // SAFETY: Teardown runs on the main thread. The delegate is detached
        // and its observers removed BEFORE windows close, so closing the
        // last window cannot re-enter delegate policy (its
        // `applicationShouldTerminateAfterLastWindowClosed:` answer would
        // otherwise let a later pump terminate the test process). No pump
        // runs after this point for this mount.
        unsafe {
            let app: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, setDelegate: std::ptr::null::<AnyObject>()];
            let center: *mut AnyObject =
                msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![center, removeObserver: &*self.delegate];
            if let Some(monitor) = self.delegate.ivars().key_monitor.borrow_mut().take() {
                let _: () = msg_send![objc2::class!(NSEvent), removeMonitor: monitor.as_object()];
            }
            for window in self.delegate.ivars().windows.borrow().iter() {
                let _: () = msg_send![window.as_object(), setDelegate: std::ptr::null::<AnyObject>()];
                let _: () = msg_send![window.as_object(), close];
            }
        }
    }
}

impl Drop for AppKitTestHost {
    fn drop(&mut self) {
        self.unmount_windows();
    }
}
