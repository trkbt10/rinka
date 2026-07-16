// In-process diagnostic for the native context-menu realization.
//
// `RINKA_APPKIT_CONTEXT_MENU_PROBE=1` drives the real explorer file table
// after initial layout: it reads the retained NSMenu on a row cell, opens it
// through a fabricated right-click, a fabricated ctrl-click, and the
// accessibility show-menu action (each cancelled by a timer so the probe
// never waits on a pointer), dispatches items through their native
// target/action pair, and asserts that deletion, enabled reconciliation, and
// checkmark reconciliation reach the retained native objects. When
// `RINKA_APPKIT_CONTEXT_MENU_PROBE_CAPTURE_DIR` names a directory, the probe
// writes PNG captures of this process's own windows — including the open
// menu window — through the window server's self-capture path, which needs
// no screen-recording grant. Every step prints one `Rinka context-menu
// probe` line and the process terminates after the summary line.

/// Opaque CoreGraphics image referenced across the capture FFI boundary.
#[repr(C)]
struct CGImage {
    _opaque: [u8; 0],
}

// SAFETY: CGImageRef is a pointer to the opaque CGImage struct; this encoding
// matches the public CoreGraphics ABI that AppKit methods declare.
unsafe impl objc2::RefEncode for CGImage {
    const ENCODING_REF: objc2::Encoding =
        objc2::Encoding::Pointer(&objc2::Encoding::Struct("CGImage", &[]));
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    /// Renders one window this process owns into a CGImage.
    fn CGWindowListCreateImage(
        screen_bounds: Rect,
        list_option: u32,
        window_id: u32,
        image_option: u32,
    ) -> *mut CGImage;
    /// Copies the window-server description list as a CFArray of
    /// CFDictionaries, toll-free bridged to NSArray/NSDictionary.
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> *mut AnyObject;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: *const std::ffi::c_void);
}

/// `kCGWindowListOptionOnScreenOnly | kCGWindowListOptionExcludingDesktopElements`.
const WINDOW_LIST_ON_SCREEN_EXCLUDING_DESKTOP: u32 = 1 | 16;
/// `kCGWindowListOptionIncludingWindow`.
const WINDOW_LIST_INCLUDING_WINDOW: u32 = 8;
/// `NSEventTypeLeftMouseDown`.
const EVENT_LEFT_MOUSE_DOWN: usize = 1;
/// `NSEventTypeLeftMouseUp`.
const EVENT_LEFT_MOUSE_UP: usize = 2;
/// `NSEventTypeRightMouseDown`.
const EVENT_RIGHT_MOUSE_DOWN: usize = 3;
/// `NSEventModifierFlagControl`.
const MODIFIER_CONTROL: usize = 1 << 18;

#[derive(Debug, Default)]
struct MenuTrackingObserverIvars {
    opened: Cell<bool>,
    capture_label: RefCell<String>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuTrackingObserverIvars]
    struct MenuTrackingObserver;

    // SAFETY: NSObjectProtocol adds no invariants beyond the NSObject superclass.
    unsafe impl NSObjectProtocol for MenuTrackingObserver {}

    impl MenuTrackingObserver {
        #[unsafe(method(menuDidBeginTracking:))]
        fn menu_did_begin_tracking(&self, _notification: &AnyObject) {
            self.ivars().opened.set(true);
        }

        #[unsafe(method(cancelProbeMenu:))]
        fn cancel_probe_menu(&self, timer: &AnyObject) {
            // The open menu is still on screen while its tracking loop runs
            // this timer, so this is the moment to photograph it.
            if self.ivars().opened.get() {
                let label = format!("{}-open", self.ivars().capture_label.borrow());
                // SAFETY: The capture reads only this process's own windows on
                // the main thread.
                unsafe {
                    capture_step_windows(&label);
                }
            }
            // SAFETY: The timer's userInfo is the NSMenu this probe opened;
            // cancelling tracking on a closed menu is harmless.
            unsafe {
                let menu: *mut AnyObject = msg_send![timer, userInfo];
                if let Some(menu) = NonNull::new(menu) {
                    let _: () = msg_send![menu.as_ref(), cancelTracking];
                }
            }
        }
    }
);

