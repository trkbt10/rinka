// In-process diagnostic for the native drag-and-drop realization
// (`reports/drag-and-drop`).
//
// `RINKA_APPKIT_DRAG_DROP_PROBE=1` drives the real explorer after initial
// layout, entirely inside this process — no global input injection, no
// focus stealing, nothing that can land in another window:
//
// 1. OS file drop-in: a uniquely named pasteboard carries real temp-file
//    URLs, a constructed NSDraggingInfo double points at the file table,
//    and the retained table delegate serves validateDrop and acceptDrop
//    through their public selectors — protocol-level evidence for the drop
//    path AppKit drives during a live Finder drag.
// 2. File drag-out: the delegate's real pasteboard writer for the
//    README.md row must be the file-promise provider carrying the typed
//    payload beside the promise; its delegate materializes the promised
//    file into a temp directory only when asked, exactly once.
// 3. Intra-app drag: the payload written by that same provider lands on
//    the sidebar's Documents row through the sidebar delegate's
//    validateDrop/acceptDrop, and the move intent reconciles the note.
//
// `RINKA_APPKIT_DRAG_DROP_PROBE_AX_DUMP=<path>` additionally writes a
// deterministic accessibility dump (class, role, label per view, sorted)
// so two runs — with and without `RINKA_EXPLORER_DISABLE_DRAG` — prove the
// AX tree is equivalent before and after attaching drag declarations.
// Every step prints one `Rinka drag-drop probe` line and the process
// terminates after the summary line.

impl ApplicationDelegate {
    fn begin_drag_drop_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_DRAG_DROP_PROBE").is_none() {
            return;
        }
        if std::env::var_os("RINKA_APPKIT_SCENE_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_ACCELERATOR_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_CONTEXT_MENU_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_CLIPBOARD_PROBE").is_some()
        {
            panic!("the drag-drop probe must run in its own process");
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
            eprintln!("Rinka drag-drop probe step=locate-window pass=false");
            eprintln!("Rinka drag-drop probe result=FAIL");
            terminate_drag_drop_probe();
            return;
        };

        // SAFETY: Every step reads and drives retained AppKit objects on the
        // main thread through their public selectors.
        let passed = unsafe {
            let dumped = dump_probe_ax_tree(content.as_ref());
            let files_table =
                find_probe_outline(content.as_ref(), &|label| label.starts_with("Files in"));
            if std::env::var_os("RINKA_EXPLORER_DISABLE_DRAG").is_some() {
                // The drag-free run exists only to produce the AX dump for
                // the equivalence comparison.
                eprintln!("Rinka drag-drop probe step=ax-dump-without-drag pass={dumped}");
                dumped
            } else if files_table.is_none() {
                // No file table means the Empty scene: the enclosing column
                // is the container drop host that keeps the status copy's
                // "drop files here" promise.
                let drop_in = probe_empty_scene_drop_in(self.ivars(), content.as_ref());
                flush_window_rendering(self.ivars());
                self.capture_windows_to_directory("drag-empty-");
                drop_in
            } else {
                let drop_in = probe_file_drop_in(self.ivars(), content.as_ref());
                flush_window_rendering(self.ivars());
                let (drag_out, payload_encoded) =
                    probe_file_drag_out(self.ivars(), content.as_ref());
                flush_window_rendering(self.ivars());
                let intra_app =
                    probe_intra_app_drag(self.ivars(), content.as_ref(), payload_encoded);
                flush_window_rendering(self.ivars());
                self.capture_windows_to_directory("drag-");
                eprintln!("Rinka drag-drop probe step=ax-dump pass={dumped}");
                drop_in && drag_out && intra_app && dumped
            }
        };
        eprintln!(
            "Rinka drag-drop probe result={}",
            if passed { "PASS" } else { "FAIL" }
        );
        terminate_drag_drop_probe();
    }
}

