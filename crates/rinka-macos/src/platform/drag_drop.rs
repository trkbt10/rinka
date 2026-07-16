// Native drag-and-drop realization (`reports/drag-and-drop`).
//
// Three layers share this file:
//
// 1. OS file drop-in: drop-hosting views (stacks, canvases) implement
//    NSDraggingDestination, and native tables serve drops through the
//    NSTableView/NSOutlineView validateDrop/acceptDrop machinery, which also
//    owns the native drop highlight.
// 2. File drag-out: rows with a file promise produce an
//    NSFilePromiseProvider whose delegate materializes the content lazily —
//    on the main operation queue, because the promise's write callback is a
//    main-thread Rust value — only when the destination accepts.
// 3. Intra-app item drag: the typed payload is pure data (type + id), so it
//    rides the pasteboard under one private type; both ends decode it
//    without an in-process session registry.
//
// Views register for dragged types only while their element declares a drop
// target, so an undeclared surface never swallows a session hit-tested
// through it. Acceptance decisions always read the stable event binding,
// whose models are current at interaction time.

unsafe extern "C" {
    #[link_name = "NSPasteboardTypeFileURL"]
    static PASTEBOARD_TYPE_FILE_URL: *mut AnyObject;
    #[link_name = "NSPasteboardURLReadingFileURLsOnlyKey"]
    static PASTEBOARD_URL_READING_FILE_URLS_ONLY_KEY: *mut AnyObject;
    #[link_name = "NSLocalizedDescriptionKey"]
    static LOCALIZED_DESCRIPTION_KEY: *mut AnyObject;
}

/// Private pasteboard type carrying one encoded intra-application payload.
const RINKA_PAYLOAD_PASTEBOARD_TYPE: &str = "jp.bunko.rinka.drag-payload";

/// `NSDragOperationNone`.
const DRAG_OPERATION_NONE: usize = 0;
/// `NSDragOperationCopy`.
const DRAG_OPERATION_COPY: usize = 1;
/// `NSDragOperationMove`.
const DRAG_OPERATION_MOVE: usize = 16;
/// `NSTableViewDropOn`.
const TABLE_DROP_ON: usize = 1;
/// `NSOutlineViewDropOnItemIndex`.
const OUTLINE_DROP_ON_ITEM_INDEX: isize = -1;

fn payload_pasteboard_type() -> Id {
    ns_string(RINKA_PAYLOAD_PASTEBOARD_TYPE)
}

/// Encodes a payload for pasteboard transport. The first line is the payload
/// type (validation forbids a line break inside it); everything after the
/// first line break is the identity, which may contain any text.
fn encode_drag_payload(payload: &DragPayload) -> String {
    format!("{}\n{}", payload.payload_type(), payload.id())
}

fn decode_drag_payload(encoded: &str) -> Option<DragPayload> {
    encoded
        .split_once('\n')
        .map(|(payload_type, id)| DragPayload::new(payload_type, id))
}

unsafe fn dragging_pasteboard(info: *mut AnyObject) -> Option<NonNull<AnyObject>> {
    let info = NonNull::new(info)?;
    // SAFETY: The argument is a live NSDraggingInfo delivered on the main
    // thread; its pasteboard is valid for the duration of the callback.
    let pasteboard: *mut AnyObject = unsafe { msg_send![info.as_ref(), draggingPasteboard] };
    NonNull::new(pasteboard)
}

unsafe fn pasteboard_drag_payload(pasteboard: &AnyObject) -> Option<DragPayload> {
    // SAFETY: stringForType: returns an autoreleased NSString or nil on a
    // live pasteboard; the bytes are copied before the pool drains.
    let value: *mut AnyObject = unsafe {
        msg_send![pasteboard, stringForType: payload_pasteboard_type().as_object()]
    };
    NonNull::new(value).and_then(|value| decode_drag_payload(&rust_string(value.as_ptr())))
}

unsafe fn pasteboard_has_file_urls(pasteboard: &AnyObject) -> bool {
    // SAFETY: The pasteboard type constant is a live static NSString and the
    // receiver is a live pasteboard.
    unsafe {
        let types = ns_array(&[Id::from_borrowed(PASTEBOARD_TYPE_FILE_URL)]);
        let available: *mut AnyObject =
            msg_send![pasteboard, availableTypeFromArray: types.as_object()];
        !available.is_null()
    }
}