impl MenuTrackingObserver {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(MenuTrackingObserverIvars::default());
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }
}

impl ApplicationDelegate {
    fn begin_context_menu_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_CONTEXT_MENU_PROBE").is_none() {
            return;
        }
        let table = {
            let windows = self.ivars().windows.borrow();
            windows.first().and_then(|window| {
                // SAFETY: The retained main window is read on the main thread
                // after initial layout.
                unsafe {
                    let content: *mut AnyObject = msg_send![window.as_object(), contentView];
                    NonNull::new(content).and_then(|content| find_files_table(content.as_ref()))
                }
            })
        };
        let Some(table) = table else {
            eprintln!("Rinka context-menu probe step=locate-table pass=false");
            eprintln!("Rinka context-menu probe result=FAIL");
            terminate_probe_application();
            return;
        };
        eprintln!("Rinka context-menu probe step=locate-table pass=true");

        // SAFETY: Every step reads and drives retained AppKit objects on the
        // main thread; item activation runs the same target/action pair
        // AppKit invokes for a chosen menu item.
        let passed = unsafe {
            capture_step_windows("baseline");
            let structure = probe_menu_structure(table.as_object());
            let right_click = probe_pointer_menu(
                self.mtm(),
                table.as_object(),
                "right-click",
                EVENT_RIGHT_MOUSE_DOWN,
                0,
            );
            let ctrl_click = probe_pointer_menu(
                self.mtm(),
                table.as_object(),
                "ctrl-click",
                EVENT_LEFT_MOUSE_DOWN,
                MODIFIER_CONTROL,
            );
            let ax_show = probe_ax_show_menu(self.mtm(), table.as_object());
            let activation = probe_delete_activation(self.ivars(), table.as_object());
            flush_window_rendering(self.ivars());
            capture_step_windows("after-delete");
            let enabled = probe_duplicate_enabled_reconcile(table.as_object());
            let checkmark = probe_favorite_checkmark_reconcile(table.as_object());
            flush_window_rendering(self.ivars());
            capture_step_windows("after-reconcile");
            structure && right_click && ctrl_click && ax_show && activation && enabled && checkmark
        };
        eprintln!(
            "Rinka context-menu probe result={}",
            if passed { "PASS" } else { "FAIL" }
        );
        terminate_probe_application();
    }
}

/// Renders pending view updates and commits them to the window server so a
/// following capture shows the reconciled state instead of the last flushed
/// frame.
fn flush_window_rendering(ivars: &ApplicationDelegateIvars) {
    let windows = ivars.windows.borrow();
    let Some(window) = windows.first() else {
        return;
    };
    // SAFETY: The retained window is displayed on the main thread; the Core
    // Animation flush commits the freshly rendered layer tree.
    unsafe {
        let _: () = msg_send![window.as_object(), displayIfNeeded];
        let _: () = msg_send![objc2::class!(CATransaction), flush];
    }
}

fn terminate_probe_application() {
    if std::env::var_os("RINKA_APPKIT_CONTEXT_MENU_PROBE_HOLD").is_some() {
        return;
    }
    // SAFETY: Diagnostic completion terminates only the current test app.
    unsafe {
        let application: *mut AnyObject =
            msg_send![objc2::class!(NSApplication), sharedApplication];
        let _: () = msg_send![application, terminate: std::ptr::null::<AnyObject>()];
    }
}