fn terminate_drag_drop_probe() {
    if std::env::var_os("RINKA_APPKIT_DRAG_DROP_PROBE_HOLD").is_some() {
        return;
    }
    // SAFETY: Diagnostic completion terminates only the current test app.
    unsafe {
        let application: *mut AnyObject =
            msg_send![objc2::class!(NSApplication), sharedApplication];
        let _: () = msg_send![application, terminate: std::ptr::null::<AnyObject>()];
    }
}

/// Constructed NSDraggingInfo double: exactly the session state the Rinka
/// drop path reads (pasteboard and pointer location), nothing more.
struct DragInfoDoubleIvars {
    pasteboard: Id,
    location: Cell<Point>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DragInfoDoubleIvars]
    struct DragInfoDouble;

    // SAFETY: NSObjectProtocol adds no invariants beyond NSObject.
    unsafe impl NSObjectProtocol for DragInfoDouble {}

    impl DragInfoDouble {
        #[unsafe(method(draggingPasteboard))]
        fn dragging_pasteboard(&self) -> *mut AnyObject {
            self.ivars().pasteboard.as_ptr()
        }

        #[unsafe(method(draggingLocation))]
        fn dragging_location(&self) -> Point {
            self.ivars().location.get()
        }

        #[unsafe(method(draggingSource))]
        fn dragging_source(&self) -> *mut AnyObject {
            std::ptr::null_mut()
        }

        #[unsafe(method(draggingSourceOperationMask))]
        fn dragging_source_operation_mask(&self) -> usize {
            // NSDragOperationEvery: the double never restricts operations.
            usize::MAX
        }

        #[unsafe(method(draggingSequenceNumber))]
        fn dragging_sequence_number(&self) -> isize {
            0
        }
    }
);

impl DragInfoDouble {
    fn new(mtm: MainThreadMarker, pasteboard: Id, location: Point) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(DragInfoDoubleIvars {
            pasteboard,
            location: Cell::new(location),
        });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }
}

/// Creates a uniquely named pasteboard so the probe never touches the
/// user's general pasteboard; the caller must release it globally.
unsafe fn probe_pasteboard(label: &str) -> Id {
    let name = ns_string(&format!(
        "jp.bunko.rinka.drag-drop-probe.{label}.{}",
        std::process::id()
    ));
    // SAFETY: pasteboardWithName: returns a non-owning pasteboard reference;
    // the wrapper balances its own retain.
    unsafe {
        let pointer: *mut AnyObject = msg_send![
            objc2::class!(NSPasteboard),
            pasteboardWithName: name.as_object()
        ];
        let pasteboard = Id::from_borrowed(pointer);
        let _: isize = msg_send![pasteboard.as_object(), clearContents];
        pasteboard
    }
}

unsafe fn release_probe_pasteboard(pasteboard: &Id) {
    // SAFETY: releaseGlobally removes the named pasteboard from the server;
    // the local retain held by Id stays balanced.
    unsafe {
        let _: () = msg_send![pasteboard.as_object(), releaseGlobally];
    }
}

/// Finds the NSOutlineView whose accessibility label satisfies `matches`.
unsafe fn find_probe_outline(
    view: &AnyObject,
    matches: &dyn Fn(&str) -> bool,
) -> Option<Id> {
    // SAFETY: The receiver is a live NSView on the main thread.
    unsafe {
        let is_outline: bool = msg_send![view, isKindOfClass: objc2::class!(NSOutlineView)];
        if is_outline {
            let label: *mut AnyObject = msg_send![view, accessibilityLabel];
            if matches(&rust_string(label)) {
                return Some(Id::from_borrowed(
                    (view as *const AnyObject).cast_mut(),
                ));
            }
        }
        let subviews: *mut AnyObject = msg_send![view, subviews];
        let count: usize = msg_send![subviews, count];
        for index in 0..count {
            let child: *mut AnyObject = msg_send![subviews, objectAtIndex: index];
            if let Some(child) = NonNull::new(child)
                && let Some(found) = find_probe_outline(child.as_ref(), matches)
            {
                return Some(found);
            }
        }
    }
    None
}