unsafe fn pasteboard_file_paths(pasteboard: &AnyObject) -> Vec<std::path::PathBuf> {
    // SAFETY: readObjectsForClasses:options: returns an autoreleased array of
    // NSURL limited to file URLs; each path string is copied into owned Rust
    // memory before the pool drains. Class objects are valid Objective-C
    // objects, and retaining them is a no-op.
    unsafe {
        let url_class: *mut AnyObject =
            (objc2::class!(NSURL) as *const objc2::runtime::AnyClass as *mut AnyObject).cast();
        let classes = ns_array(&[Id::from_borrowed(url_class)]);
        let file_urls_only: *mut AnyObject =
            msg_send![objc2::class!(NSNumber), numberWithBool: true];
        let options: *mut AnyObject = msg_send![objc2::class!(NSDictionary),
            dictionaryWithObject: file_urls_only,
            forKey: PASTEBOARD_URL_READING_FILE_URLS_ONLY_KEY
        ];
        let urls: *mut AnyObject = msg_send![pasteboard,
            readObjectsForClasses: classes.as_object(),
            options: options
        ];
        let Some(urls) = NonNull::new(urls) else {
            return Vec::new();
        };
        let count: usize = msg_send![urls.as_ref(), count];
        let mut paths = Vec::with_capacity(count);
        for index in 0..count {
            let url: *mut AnyObject = msg_send![urls.as_ref(), objectAtIndex: index];
            if url.is_null() {
                continue;
            }
            let path: *mut AnyObject = msg_send![url, path];
            if !path.is_null() {
                paths.push(std::path::PathBuf::from(rust_string(path)));
            }
        }
        paths
    }
}

/// Converts the session's pointer location into the view's local coordinate
/// space with a top-left origin, which is the element-local contract.
unsafe fn dragging_local_position(view: &AnyObject, info: *mut AnyObject) -> DropPosition {
    let Some(info) = NonNull::new(info) else {
        return DropPosition::default();
    };
    // SAFETY: The info and view are live main-thread objects; conversion
    // uses the view's own geometry.
    unsafe {
        let location: Point = msg_send![info.as_ref(), draggingLocation];
        let local: Point = msg_send![
            view,
            convertPoint: location,
            fromView: std::ptr::null::<AnyObject>()
        ];
        let flipped: bool = msg_send![view, isFlipped];
        let bounds: Rect = msg_send![view, bounds];
        let y = if flipped {
            local.y - bounds.origin.y
        } else {
            bounds.origin.y + bounds.size.height - local.y
        };
        DropPosition::new(local.x - bounds.origin.x, y)
    }
}

/// Registers or unregisters a view for the dragged types Rinka serves.
///
/// Registration follows the declarative drop-target model exactly: an
/// element without a target stays unregistered so AppKit hit-tests drag
/// sessions through it to the nearest declared ancestor.
fn set_view_drag_registration(view: &AnyObject, active: bool) {
    // SAFETY: Both selectors are public NSView API on a live main-thread view.
    unsafe {
        if active {
            let types = ns_array(&[
                Id::from_borrowed(PASTEBOARD_TYPE_FILE_URL),
                payload_pasteboard_type(),
            ]);
            let _: () = msg_send![view, registerForDraggedTypes: types.as_object()];
        } else {
            let _: () = msg_send![view, unregisterDraggedTypes];
        }
    }
}

/// Returns the drag operation one hovering session may perform over an
/// element with the given stable binding.
fn drop_session_operation(events: &EventBindings, info: *mut AnyObject) -> usize {
    let Some(target) = events.drop_target() else {
        return DRAG_OPERATION_NONE;
    };
    // SAFETY: The dragging info is live for the duration of the callback.
    unsafe {
        let Some(pasteboard) = dragging_pasteboard(info) else {
            return DRAG_OPERATION_NONE;
        };
        if let Some(payload) = pasteboard_drag_payload(pasteboard.as_ref())
            && target.accepts_payload_type(payload.payload_type())
        {
            return DRAG_OPERATION_MOVE;
        }
        if target.accepts_files() && pasteboard_has_file_urls(pasteboard.as_ref()) {
            return DRAG_OPERATION_COPY;
        }
    }
    DRAG_OPERATION_NONE
}