/// Finds the file table: the retained NSOutlineView whose accessibility label
/// names the file listing.
unsafe fn find_files_table(view: &AnyObject) -> Option<Id> {
    // SAFETY: The receiver is a live NSView on the main thread.
    unsafe {
        let is_outline: bool = msg_send![view, isKindOfClass: objc2::class!(NSOutlineView)];
        if is_outline {
            let label: *mut AnyObject = msg_send![view, accessibilityLabel];
            if rust_string(label).starts_with("Files in") {
                return Some(Id::from_borrowed(
                    view as *const AnyObject as *mut AnyObject,
                ));
            }
        }
        let subviews: *mut AnyObject = msg_send![view, subviews];
        let count: usize = msg_send![subviews, count];
        for index in 0..count {
            let child: *mut AnyObject = msg_send![subviews, objectAtIndex: index];
            if let Some(child) = NonNull::new(child)
                && let Some(found) = find_files_table(child.as_ref())
            {
                return Some(found);
            }
        }
    }
    None
}

/// Returns the primary-column cell view of the row with the given title.
unsafe fn find_row_cell(table: &AnyObject, title: &str) -> Option<Id> {
    // SAFETY: The receiver is a live NSOutlineView; makeIfNecessary creates
    // the same cells the delegate serves during display.
    unsafe {
        let rows: isize = msg_send![table, numberOfRows];
        for row in 0..rows {
            let cell: *mut AnyObject = msg_send![
                table,
                viewAtColumn: 0_isize,
                row: row,
                makeIfNecessary: true
            ];
            let Some(cell) = NonNull::new(cell) else {
                continue;
            };
            let text_field: *mut AnyObject = msg_send![cell.as_ref(), textField];
            if text_field.is_null() {
                continue;
            }
            let value: *mut AnyObject = msg_send![text_field, stringValue];
            if rust_string(value) == title {
                return Some(Id::from_borrowed(cell.as_ptr()));
            }
        }
    }
    None
}

unsafe fn menu_item_titled(menu: &AnyObject, title: &str) -> Option<Id> {
    let title = ns_string(title);
    // SAFETY: The receiver is a live NSMenu.
    unsafe {
        let item: *mut AnyObject = msg_send![menu, itemWithTitle: title.as_object()];
        NonNull::new(item).map(|item| Id::from_borrowed(item.as_ptr()))
    }
}

/// Asserts the retained NSMenu on the README.md row matches the declared
/// model: seven entries with separators, a nested Open With submenu, and the
/// destructive Delete command.
unsafe fn probe_menu_structure(table: &AnyObject) -> bool {
    // SAFETY: All receivers are live AppKit objects on the main thread.
    unsafe {
        let Some(cell) = find_row_cell(table, "README.md") else {
            eprintln!("Rinka context-menu probe step=structure error=row-missing pass=false");
            return false;
        };
        let menu: *mut AnyObject = msg_send![cell.as_object(), menu];
        let Some(menu) = NonNull::new(menu) else {
            eprintln!("Rinka context-menu probe step=structure error=menu-missing pass=false");
            return false;
        };
        let menu = menu.as_ref();
        let count: isize = msg_send![menu, numberOfItems];
        let separator_second: *mut AnyObject = msg_send![menu, itemAtIndex: 2_isize];
        let separator_sixth: *mut AnyObject = msg_send![menu, itemAtIndex: 5_isize];
        let separators_pass = NonNull::new(separator_second).is_some_and(|item| {
            let is_separator: bool = msg_send![item.as_ref(), isSeparatorItem];
            is_separator
        }) && NonNull::new(separator_sixth).is_some_and(|item| {
            let is_separator: bool = msg_send![item.as_ref(), isSeparatorItem];
            is_separator
        });
        let rename_pass = menu_item_titled(menu, "Rename").is_some_and(|item| {
            let enabled: bool = msg_send![item.as_object(), isEnabled];
            enabled
        });
        let duplicate_pass = menu_item_titled(menu, "Duplicate").is_some_and(|item| {
            let enabled: bool = msg_send![item.as_object(), isEnabled];
            enabled
        });
        let submenu_pass = menu_item_titled(menu, "Open With").is_some_and(|item| {
            let has_submenu: bool = msg_send![item.as_object(), hasSubmenu];
            if !has_submenu {
                return false;
            }
            let submenu: *mut AnyObject = msg_send![item.as_object(), submenu];
            NonNull::new(submenu).is_some_and(|submenu| {
                let entries: isize = msg_send![submenu.as_ref(), numberOfItems];
                entries == 2
                    && menu_item_titled(submenu.as_ref(), "Editor").is_some()
                    && menu_item_titled(submenu.as_ref(), "Terminal").is_some()
            })
        });
        let favorite_pass = menu_item_titled(menu, "Favorite").is_some_and(|item| {
            let state: isize = msg_send![item.as_object(), state];
            state == 0
        });
        let delete_pass = menu_item_titled(menu, "Delete").is_some_and(|item| {
            let enabled: bool = msg_send![item.as_object(), isEnabled];
            enabled
        });
        let passed = count == 7
            && separators_pass
            && rename_pass
            && duplicate_pass
            && submenu_pass
            && favorite_pass
            && delete_pass;
        eprintln!(
            "Rinka context-menu probe step=structure items={count} separators={separators_pass} rename={rename_pass} duplicate={duplicate_pass} submenu={submenu_pass} favorite_unchecked={favorite_pass} delete={delete_pass} pass={passed}"
        );
        passed
    }
}