/// Returns the outline item whose primary-column title matches exactly.
unsafe fn find_probe_outline_item(outline: &AnyObject, title: &str) -> Option<Id> {
    // SAFETY: The receiver is a live NSOutlineView; makeIfNecessary builds
    // the same cells the delegate serves during display.
    unsafe {
        let rows: isize = msg_send![outline, numberOfRows];
        for row in 0..rows {
            let cell: *mut AnyObject = msg_send![
                outline,
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
                let item: *mut AnyObject = msg_send![outline, itemAtRow: row];
                return NonNull::new(item).map(|item| Id::from_borrowed(item.as_ptr()));
            }
        }
    }
    None
}

/// Returns the window-coordinate center of a view, for the info double.
unsafe fn probe_view_center_in_window(view: &AnyObject) -> Point {
    // SAFETY: The receiver is a live view attached to a window.
    unsafe {
        let bounds: Rect = msg_send![view, bounds];
        let center = Point {
            x: bounds.origin.x + bounds.size.width / 2.0,
            y: bounds.origin.y + bounds.size.height / 2.0,
        };
        msg_send![view, convertPoint: center, toView: std::ptr::null::<AnyObject>()]
    }
}

/// Finds the text of the first text field whose value starts with `prefix`.
unsafe fn view_tree_text_with_prefix(view: &AnyObject, prefix: &str) -> Option<String> {
    // SAFETY: The receiver is a live NSView on the main thread.
    unsafe {
        let is_text_field: bool = msg_send![view, isKindOfClass: objc2::class!(NSTextField)];
        if is_text_field {
            let value: *mut AnyObject = msg_send![view, stringValue];
            let text = rust_string(value);
            if text.starts_with(prefix) {
                return Some(text);
            }
        }
        let subviews: *mut AnyObject = msg_send![view, subviews];
        let count: usize = msg_send![subviews, count];
        for index in 0..count {
            let child: *mut AnyObject = msg_send![subviews, objectAtIndex: index];
            if let Some(child) = NonNull::new(child)
                && let Some(found) = view_tree_text_with_prefix(child.as_ref(), prefix)
            {
                return Some(found);
            }
        }
    }
    None
}

/// Reads the explorer's drag note from the first window.
unsafe fn probe_drag_note(ivars: &ApplicationDelegateIvars, prefix: &str) -> Option<String> {
    let windows = ivars.windows.borrow();
    let window = windows.first()?;
    // SAFETY: The retained window's content view is read on the main thread.
    unsafe {
        let content: *mut AnyObject = msg_send![window.as_object(), contentView];
        let content = NonNull::new(content)?;
        view_tree_text_with_prefix(content.as_ref(), prefix)
    }
}