/// Delivers a completed drop to an element through its stable binding, with
/// the drop position in the view's local top-left coordinates.
///
/// The binding enforces acceptance again, so a model that changed since the
/// session's validation phase refuses instead of misdelivering. Delivery may
/// re-render synchronously; the caller must hold no adapter borrows.
fn deliver_view_drop(view: &AnyObject, events: &EventBindings, info: *mut AnyObject) -> bool {
    // SAFETY: The dragging info is live for the duration of the callback.
    let (payload, paths, position) = unsafe {
        let Some(pasteboard) = dragging_pasteboard(info) else {
            return false;
        };
        (
            pasteboard_drag_payload(pasteboard.as_ref()),
            pasteboard_file_paths(pasteboard.as_ref()),
            dragging_local_position(view, info),
        )
    };
    if let Some(payload) = payload {
        return events.emit_payload_drop(PayloadDrop { payload, position });
    }
    if !paths.is_empty() {
        return events.emit_file_drop(FileDrop { paths, position });
    }
    false
}

struct DropHostViewIvars {
    events: EventBindings,
}

define_class!(
    /// Container backing view that serves NSDraggingDestination for the
    /// element mounted on it.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DropHostViewIvars]
    struct DropHostView;

    impl DropHostView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, info: *mut AnyObject) -> usize {
            drop_session_operation(&self.ivars().events, info)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, info: *mut AnyObject) -> usize {
            drop_session_operation(&self.ivars().events, info)
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_for_drag_operation(&self, info: *mut AnyObject) -> bool {
            drop_session_operation(&self.ivars().events, info) != DRAG_OPERATION_NONE
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, info: *mut AnyObject) -> bool {
            let events = self.ivars().events.clone();
            deliver_view_drop(self, &events, info)
        }
    }
);

impl DropHostView {
    fn new(mtm: MainThreadMarker, events: EventBindings) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(DropHostViewIvars { events });
        // SAFETY: initWithFrame: is NSView's designated initializer and the
        // ivars were initialized above on the main thread.
        unsafe { msg_send![super(object), initWithFrame: Rect::default()] }
    }
}

objc2::extern_class!(
    /// AppKit's lazily materialized file-export pasteboard writer.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct NSFilePromiseProvider;
);

struct PromiseProviderDelegateIvars {
    /// The drag source element's stable binding: the promise fetched at
    /// materialization time carries the write callback of the latest render.
    events: EventBindings,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PromiseProviderDelegateIvars]
    struct PromiseProviderDelegate;

    // SAFETY: NSObjectProtocol adds no invariants beyond NSObject.
    unsafe impl NSObjectProtocol for PromiseProviderDelegate {}

    impl PromiseProviderDelegate {
        #[unsafe(method(filePromiseProvider:fileNameForType:))]
        fn file_name_for_type(
            &self,
            _provider: *mut AnyObject,
            _file_type: *mut AnyObject,
        ) -> *mut AnyObject {
            let file_name = self
                .ivars()
                .events
                .file_promise()
                .map(|promise| promise.file_name().to_owned())
                .unwrap_or_default();
            autorelease_id(ns_string(&file_name))
        }

        #[unsafe(method(operationQueueForFilePromiseProvider:))]
        fn operation_queue(&self, _provider: *mut AnyObject) -> *mut AnyObject {
            // The promise's write callback is a main-thread Rust value like
            // every other handler, so materialization runs on the main queue.
            // SAFETY: mainQueue returns the process-lifetime shared queue.
            unsafe { msg_send![objc2::class!(NSOperationQueue), mainQueue] }
        }

        #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
        fn write_promise_to_url(
            &self,
            _provider: *mut AnyObject,
            url: *mut AnyObject,
            completion: *mut block2::Block<dyn Fn(*mut AnyObject)>,
        ) {
            // SAFETY: The destination URL is live for the duration of the
            // callback; its path bytes are copied before use.
            let destination = unsafe {
                let path: *mut AnyObject = msg_send![url, path];
                std::path::PathBuf::from(rust_string(path))
            };
            let outcome = self
                .ivars()
                .events
                .file_promise()
                .ok_or_else(|| "the drag source no longer promises a file".to_owned())
                .and_then(|promise| promise.write_to(&destination));
            // SAFETY: The completion block is live for the duration of the
            // callback; a promise error is a live autoreleased NSError.
            unsafe {
                let error = match outcome {
                    Ok(()) => std::ptr::null_mut(),
                    Err(reason) => promise_error(&reason),
                };
                if let Some(completion) = completion.as_ref() {
                    completion.call((error,));
                }
            }
        }
    }
);