/// Opens the README.md row menu through one blocking interaction and reports
/// whether the native menu actually began tracking. A timer scheduled in the
/// event-tracking mode photographs and closes the menu, so the probe never
/// waits on user input.
unsafe fn drive_menu_open(
    mtm: MainThreadMarker,
    menu: &AnyObject,
    label: &str,
    open: impl FnOnce() -> bool,
) -> (bool, bool) {
    // SAFETY: All receivers are live AppKit objects on the main thread.
    unsafe {
        let observer = MenuTrackingObserver::new(mtm);
        *observer.ivars().capture_label.borrow_mut() = label.to_owned();
        let center: *mut AnyObject = msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
        let begin_name = ns_string("NSMenuDidBeginTrackingNotification");
        let _: () = msg_send![
            center,
            addObserver: &*observer,
            selector: sel!(menuDidBeginTracking:),
            name: begin_name.as_object(),
            object: menu
        ];
        let timer: *mut AnyObject = msg_send![
            objc2::class!(NSTimer),
            timerWithTimeInterval: 0.4_f64,
            target: &*observer,
            selector: sel!(cancelProbeMenu:),
            userInfo: menu,
            repeats: false
        ];
        let run_loop: *mut AnyObject = msg_send![objc2::class!(NSRunLoop), currentRunLoop];
        let tracking_mode = ns_string("NSEventTrackingRunLoopMode");
        let default_mode = ns_string("kCFRunLoopDefaultMode");
        let _: () = msg_send![run_loop, addTimer: timer, forMode: tracking_mode.as_object()];
        let _: () = msg_send![run_loop, addTimer: timer, forMode: default_mode.as_object()];

        let performed = open();
        let opened = observer.ivars().opened.get();
        let _: () = msg_send![center, removeObserver: &*observer];
        if !opened {
            let _: () = msg_send![timer, invalidate];
        }
        (performed, opened)
    }
}

/// Fabricates one pointer event at the center of the README.md row cell and
/// delivers it through the window's real event routing.
unsafe fn probe_pointer_menu(
    mtm: MainThreadMarker,
    table: &AnyObject,
    label: &str,
    event_type: usize,
    modifiers: usize,
) -> bool {
    // SAFETY: All receivers are live AppKit objects on the main thread; the
    // fabricated event carries this window's own number and never leaves the
    // process.
    unsafe {
        let Some(cell) = find_row_cell(table, "README.md") else {
            eprintln!("Rinka context-menu probe step={label} error=row-missing pass=false");
            return false;
        };
        let menu: *mut AnyObject = msg_send![cell.as_object(), menu];
        let Some(menu) = NonNull::new(menu) else {
            eprintln!("Rinka context-menu probe step={label} error=menu-missing pass=false");
            return false;
        };
        let (performed, opened) = drive_menu_open(mtm, menu.as_ref(), label, || {
            send_pointer_event(cell.as_object(), event_type, modifiers)
        });
        let passed = performed && opened;
        eprintln!(
            "Rinka context-menu probe step={label} performed={performed} opened={opened} pass={passed}"
        );
        passed
    }
}