/// Layer 1: real temp-file URLs on a named pasteboard, dropped onto the
/// file table through the delegate's validateDrop/acceptDrop selectors.
unsafe fn probe_file_drop_in(ivars: &ApplicationDelegateIvars, content: &AnyObject) -> bool {
    // SAFETY: Every receiver is a live retained AppKit object on the main
    // thread; the dropped URLs point at files this probe created.
    unsafe {
        let Some(table) = find_probe_outline(content, &|label| label.starts_with("Files in"))
        else {
            eprintln!("Rinka drag-drop probe step=drop-in error=table-missing pass=false");
            return false;
        };
        let mtm = MainThreadMarker::new().expect("probe runs on the main thread");

        let directory =
            std::env::temp_dir().join(format!("rinka-drag-drop-probe-{}", std::process::id()));
        if std::fs::create_dir_all(&directory).is_err() {
            eprintln!("Rinka drag-drop probe step=drop-in error=temp-dir pass=false");
            return false;
        }
        let first = directory.join("alpha.txt");
        let second = directory.join("beta.md");
        if std::fs::write(&first, "alpha").is_err() || std::fs::write(&second, "beta").is_err() {
            eprintln!("Rinka drag-drop probe step=drop-in error=temp-files pass=false");
            return false;
        }

        let pasteboard = probe_pasteboard("files");
        let urls = [&first, &second]
            .iter()
            .map(|path| {
                let path = ns_string(&path.display().to_string());
                let pointer: *mut AnyObject = msg_send![
                    objc2::class!(NSURL),
                    fileURLWithPath: path.as_object()
                ];
                Id::from_borrowed(pointer)
            })
            .collect::<Vec<_>>();
        let url_array = ns_array(&urls);
        let written: bool =
            msg_send![pasteboard.as_object(), writeObjects: url_array.as_object()];

        let location = probe_view_center_in_window(table.as_object());
        let info = DragInfoDouble::new(mtm, pasteboard.clone(), location);
        let source: *mut AnyObject = msg_send![table.as_object(), dataSource];
        let Some(source) = NonNull::new(source) else {
            eprintln!("Rinka drag-drop probe step=drop-in error=data-source pass=false");
            return false;
        };

        let operation: usize = msg_send![source.as_ref(),
            outlineView: table.as_object(),
            validateDrop: &*info,
            proposedItem: std::ptr::null::<AnyObject>(),
            proposedChildIndex: -1_isize
        ];
        let accepted: bool = msg_send![source.as_ref(),
            outlineView: table.as_object(),
            acceptDrop: &*info,
            item: std::ptr::null::<AnyObject>(),
            childIndex: -1_isize
        ];
        release_probe_pasteboard(&pasteboard);
        let _ = std::fs::remove_dir_all(&directory);

        let note = probe_drag_note(ivars, "Dropped 2 file(s)").unwrap_or_default();
        let note_pass = note.contains("alpha.txt") && note.contains("beta.md");
        let passed =
            written && operation == DRAG_OPERATION_COPY && accepted && note_pass;
        eprintln!(
            "Rinka drag-drop probe step=drop-in written={written} operation={operation} accepted={accepted} note={note:?} pass={passed}"
        );
        passed
    }
}

/// Finds the deepest view registered for dragged file URLs that is not a
/// table — the container drop host serving NSDraggingDestination.
unsafe fn find_registered_drop_host(view: &AnyObject) -> Option<Id> {
    // SAFETY: The receiver is a live NSView on the main thread.
    unsafe {
        let subviews: *mut AnyObject = msg_send![view, subviews];
        let count: usize = msg_send![subviews, count];
        for index in 0..count {
            let child: *mut AnyObject = msg_send![subviews, objectAtIndex: index];
            if let Some(child) = NonNull::new(child)
                && let Some(found) = find_registered_drop_host(child.as_ref())
            {
                return Some(found);
            }
        }
        let is_table: bool = msg_send![view, isKindOfClass: objc2::class!(NSTableView)];
        if is_table {
            return None;
        }
        let registered: *mut AnyObject = msg_send![view, registeredDraggedTypes];
        let registered_count: usize = if registered.is_null() {
            0
        } else {
            msg_send![registered, count]
        };
        if registered_count > 0 {
            return Some(Id::from_borrowed((view as *const AnyObject).cast_mut()));
        }
    }
    None
}