impl PromiseProviderDelegate {
    fn new(mtm: MainThreadMarker, events: EventBindings) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(PromiseProviderDelegateIvars { events });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }
}

/// Builds the localized NSError a failed promise write reports back to the
/// destination.
unsafe fn promise_error(reason: &str) -> *mut AnyObject {
    // SAFETY: The description key is a live static NSString and the factory
    // methods return live autoreleased Foundation objects.
    unsafe {
        let description = ns_string(reason);
        let info: *mut AnyObject = msg_send![objc2::class!(NSDictionary),
            dictionaryWithObject: description.as_object(),
            forKey: LOCALIZED_DESCRIPTION_KEY
        ];
        let domain = ns_string("jp.bunko.rinka.drag-and-drop");
        msg_send![objc2::class!(NSError),
            errorWithDomain: domain.as_object(),
            code: 1_isize,
            userInfo: info
        ]
    }
}

struct RinkaPromiseProviderIvars {
    /// Encoded intra-application payload carried beside the file promise, so
    /// one drag session serves both an internal move and an external export.
    payload: RefCell<Option<String>>,
    /// NSFilePromiseProvider's delegate property is weak; the provider keeps
    /// its Rinka delegate alive for the drag session's lifetime.
    delegate_retainer: RefCell<Option<Retained<PromiseProviderDelegate>>>,
}

define_class!(
    /// File-promise provider that may additionally write the typed payload.
    #[unsafe(super = NSFilePromiseProvider)]
    #[thread_kind = MainThreadOnly]
    #[ivars = RinkaPromiseProviderIvars]
    struct RinkaPromiseProvider;

    impl RinkaPromiseProvider {
        #[unsafe(method(writableTypesForPasteboard:))]
        fn writable_types_for_pasteboard(&self, pasteboard: *mut AnyObject) -> *mut AnyObject {
            // SAFETY: The superclass implements the NSPasteboardWriting
            // protocol; both arrays are autoreleased.
            unsafe {
                let base: *mut AnyObject =
                    msg_send![super(self), writableTypesForPasteboard: pasteboard];
                if self.ivars().payload.borrow().is_none() {
                    return base;
                }
                msg_send![base, arrayByAddingObject: payload_pasteboard_type().as_object()]
            }
        }

        #[unsafe(method(pasteboardPropertyListForType:))]
        fn pasteboard_property_list_for_type(
            &self,
            pasteboard_type: *mut AnyObject,
        ) -> *mut AnyObject {
            if rust_string(pasteboard_type) == RINKA_PAYLOAD_PASTEBOARD_TYPE {
                let payload = self.ivars().payload.borrow().clone();
                return payload.map_or(std::ptr::null_mut(), |encoded| {
                    autorelease_id(ns_string(&encoded))
                });
            }
            // SAFETY: The superclass serves its promise bookkeeping types.
            unsafe { msg_send![super(self), pasteboardPropertyListForType: pasteboard_type] }
        }
    }
);

impl RinkaPromiseProvider {
    fn new(
        mtm: MainThreadMarker,
        content_type: &str,
        delegate: Retained<PromiseProviderDelegate>,
        payload: Option<String>,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(RinkaPromiseProviderIvars {
            payload: RefCell::new(payload),
            delegate_retainer: RefCell::new(None),
        });
        let file_type = ns_string(content_type);
        // SAFETY: initWithFileType:delegate: is the designated initializer;
        // the delegate property is weak, so the ivar below keeps the Rinka
        // delegate alive for the provider's lifetime.
        let provider: Retained<Self> = unsafe {
            msg_send![super(object),
                initWithFileType: file_type.as_object(),
                delegate: &*delegate
            ]
        };
        *provider.ivars().delegate_retainer.borrow_mut() = Some(delegate);
        provider
    }
}