unsafe fn send_pointer_event(cell: &AnyObject, event_type: usize, modifiers: usize) -> bool {
    // SAFETY: All receivers are live AppKit objects on the main thread.
    unsafe {
        let window: *mut AnyObject = msg_send![cell, window];
        let Some(window) = NonNull::new(window) else {
            return false;
        };
        let bounds: Rect = msg_send![cell, bounds];
        let center = Point {
            x: bounds.origin.x + bounds.size.width / 2.0,
            y: bounds.origin.y + bounds.size.height / 2.0,
        };
        let base: Point = msg_send![
            cell,
            convertPoint: center,
            toView: std::ptr::null::<AnyObject>()
        ];
        // Diagnostic: report which view the window would route this event to.
        let content: *mut AnyObject = msg_send![window.as_ref(), contentView];
        if let Some(content) = NonNull::new(content) {
            let frame_view: *mut AnyObject = msg_send![content.as_ref(), superview];
            if let Some(frame_view) = NonNull::new(frame_view) {
                let in_frame: Point = msg_send![
                    frame_view.as_ref(),
                    convertPoint: base,
                    fromView: std::ptr::null::<AnyObject>()
                ];
                let hit: *mut AnyObject = msg_send![frame_view.as_ref(), hitTest: in_frame];
                if let Some(hit) = NonNull::new(hit) {
                    let class: *mut AnyObject = msg_send![hit.as_ref(), className];
                    eprintln!(
                        "Rinka context-menu probe diagnostic hit_view={}",
                        rust_string(class)
                    );
                }
            }
        }
        let window_number: isize = msg_send![window.as_ref(), windowNumber];
        let process_info: *mut AnyObject = msg_send![objc2::class!(NSProcessInfo), processInfo];
        let uptime: f64 = msg_send![process_info, systemUptime];
        // A fabricated left-press has no physical release; queue the matching
        // release first so any control-tracking loop the press enters can
        // finish instead of waiting on a pointer forever.
        if event_type == EVENT_LEFT_MOUSE_DOWN {
            let release: *mut AnyObject = msg_send![
                objc2::class!(NSEvent),
                mouseEventWithType: EVENT_LEFT_MOUSE_UP,
                location: base,
                modifierFlags: modifiers,
                timestamp: uptime,
                windowNumber: window_number,
                context: std::ptr::null::<AnyObject>(),
                eventNumber: 0_isize,
                clickCount: 1_isize,
                pressure: 0.0_f32
            ];
            if let Some(release) = NonNull::new(release) {
                let application: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let _: () = msg_send![application, postEvent: release.as_ref(), atStart: false];
            }
        }
        let event: *mut AnyObject = msg_send![
            objc2::class!(NSEvent),
            mouseEventWithType: event_type,
            location: base,
            modifierFlags: modifiers,
            timestamp: uptime,
            windowNumber: window_number,
            context: std::ptr::null::<AnyObject>(),
            eventNumber: 0_isize,
            clickCount: 1_isize,
            pressure: 1.0_f32
        ];
        let Some(event) = NonNull::new(event) else {
            return false;
        };
        let application: *mut AnyObject =
            msg_send![objc2::class!(NSApplication), sharedApplication];
        let _: () = msg_send![application, sendEvent: event.as_ref()];
        true
    }
}