/// Layer 1 on the Empty scene: the "drop files here" column serves
/// NSDraggingDestination directly through its own protocol methods.
unsafe fn probe_empty_scene_drop_in(
    ivars: &ApplicationDelegateIvars,
    content: &AnyObject,
) -> bool {
    // SAFETY: Every receiver is a live retained AppKit object on the main
    // thread; the dropped URL points at a file this probe created.
    unsafe {
        let Some(host) = find_registered_drop_host(content) else {
            eprintln!("Rinka drag-drop probe step=empty-drop-in error=host-missing pass=false");
            return false;
        };
        let mtm = MainThreadMarker::new().expect("probe runs on the main thread");

        let directory = std::env::temp_dir().join(format!(
            "rinka-drag-drop-probe-empty-{}",
            std::process::id()
        ));
        if std::fs::create_dir_all(&directory).is_err() {
            eprintln!("Rinka drag-drop probe step=empty-drop-in error=temp-dir pass=false");
            return false;
        }
        let file = directory.join("gamma.txt");
        if std::fs::write(&file, "gamma").is_err() {
            eprintln!("Rinka drag-drop probe step=empty-drop-in error=temp-file pass=false");
            return false;
        }

        let pasteboard = probe_pasteboard("empty-files");
        let url: *mut AnyObject = msg_send![
            objc2::class!(NSURL),
            fileURLWithPath: ns_string(&file.display().to_string()).as_object()
        ];
        let url_array = ns_array(&[Id::from_borrowed(url)]);
        let written: bool =
            msg_send![pasteboard.as_object(), writeObjects: url_array.as_object()];

        let location = probe_view_center_in_window(host.as_object());
        let info = DragInfoDouble::new(mtm, pasteboard.clone(), location);
        let operation: usize = msg_send![host.as_object(), draggingEntered: &*info];
        let prepared: bool = msg_send![host.as_object(), prepareForDragOperation: &*info];
        let performed: bool = msg_send![host.as_object(), performDragOperation: &*info];
        release_probe_pasteboard(&pasteboard);
        let _ = std::fs::remove_dir_all(&directory);

        let note = probe_drag_note(ivars, "Dropped 1 file(s)").unwrap_or_default();
        let note_pass = note.contains("gamma.txt");
        let passed = written
            && operation == DRAG_OPERATION_COPY
            && prepared
            && performed
            && note_pass;
        eprintln!(
            "Rinka drag-drop probe step=empty-drop-in written={written} operation={operation} prepared={prepared} performed={performed} note={note:?} pass={passed}"
        );
        passed
    }
}