/// Builds the pasteboard writer for one row's declared drag sources, or nil
/// when the row declares none (which keeps the row non-draggable).
fn row_pasteboard_writer(
    mtm: MainThreadMarker,
    record: &Rc<RefCell<TableRowRecord>>,
) -> *mut AnyObject {
    let events = record.borrow().events.clone();
    let promise = events.file_promise();
    let payload = events
        .drag_payload()
        .map(|payload| encode_drag_payload(&payload));
    match (promise, payload) {
        (Some(promise), payload) => {
            let delegate = PromiseProviderDelegate::new(mtm, events);
            let provider =
                RinkaPromiseProvider::new(mtm, promise.content_type(), delegate, payload);
            Retained::autorelease_return(provider).cast::<AnyObject>()
        }
        (None, Some(encoded)) => {
            let item = new_object(objc2::class!(NSPasteboardItem));
            // SAFETY: setString:forType: stores a copy on a live item.
            unsafe {
                let _: bool = msg_send![item.as_object(),
                    setString: ns_string(&encoded).as_object(),
                    forType: payload_pasteboard_type().as_object()
                ];
            }
            autorelease_id(item)
        }
        (None, None) => std::ptr::null_mut(),
    }
}

/// Configures a freshly created native table as a drag source. The table
/// only begins a session when a row's writer is non-nil.
fn configure_table_drag_source(table: &AnyObject) {
    // SAFETY: Both selectors are public NSTableView API. External sessions
    // export copies (file promises); local sessions move typed payloads.
    unsafe {
        let _: () = msg_send![table,
            setDraggingSourceOperationMask: DRAG_OPERATION_COPY,
            forLocal: false
        ];
        let _: () = msg_send![table,
            setDraggingSourceOperationMask: DRAG_OPERATION_MOVE | DRAG_OPERATION_COPY,
            forLocal: true
        ];
        let _: () = msg_send![table, setVerticalMotionCanBeginDrag: true];
    }
}

/// Aligns the native table's dragged-type registration with the current
/// declarative state: the list's own drop target or any row's drop target.
fn refresh_table_drag_registration(handle: &AppKitHandle) {
    let Some(delegate) = handle.0.table_delegate.borrow().as_ref().cloned() else {
        return;
    };
    let accepts = delegate.ivars().list_drop_target.borrow().is_some()
        || rows_declare_drop_target(&delegate.ivars().rows.borrow());
    set_view_drag_registration(handle.host_view(), accepts);
}

fn rows_declare_drop_target(rows: &[Rc<RefCell<TableRowRecord>>]) -> bool {
    rows.iter().any(|record| {
        if record.borrow().drop_target.is_some() {
            return true;
        }
        let children = record.borrow().children.borrow().clone();
        rows_declare_drop_target(&children)
    })
}

/// One resolved native table drop: the receiving binding plus the view whose
/// local coordinates express the drop position.
struct TableDropRoute {
    events: EventBindings,
    /// Origin of the target's frame within the table's content coordinates;
    /// zero for a list-level drop.
    frame_origin: Point,
}

/// A hovering or dropping session resolved against the declared models.
struct ResolvedTableDrop {
    route: TableDropRoute,
    /// The drag operation the session may perform.
    operation: usize,
    /// The routed row, kept for native drop-highlight retargeting; `None`
    /// routes to the whole list.
    routed_record: Option<Rc<RefCell<TableRowRecord>>>,
}