/// Opens the row menu through the accessibility show-menu action, proving the
/// pointer-free path served by the context-menu cell class.
unsafe fn probe_ax_show_menu(mtm: MainThreadMarker, table: &AnyObject) -> bool {
    // SAFETY: All receivers are live AppKit objects on the main thread.
    unsafe {
        let Some(cell) = find_row_cell(table, "README.md") else {
            eprintln!("Rinka context-menu probe step=ax-show-menu error=row-missing pass=false");
            return false;
        };
        let menu: *mut AnyObject = msg_send![cell.as_object(), menu];
        let Some(menu) = NonNull::new(menu) else {
            eprintln!("Rinka context-menu probe step=ax-show-menu error=menu-missing pass=false");
            return false;
        };
        let (performed, opened) = drive_menu_open(mtm, menu.as_ref(), "ax-show-menu", || {
            let performed: bool = msg_send![cell.as_object(), accessibilityPerformShowMenu];
            performed
        });
        let passed = performed && opened;
        eprintln!(
            "Rinka context-menu probe step=ax-show-menu performed={performed} opened={opened} pass={passed}"
        );
        passed
    }
}

/// Activates the destructive Delete item through its native target/action
/// pair and asserts the dispatched message reconciled the table and status
/// note.
unsafe fn probe_delete_activation(ivars: &ApplicationDelegateIvars, table: &AnyObject) -> bool {
    // SAFETY: All receivers are live AppKit objects on the main thread and
    // reconciliation runs synchronously inside the dispatched activation.
    unsafe {
        let rows_before: isize = msg_send![table, numberOfRows];
        let Some(cell) = find_row_cell(table, "README.md") else {
            eprintln!("Rinka context-menu probe step=activation error=row-missing pass=false");
            return false;
        };
        let menu: *mut AnyObject = msg_send![cell.as_object(), menu];
        let Some(menu) = NonNull::new(menu) else {
            eprintln!("Rinka context-menu probe step=activation error=menu-missing pass=false");
            return false;
        };
        let Some(delete) = menu_item_titled(menu.as_ref(), "Delete") else {
            eprintln!("Rinka context-menu probe step=activation error=item-missing pass=false");
            return false;
        };
        let target: *mut AnyObject = msg_send![delete.as_object(), target];
        let Some(target) = NonNull::new(target) else {
            eprintln!("Rinka context-menu probe step=activation error=target-missing pass=false");
            return false;
        };
        let _: () = msg_send![target.as_ref(), performAction: delete.as_object()];

        let rows_after: isize = msg_send![table, numberOfRows];
        let row_removed = find_row_cell(table, "README.md").is_none();
        let note_shown = {
            let windows = ivars.windows.borrow();
            windows.first().is_some_and(|window| {
                let content: *mut AnyObject = msg_send![window.as_object(), contentView];
                NonNull::new(content).is_some_and(|content| {
                    view_tree_contains_text(content.as_ref(), "Deleted README.md")
                })
            })
        };
        let passed = rows_after == rows_before - 1 && row_removed && note_shown;
        eprintln!(
            "Rinka context-menu probe step=activation rows_before={rows_before} rows_after={rows_after} row_removed={row_removed} note_shown={note_shown} pass={passed}"
        );
        passed
    }
}

/// Duplicates Cargo.toml through the menu and asserts the freshly realized
/// menu disables Duplicate while the native copy row appears.
unsafe fn probe_duplicate_enabled_reconcile(table: &AnyObject) -> bool {
    // SAFETY: All receivers are live AppKit objects on the main thread.
    unsafe {
        let activated = activate_row_menu_item(table, "Cargo.toml", "Duplicate");
        if !activated {
            eprintln!(
                "Rinka context-menu probe step=enabled-reconcile error=activation pass=false"
            );
            return false;
        }
        let copy_exists = find_row_cell(table, "Cargo.toml copy").is_some();
        let duplicate_disabled = find_row_cell(table, "Cargo.toml").is_some_and(|cell| {
            let menu: *mut AnyObject = msg_send![cell.as_object(), menu];
            NonNull::new(menu).is_some_and(|menu| {
                menu_item_titled(menu.as_ref(), "Duplicate").is_some_and(|item| {
                    let enabled: bool = msg_send![item.as_object(), isEnabled];
                    !enabled
                })
            })
        });
        let passed = copy_exists && duplicate_disabled;
        eprintln!(
            "Rinka context-menu probe step=enabled-reconcile copy_row={copy_exists} duplicate_disabled={duplicate_disabled} pass={passed}"
        );
        passed
    }
}