/// Layer 2: the README.md row's real pasteboard writer materializes its
/// promised file lazily. Returns the intra-app payload the same writer
/// carries, for the next step.
unsafe fn probe_file_drag_out(
    ivars: &ApplicationDelegateIvars,
    content: &AnyObject,
) -> (bool, Option<String>) {
    // SAFETY: Every receiver is a live retained AppKit object on the main
    // thread; the promise writes into a directory this probe created.
    unsafe {
        let Some(table) = find_probe_outline(content, &|label| label.starts_with("Files in"))
        else {
            eprintln!("Rinka drag-drop probe step=drag-out error=table-missing pass=false");
            return (false, None);
        };
        let Some(item) = find_probe_outline_item(table.as_object(), "README.md") else {
            eprintln!("Rinka drag-drop probe step=drag-out error=row-missing pass=false");
            return (false, None);
        };
        let source: *mut AnyObject = msg_send![table.as_object(), dataSource];
        let Some(source) = NonNull::new(source) else {
            eprintln!("Rinka drag-drop probe step=drag-out error=data-source pass=false");
            return (false, None);
        };
        let writer: *mut AnyObject = msg_send![source.as_ref(),
            outlineView: table.as_object(),
            pasteboardWriterForItem: item.as_object()
        ];
        let Some(writer) = NonNull::new(writer) else {
            eprintln!("Rinka drag-drop probe step=drag-out error=writer-missing pass=false");
            return (false, None);
        };
        let is_promise: bool = msg_send![
            writer.as_ref(),
            isKindOfClass: objc2::class!(NSFilePromiseProvider)
        ];
        let file_type: *mut AnyObject = msg_send![writer.as_ref(), fileType];
        let file_type = rust_string(file_type);

        // The same writer must carry the intra-app payload beside the
        // promise: one session serves both representations.
        let payload_value: *mut AnyObject = msg_send![
            writer.as_ref(),
            pasteboardPropertyListForType: payload_pasteboard_type().as_object()
        ];
        let payload_encoded = NonNull::new(payload_value).map(|value| rust_string(value.as_ptr()));

        let delegate: *mut AnyObject = msg_send![writer.as_ref(), delegate];
        let Some(delegate) = NonNull::new(delegate) else {
            eprintln!("Rinka drag-drop probe step=drag-out error=delegate-missing pass=false");
            return (false, None);
        };
        let name: *mut AnyObject = msg_send![delegate.as_ref(),
            filePromiseProvider: writer.as_ref(),
            fileNameForType: ns_string(&file_type).as_object()
        ];
        let promised_name = rust_string(name);

        let queue: *mut AnyObject =
            msg_send![delegate.as_ref(), operationQueueForFilePromiseProvider: writer.as_ref()];
        let main_queue: *mut AnyObject =
            msg_send![objc2::class!(NSOperationQueue), mainQueue];
        let main_queue_pass = std::ptr::eq(queue, main_queue);

        let directory = std::env::temp_dir().join(format!(
            "rinka-drag-drop-probe-export-{}",
            std::process::id()
        ));
        if std::fs::create_dir_all(&directory).is_err() {
            eprintln!("Rinka drag-drop probe step=drag-out error=temp-dir pass=false");
            return (false, payload_encoded);
        }
        let destination = directory.join(&promised_name);
        // Nothing materializes before the destination accepts the drop.
        let lazy_pass = !destination.exists();

        let completion_error = Rc::new(RefCell::new(None::<String>));
        let completion_ran = Rc::new(Cell::new(false));
        let error_sink = completion_error.clone();
        let ran_sink = completion_ran.clone();
        let completion = block2::RcBlock::new(move |error: *mut AnyObject| {
            ran_sink.set(true);
            if let Some(error) = NonNull::new(error) {
                let description: *mut AnyObject =
                    msg_send![error.as_ref(), localizedDescription];
                *error_sink.borrow_mut() = Some(rust_string(description));
            }
        });
        let destination_url: *mut AnyObject = msg_send![
            objc2::class!(NSURL),
            fileURLWithPath: ns_string(&destination.display().to_string()).as_object()
        ];
        let _: () = msg_send![delegate.as_ref(),
            filePromiseProvider: writer.as_ref(),
            writePromiseToURL: destination_url,
            completionHandler: &*completion
        ];

        let exported = std::fs::read_to_string(&destination).unwrap_or_default();
        let content_pass = exported.contains("file: README.md");
        let note = probe_drag_note(ivars, "Exported ").unwrap_or_default();
        let note_pass = note == format!("Exported {promised_name}");
        let _ = std::fs::remove_dir_all(&directory);

        let passed = is_promise
            && file_type == "public.plain-text"
            && promised_name == "README.md.txt"
            && main_queue_pass
            && lazy_pass
            && completion_ran.get()
            && completion_error.borrow().is_none()
            && content_pass
            && note_pass;
        eprintln!(
            "Rinka drag-drop probe step=drag-out promise={is_promise} file_type={file_type} name={promised_name} main_queue={main_queue_pass} lazy={lazy_pass} completion={} error={:?} content={content_pass} note={note:?} pass={passed}",
            completion_ran.get(),
            completion_error.borrow(),
        );
        (passed, payload_encoded)
    }
}