impl TableDelegate {
    /// Resolves where a hovering or dropping session lands: the proposed
    /// row when its model accepts the payload, otherwise the list itself.
    fn resolve_table_drop(
        &self,
        table: &AnyObject,
        info: *mut AnyObject,
        proposed_record: Option<Rc<RefCell<TableRowRecord>>>,
    ) -> Option<ResolvedTableDrop> {
        // SAFETY: The dragging info is live for the duration of the callback.
        let (payload, has_files) = unsafe {
            let pasteboard = dragging_pasteboard(info)?;
            (
                pasteboard_drag_payload(pasteboard.as_ref()),
                pasteboard_has_file_urls(pasteboard.as_ref()),
            )
        };
        if let Some(payload) = payload {
            if let Some(record) = proposed_record {
                let events = record.borrow().events.clone();
                if events
                    .drop_target()
                    .is_some_and(|target| target.accepts_payload_type(payload.payload_type()))
                {
                    let frame_origin = unsafe { record_frame_origin(table, &record) };
                    return Some(ResolvedTableDrop {
                        route: TableDropRoute {
                            events,
                            frame_origin,
                        },
                        operation: DRAG_OPERATION_MOVE,
                        routed_record: Some(record),
                    });
                }
            }
            let list_events = self.ivars().events.clone();
            if list_events
                .drop_target()
                .is_some_and(|target| target.accepts_payload_type(payload.payload_type()))
            {
                return Some(ResolvedTableDrop {
                    route: TableDropRoute {
                        events: list_events,
                        frame_origin: Point::default(),
                    },
                    operation: DRAG_OPERATION_MOVE,
                    routed_record: None,
                });
            }
            return None;
        }
        if has_files {
            let list_events = self.ivars().events.clone();
            if list_events
                .drop_target()
                .is_some_and(|target| target.accepts_files())
            {
                return Some(ResolvedTableDrop {
                    route: TableDropRoute {
                        events: list_events,
                        frame_origin: Point::default(),
                    },
                    operation: DRAG_OPERATION_COPY,
                    routed_record: None,
                });
            }
        }
        None
    }

    /// Delivers an accepted session through the route's binding, with the
    /// drop position local to the routed target.
    fn deliver_table_drop(
        &self,
        table: &AnyObject,
        info: *mut AnyObject,
        route: &TableDropRoute,
    ) -> bool {
        // SAFETY: The dragging info is live for the duration of the callback.
        let (payload, paths, table_position) = unsafe {
            let Some(pasteboard) = dragging_pasteboard(info) else {
                return false;
            };
            (
                pasteboard_drag_payload(pasteboard.as_ref()),
                pasteboard_file_paths(pasteboard.as_ref()),
                dragging_local_position(table, info),
            )
        };
        let position = DropPosition::new(
            table_position.x - route.frame_origin.x,
            table_position.y - route.frame_origin.y,
        );
        // No adapter borrow is held here: delivery may re-render this table.
        if let Some(payload) = payload {
            return route.events.emit_payload_drop(PayloadDrop { payload, position });
        }
        if !paths.is_empty() {
            return route.events.emit_file_drop(FileDrop { paths, position });
        }
        false
    }
}

/// Returns a record's row frame origin in table content coordinates, so a
/// row-level drop position is local to the row.
unsafe fn record_frame_origin(table: &AnyObject, record: &Rc<RefCell<TableRowRecord>>) -> Point {
    // SAFETY: The receiver is the live table serving the delegate callback;
    // NSTableView content coordinates are flipped (top-left origin).
    unsafe {
        let is_outline: bool = msg_send![table, isKindOfClass: objc2::class!(NSOutlineView)];
        let row: isize = if is_outline {
            msg_send![table, rowForItem: record.borrow().outline_identity.as_object()]
        } else {
            let rows = table_delegate_row_index(table, record);
            rows.map_or(-1, |index| isize::try_from(index).unwrap_or(-1))
        };
        if row < 0 {
            return Point::default();
        }
        let rect: Rect = msg_send![table, rectOfRow: row];
        rect.origin
    }
}

/// Finds a flat-table record's row index through the table's data source.
unsafe fn table_delegate_row_index(
    table: &AnyObject,
    record: &Rc<RefCell<TableRowRecord>>,
) -> Option<usize> {
    // SAFETY: The data source is the retained TableDelegate serving this
    // callback on the main thread.
    unsafe {
        let source: *mut AnyObject = msg_send![table, dataSource];
        let source = NonNull::new(source)?;
        let delegate = source.cast::<TableDelegate>();
        let rows = delegate.as_ref().ivars().rows.borrow();
        rows.iter().position(|candidate| Rc::ptr_eq(candidate, record))
    }
}