/// Toggles the Favorite item and asserts the freshly realized menu shows the
/// native checkmark.
unsafe fn probe_favorite_checkmark_reconcile(table: &AnyObject) -> bool {
    // SAFETY: All receivers are live AppKit objects on the main thread.
    unsafe {
        let activated = activate_row_menu_item(table, "Cargo.toml", "Favorite");
        if !activated {
            eprintln!(
                "Rinka context-menu probe step=checkmark-reconcile error=activation pass=false"
            );
            return false;
        }
        let checked = find_row_cell(table, "Cargo.toml").is_some_and(|cell| {
            let menu: *mut AnyObject = msg_send![cell.as_object(), menu];
            NonNull::new(menu).is_some_and(|menu| {
                menu_item_titled(menu.as_ref(), "Favorite").is_some_and(|item| {
                    let state: isize = msg_send![item.as_object(), state];
                    state == 1
                })
            })
        });
        eprintln!(
            "Rinka context-menu probe step=checkmark-reconcile checked={checked} pass={checked}"
        );
        checked
    }
}

unsafe fn activate_row_menu_item(table: &AnyObject, row_title: &str, item_title: &str) -> bool {
    // SAFETY: All receivers are live AppKit objects on the main thread.
    unsafe {
        let Some(cell) = find_row_cell(table, row_title) else {
            return false;
        };
        let menu: *mut AnyObject = msg_send![cell.as_object(), menu];
        let Some(menu) = NonNull::new(menu) else {
            return false;
        };
        let Some(item) = menu_item_titled(menu.as_ref(), item_title) else {
            return false;
        };
        let target: *mut AnyObject = msg_send![item.as_object(), target];
        let Some(target) = NonNull::new(target) else {
            return false;
        };
        let _: () = msg_send![target.as_ref(), performAction: item.as_object()];
        true
    }
}

/// Walks a native view tree for a text field showing exactly `text`.
unsafe fn view_tree_contains_text(view: &AnyObject, text: &str) -> bool {
    // SAFETY: The receiver is a live NSView on the main thread.
    unsafe {
        let is_text_field: bool = msg_send![view, isKindOfClass: objc2::class!(NSTextField)];
        if is_text_field {
            let value: *mut AnyObject = msg_send![view, stringValue];
            if rust_string(value) == text {
                return true;
            }
        }
        let subviews: *mut AnyObject = msg_send![view, subviews];
        let count: usize = msg_send![subviews, count];
        for index in 0..count {
            let child: *mut AnyObject = msg_send![subviews, objectAtIndex: index];
            if let Some(child) = NonNull::new(child)
                && view_tree_contains_text(child.as_ref(), text)
            {
                return true;
            }
        }
    }
    false
}

/// Names the capture directory of whichever menu probe is running.
fn probe_capture_directory() -> Option<String> {
    std::env::var("RINKA_APPKIT_CONTEXT_MENU_PROBE_CAPTURE_DIR")
        .or_else(|_| std::env::var("RINKA_APPKIT_MENU_BAR_PROBE_CAPTURE_DIR"))
        .ok()
}

/// Writes PNG captures of every window this process owns, labeled per step.
///
/// Self-capture through the window server does not require the
/// screen-recording grant, so the probe can photograph the open menu window
/// even in an unattended session.
unsafe fn capture_step_windows(label: &str) {
    let Some(directory) = probe_capture_directory() else {
        return;
    };
    // SAFETY: The window list and images describe only this process's own
    // windows; Foundation receivers are toll-free bridged CF objects owned by
    // this call.
    unsafe {
        for (index, (window_id, layer)) in own_window_ids().into_iter().enumerate() {
            let path = format!("{directory}/{label}-window{index}-layer{layer}.png");
            let written = capture_window_image(window_id, &path);
            eprintln!(
                "Rinka context-menu probe capture label={label} window={window_id} layer={layer} written={written}"
            );
        }
    }
}