/// Layer 3: the payload carried by the file row's writer lands on the
/// sidebar's Documents row through the sidebar delegate's drop selectors.
unsafe fn probe_intra_app_drag(
    ivars: &ApplicationDelegateIvars,
    content: &AnyObject,
    payload_encoded: Option<String>,
) -> bool {
    // SAFETY: Every receiver is a live retained AppKit object on the main
    // thread; the payload pasteboard is uniquely named.
    unsafe {
        let Some(encoded) = payload_encoded else {
            eprintln!("Rinka drag-drop probe step=intra-app error=payload-missing pass=false");
            return false;
        };
        let payload_pass = decode_drag_payload(&encoded).is_some_and(|payload| {
            payload.payload_type() == "jp.bunko.rinka.explorer.file"
                && payload.id() == "README.md"
        });
        let Some(sidebar) = find_probe_outline(content, &|label| label == "Locations") else {
            eprintln!("Rinka drag-drop probe step=intra-app error=sidebar-missing pass=false");
            return false;
        };
        let Some(item) = find_probe_outline_item(sidebar.as_object(), "Documents") else {
            eprintln!("Rinka drag-drop probe step=intra-app error=item-missing pass=false");
            return false;
        };
        let mtm = MainThreadMarker::new().expect("probe runs on the main thread");

        let pasteboard = probe_pasteboard("payload");
        let stored: bool = msg_send![pasteboard.as_object(),
            setString: ns_string(&encoded).as_object(),
            forType: payload_pasteboard_type().as_object()
        ];
        let location = probe_view_center_in_window(sidebar.as_object());
        let info = DragInfoDouble::new(mtm, pasteboard.clone(), location);
        let source: *mut AnyObject = msg_send![sidebar.as_object(), dataSource];
        let Some(source) = NonNull::new(source) else {
            eprintln!("Rinka drag-drop probe step=intra-app error=data-source pass=false");
            return false;
        };

        let operation: usize = msg_send![source.as_ref(),
            outlineView: sidebar.as_object(),
            validateDrop: &*info,
            proposedItem: item.as_object(),
            proposedChildIndex: -1_isize
        ];
        let accepted: bool = msg_send![source.as_ref(),
            outlineView: sidebar.as_object(),
            acceptDrop: &*info,
            item: item.as_object(),
            childIndex: -1_isize
        ];
        release_probe_pasteboard(&pasteboard);

        let note = probe_drag_note(ivars, "Moved ").unwrap_or_default();
        let note_pass = note == "Moved README.md to Documents";
        let passed = payload_pass
            && stored
            && operation == DRAG_OPERATION_MOVE
            && accepted
            && note_pass;
        eprintln!(
            "Rinka drag-drop probe step=intra-app payload={payload_pass} stored={stored} operation={operation} accepted={accepted} note={note:?} pass={passed}"
        );
        passed
    }
}

/// Writes the deterministic accessibility dump when the dump path is set.
///
/// Two runs — with and without `RINKA_EXPLORER_DISABLE_DRAG` — must produce
/// identical dumps: attaching drag declarations may not change the class,
/// accessibility role, or accessibility label of any view.
unsafe fn dump_probe_ax_tree(content: &AnyObject) -> bool {
    let Ok(path) = std::env::var("RINKA_APPKIT_DRAG_DROP_PROBE_AX_DUMP") else {
        return true;
    };
    let mut lines = Vec::new();
    // SAFETY: The walk reads live views on the main thread.
    unsafe {
        collect_ax_lines(content, &mut lines);
    }
    lines.sort();
    std::fs::write(&path, lines.join("\n") + "\n").is_ok()
}

unsafe fn collect_ax_lines(view: &AnyObject, lines: &mut Vec<String>) {
    // SAFETY: The receiver is a live NSView on the main thread.
    unsafe {
        let class: *mut AnyObject = msg_send![view, className];
        let role: *mut AnyObject = msg_send![view, accessibilityRole];
        let label: *mut AnyObject = msg_send![view, accessibilityLabel];
        let is_element: bool = msg_send![view, isAccessibilityElement];
        lines.push(format!(
            "{}|{}|{}|{}",
            rust_string(class),
            rust_string(role),
            rust_string(label),
            is_element,
        ));
        let subviews: *mut AnyObject = msg_send![view, subviews];
        let count: usize = msg_send![subviews, count];
        for index in 0..count {
            let child: *mut AnyObject = msg_send![subviews, objectAtIndex: index];
            if let Some(child) = NonNull::new(child) {
                collect_ax_lines(child.as_ref(), lines);
            }
        }
    }
}