/// Lists (window id, layer) for every on-screen window this process owns.
unsafe fn own_window_ids() -> Vec<(u32, i32)> {
    let process_id = i64::from(std::process::id());
    // SAFETY: The returned CFArray is owned by this call and released through
    // its toll-free NSArray identity when `info` drops.
    unsafe {
        let info = CGWindowListCopyWindowInfo(WINDOW_LIST_ON_SCREEN_EXCLUDING_DESKTOP, 0);
        let Some(info) = NonNull::new(info) else {
            return Vec::new();
        };
        let info = Id::from_owned(info.as_ptr());
        let count: usize = msg_send![info.as_object(), count];
        let mut windows = Vec::new();
        for index in 0..count {
            let entry: *mut AnyObject = msg_send![info.as_object(), objectAtIndex: index];
            let Some(entry) = NonNull::new(entry) else {
                continue;
            };
            let owner_key = ns_string("kCGWindowOwnerPID");
            let owner: *mut AnyObject =
                msg_send![entry.as_ref(), objectForKey: owner_key.as_object()];
            if owner.is_null() {
                continue;
            }
            let owner_pid: i64 = msg_send![owner, longLongValue];
            if owner_pid != process_id {
                continue;
            }
            let number_key = ns_string("kCGWindowNumber");
            let number: *mut AnyObject =
                msg_send![entry.as_ref(), objectForKey: number_key.as_object()];
            if number.is_null() {
                continue;
            }
            let number: i64 = msg_send![number, longLongValue];
            let layer_key = ns_string("kCGWindowLayer");
            let layer: *mut AnyObject =
                msg_send![entry.as_ref(), objectForKey: layer_key.as_object()];
            let layer: i64 = if layer.is_null() {
                0
            } else {
                msg_send![layer, longLongValue]
            };
            let Ok(number) = u32::try_from(number) else {
                continue;
            };
            let Ok(layer) = i32::try_from(layer) else {
                continue;
            };
            windows.push((number, layer));
        }
        windows
    }
}

/// Renders one own window into a PNG file; returns whether the file was
/// written.
unsafe fn capture_window_image(window_id: u32, path: &str) -> bool {
    // CGRectNull selects the window's own bounds.
    let null_rect = Rect {
        origin: Point {
            x: f64::INFINITY,
            y: f64::INFINITY,
        },
        size: Size {
            width: 0.0,
            height: 0.0,
        },
    };
    // SAFETY: The image is owned by this call and released after the bitmap
    // representation copies it; all Foundation receivers are live.
    unsafe {
        let image = CGWindowListCreateImage(null_rect, WINDOW_LIST_INCLUDING_WINDOW, window_id, 0);
        if image.is_null() {
            return false;
        }
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSBitmapImageRep), alloc];
        let representation: *mut AnyObject = msg_send![allocated, initWithCGImage: image];
        CFRelease(image.cast());
        let Some(representation) = NonNull::new(representation) else {
            return false;
        };
        let representation = Id::from_owned(representation.as_ptr());
        let properties: *mut AnyObject = msg_send![objc2::class!(NSDictionary), dictionary];
        // NSBitmapImageFileTypePNG is the stable public value 4.
        let data: *mut AnyObject = msg_send![
            representation.as_object(),
            representationUsingType: 4_usize,
            properties: properties
        ];
        let Some(data) = NonNull::new(data) else {
            return false;
        };
        let path = ns_string(path);
        let written: bool = msg_send![
            data.as_ref(),
            writeToFile: path.as_object(),
            atomically: true
        ];
        written
    }
}
