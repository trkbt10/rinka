// Tabbed-document dock realization (`reports/document-tabs-and-splits`).
//
// The split tree is realized as recursive NSSplitView instances — every
// divider is the real AppKit divider. Each tab group is a group host: a tab
// strip over a native separator over a content host. AppKit ships no public
// reorderable tab-bar control (NSTabView cannot reorder tabs and
// NSWindowTabGroup tabs whole windows), so the strip is composed from real
// AppKit controls — one recessed toggle NSButton per tab (the Mail
// favorites-bar idiom), a real borderless close NSButton, and real
// NSTextField labels — inside an NSScrollView for overflow. The only custom
// behavior is container plumbing: hover tracking for the close affordance, a
// drag-threshold tracking loop, and NSDraggingSource/Destination glue. No
// control is drawn by hand; the written justification for this composition
// lives in the report.
//
// Tab drags ride the same private pasteboard transport as intra-application
// item drags (`drag_drop.rs`): the payload type identifies a dock tab and
// the payload id is the tab id, so both ends resolve everything else from
// the retained declarative layout. Every gesture — select, close, strip
// drop, content-edge drop — surfaces as a semantic `DockEvent` through the
// dock element's stable event binding; the consumer mutates its layout and
// the next render reconciles the retained native tree.

/// Payload type carried by a dragged dock tab; the payload id is the tab id.
const DOCK_TAB_PAYLOAD_TYPE: &str = "jp.bunko.rinka.dock-tab";

/// Narrowest tab item before titles truncate (adapter metric).
const DOCK_TAB_MIN_WIDTH: f64 = 96.0;
/// Widest tab item; longer titles truncate with a tooltip (adapter metric).
const DOCK_TAB_MAX_WIDTH: f64 = 220.0;
/// Tab strip height in points (adapter metric).
const DOCK_STRIP_HEIGHT: f64 = 28.0;
/// Edge band of a content host that requests a split on drop, as a fraction
/// of the dropped axis.
const DOCK_EDGE_BAND: f64 = 0.25;
/// Pointer travel that turns a tab press into a drag, in points.
const DOCK_DRAG_THRESHOLD: f64 = 4.0;

/// `NSEventMaskLeftMouseUp | NSEventMaskLeftMouseDragged`.
const DOCK_TRACKING_EVENT_MASK: u64 = (1 << 2) | (1 << 6);
/// `NSEventTypeLeftMouseUp`.
const EVENT_TYPE_LEFT_MOUSE_UP: usize = 2;
/// `NSTrackingMouseEnteredAndExited | NSTrackingActiveAlways |
/// NSTrackingInVisibleRect`.
const DOCK_TRACKING_AREA_OPTIONS: usize = 0x01 | 0x80 | 0x200;
/// `NSDraggingContextWithinApplication`.
const DRAGGING_CONTEXT_WITHIN_APPLICATION: isize = 1;
/// `NSSplitViewDividerStyleThin`.
const SPLIT_DIVIDER_STYLE_THIN: isize = 1;
/// `NSLineBreakByTruncatingTail`.
const LINE_BREAK_TRUNCATING_TAIL: usize = 4;
/// `NSBezelStyleRecessed` — AppKit's tab-shaped toggle bezel.
const BEZEL_STYLE_RECESSED: isize = 13;
/// `NSButtonTypePushOnPushOff`.
const BUTTON_TYPE_PUSH_ON_PUSH_OFF: isize = 1;

unsafe extern "C" {
    #[link_name = "NSEventTrackingRunLoopMode"]
    static EVENT_TRACKING_RUN_LOOP_MODE: *mut AnyObject;
}

objc2::extern_class!(
    /// AppKit control superclass in the NSButton inheritance chain.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct NSControl;
);

objc2::extern_class!(
    /// AppKit button superclass of the drag-aware tab button.
    #[unsafe(super(NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct NSButton;
);

/// Retained native realization of one mounted dock element.
struct DockState {
    /// The declarative layout shared with strip and content views, which
    /// resolve drop sources and validity against it at interaction time.
    layout: Rc<RefCell<DockLayout>>,
    /// The dock element's stable event binding.
    events: EventBindings,
    /// Group hosts retained across skeleton rebuilds, keyed by group id.
    groups: HashMap<String, DockGroupHost>,
    /// Mounted tab content views keyed by tab id.
    content: HashMap<String, DockContentEntry>,
    /// The realized root node view (a split view or a group container).
    root_view: Option<Id>,
    /// Constraints pinning the root node view to the dock element view.
    root_constraints: Vec<Id>,
    /// Realized split views in depth-first order with their current weights.
    splits: Vec<DockSplitRealization>,
    /// Structure signature of the realized skeleton.
    signature: String,
    /// Deferred weight application target.
    applier: Retained<DockWeightApplier>,
}

impl fmt::Debug for DockState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockState")
            .field("groups", &self.groups.len())
            .field("content", &self.content.len())
            .field("splits", &self.splits.len())
            .field("signature", &self.signature)
            .finish_non_exhaustive()
    }
}

struct DockSplitRealization {
    view: Id,
    axis: Axis,
    weights: Vec<f64>,
}

/// One mounted tab content subtree and where it is currently hosted.
struct DockContentEntry {
    view: Id,
    /// Present while the content is installed in a group's content host.
    hooked: Option<(String, Vec<Id>)>,
}

/// Retained native views of one tab group.
struct DockGroupHost {
    container: Id,
    strip: Retained<DockStripView>,
    strip_stack: Id,
    content_host: Retained<DockContentHostView>,
    tabs: Vec<DockTabItemHost>,
    constraints: Vec<Id>,
}

/// Retained native views and targets of one tab item.
struct DockTabItemHost {
    tab_id: String,
    item: Retained<DockTabItemView>,
    /// Retention only: NSControl holds its target weakly, so the select and
    /// close targets must live exactly as long as their buttons.
    _select_target: Retained<ActionTarget>,
    _close_target: Retained<ActionTarget>,
}

struct DockTabItemViewIvars {
    events: EventBindings,
    group_id: RefCell<String>,
    tab_id: RefCell<String>,
    active: Cell<bool>,
    dirty: Cell<bool>,
    closeable: Cell<bool>,
    hovered: Cell<bool>,
    select_button: RefCell<Option<Id>>,
    dot_label: RefCell<Option<Id>>,
    close_button: RefCell<Option<Id>>,
    tracking_area: RefCell<Option<Id>>,
    /// The realized per-tab context-menu model, for in-place refresh.
    menu_model: RefCell<Option<ContextMenu>>,
}

define_class!(
    /// Container of one tab's strip item: a real recessed NSButton carrying
    /// the title and selection state plus the dirty/close indicator slot.
    /// The container owns hover tracking, the tab drag session, and the
    /// per-tab context menu anchor.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DockTabItemViewIvars]
    struct DockTabItemView;

    impl DockTabItemView {
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            // SAFETY: Tracking areas are public NSView API on the main
            // thread; the previous area is removed before being replaced.
            unsafe {
                let _: () = msg_send![super(self), updateTrackingAreas];
                if let Some(area) = self.ivars().tracking_area.borrow_mut().take() {
                    let _: () = msg_send![self, removeTrackingArea: area.as_object()];
                }
                let allocated: *mut AnyObject = msg_send![objc2::class!(NSTrackingArea), alloc];
                let area: *mut AnyObject = msg_send![allocated,
                    initWithRect: Rect::default(),
                    options: DOCK_TRACKING_AREA_OPTIONS,
                    owner: self,
                    userInfo: std::ptr::null::<AnyObject>()
                ];
                let area = Id::from_owned(area);
                let _: () = msg_send![self, addTrackingArea: area.as_object()];
                *self.ivars().tracking_area.borrow_mut() = Some(area);
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: *mut AnyObject) {
            self.ivars().hovered.set(true);
            self.refresh_indicator();
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: *mut AnyObject) {
            self.ivars().hovered.set(false);
            self.refresh_indicator();
        }

        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operation_mask(&self, _session: *mut AnyObject, context: isize) -> usize {
            // Dock tabs move within the application only; nothing is
            // exported to other processes.
            if context == DRAGGING_CONTEXT_WITHIN_APPLICATION {
                DRAG_OPERATION_MOVE
            } else {
                DRAG_OPERATION_NONE
            }
        }
    }
);

impl DockTabItemView {
    fn new(
        mtm: MainThreadMarker,
        events: EventBindings,
        group_id: &str,
        tab_id: &str,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(DockTabItemViewIvars {
            events,
            group_id: RefCell::new(group_id.to_owned()),
            tab_id: RefCell::new(tab_id.to_owned()),
            active: Cell::new(false),
            dirty: Cell::new(false),
            closeable: Cell::new(true),
            hovered: Cell::new(false),
            select_button: RefCell::new(None),
            dot_label: RefCell::new(None),
            close_button: RefCell::new(None),
            tracking_area: RefCell::new(None),
            menu_model: RefCell::new(None),
        });
        // SAFETY: initWithFrame: is NSView's designated initializer and the
        // ivars were initialized above on the main thread.
        unsafe { msg_send![super(object), initWithFrame: Rect::default()] }
    }

    /// Shows the anatomy slot's current face: the close affordance while a
    /// closeable tab is hovered, otherwise the dirty dot, otherwise nothing.
    fn refresh_indicator(&self) {
        let show_close = self.ivars().hovered.get() && self.ivars().closeable.get();
        let show_dot = self.ivars().dirty.get() && !show_close;
        // SAFETY: The retained indicator views are live subviews mutated on
        // the main thread.
        unsafe {
            if let Some(close) = self.ivars().close_button.borrow().as_ref() {
                let _: () = msg_send![close.as_object(), setHidden: !show_close];
            }
            if let Some(dot) = self.ivars().dot_label.borrow().as_ref() {
                let _: () = msg_send![dot.as_object(), setHidden: !show_dot];
            }
        }
    }

    /// Reasserts the select button's toggle state from the reconciled model.
    fn refresh_select_state(&self) {
        // SAFETY: The retained button is a live NSButton on the main thread.
        unsafe {
            if let Some(button) = self.ivars().select_button.borrow().as_ref() {
                let _: () =
                    msg_send![button.as_object(), setState: isize::from(self.ivars().active.get())];
            }
        }
    }

    /// Requests selection through the stable binding, then reasserts the
    /// controlled toggle state — the consumer decides whether it changed.
    fn perform_select(&self) {
        let group = self.ivars().group_id.borrow().clone();
        let tab = self.ivars().tab_id.borrow().clone();
        let _ = self
            .ivars()
            .events
            .emit_dock(DockEvent::SelectTab { group, tab });
        // Message delivery re-rendered synchronously; the active cell now
        // holds the reconciled truth.
        self.refresh_select_state();
    }

    /// Requests the close through the stable binding; a dirty-tab veto is
    /// the consumer's decision, never the strip's.
    fn perform_close(&self) {
        let group = self.ivars().group_id.borrow().clone();
        let tab = self.ivars().tab_id.borrow().clone();
        let _ = self
            .ivars()
            .events
            .emit_dock(DockEvent::CloseTab { group, tab });
    }

    /// Begins the native drag session for this tab.
    fn begin_tab_drag(&self, event: *mut AnyObject) {
        let tab = self.ivars().tab_id.borrow().clone();
        let payload = DragPayload::new(DOCK_TAB_PAYLOAD_TYPE, tab);
        let encoded = encode_drag_payload(&payload);
        // SAFETY: All receivers are live main-thread AppKit objects; the
        // pasteboard item copies the encoded payload, the dragging item is
        // created through its designated initializer, and the session source
        // is this retained view.
        unsafe {
            let pasteboard_item = new_object(objc2::class!(NSPasteboardItem));
            let _: bool = msg_send![pasteboard_item.as_object(),
                setString: ns_string(&encoded).as_object(),
                forType: payload_pasteboard_type().as_object()
            ];
            let allocated: *mut AnyObject = msg_send![objc2::class!(NSDraggingItem), alloc];
            let dragging_item: *mut AnyObject =
                msg_send![allocated, initWithPasteboardWriter: pasteboard_item.as_object()];
            let dragging_item = Id::from_owned(dragging_item);
            let bounds: Rect = msg_send![self, bounds];
            match snapshot_view_image(self, bounds) {
                Some(image) => {
                    let _: () = msg_send![dragging_item.as_object(),
                        setDraggingFrame: bounds,
                        contents: image.as_object()
                    ];
                }
                None => {
                    let _: () = msg_send![dragging_item.as_object(), setDraggingFrame: bounds];
                }
            }
            let items = ns_array(&[dragging_item]);
            let _: *mut AnyObject = msg_send![self,
                beginDraggingSessionWithItems: items.as_object(),
                event: event,
                source: self
            ];
        }
    }
}

define_class!(
    /// Recessed toggle button carrying a tab's title, selection state, and
    /// accessibility exposure. Its mouse-down runs a drag-threshold tracking
    /// loop so a click selects while a drag starts the tab's native session;
    /// the accessibility press path keeps the ordinary target/action route.
    #[unsafe(super = NSButton)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct DockTabButton;

    impl DockTabButton {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: *mut AnyObject) {
            let Some(item) = self.owning_item() else {
                return;
            };
            // SAFETY: The tracking loop is the standard AppKit local-event
            // pattern: the window dequeues drag/up events on the main thread
            // in the tracking run-loop mode for the duration of the press.
            unsafe {
                let window: *mut AnyObject = msg_send![self, window];
                let Some(window) = NonNull::new(window) else {
                    return;
                };
                let start: Point = msg_send![event, locationInWindow];
                let distant_future: *mut AnyObject =
                    msg_send![objc2::class!(NSDate), distantFuture];
                loop {
                    let next: *mut AnyObject = msg_send![window.as_ref(),
                        nextEventMatchingMask: DOCK_TRACKING_EVENT_MASK,
                        untilDate: distant_future,
                        inMode: EVENT_TRACKING_RUN_LOOP_MODE,
                        dequeue: true
                    ];
                    let Some(next) = NonNull::new(next) else {
                        break;
                    };
                    let event_type: usize = msg_send![next.as_ref(), type];
                    let location: Point = msg_send![next.as_ref(), locationInWindow];
                    if event_type == EVENT_TYPE_LEFT_MOUSE_UP {
                        let local: Point = msg_send![self,
                            convertPoint: location,
                            fromView: std::ptr::null::<AnyObject>()
                        ];
                        let bounds: Rect = msg_send![self, bounds];
                        let inside: bool = msg_send![self, mouse: local, inRect: bounds];
                        if inside {
                            item.perform_select();
                        }
                        break;
                    }
                    let travel =
                        ((location.x - start.x).powi(2) + (location.y - start.y).powi(2)).sqrt();
                    if travel > DOCK_DRAG_THRESHOLD {
                        item.begin_tab_drag(next.as_ptr());
                        break;
                    }
                }
            }
        }
    }
);

impl DockTabButton {
    /// Finds the enclosing tab item container.
    fn owning_item(&self) -> Option<Retained<DockTabItemView>> {
        // SAFETY: The superview walk reads retained main-thread views; the
        // class check guarantees the cast target's type.
        unsafe {
            let mut view: *mut AnyObject = msg_send![self, superview];
            while let Some(current) = NonNull::new(view) {
                let is_item: bool =
                    msg_send![current.as_ref(), isKindOfClass: DockTabItemView::class()];
                if is_item {
                    return Some(Retained::retain(current.as_ptr().cast::<DockTabItemView>()))
                        .flatten();
                }
                view = msg_send![current.as_ref(), superview];
            }
            None
        }
    }
}

struct DockStripViewIvars {
    events: EventBindings,
    layout: Rc<RefCell<DockLayout>>,
    group_id: RefCell<String>,
    stack: RefCell<Option<Id>>,
    hovering: Cell<bool>,
}

define_class!(
    /// One group's tab strip: hosts the scrolling item row and serves tab
    /// drops (reorder within the group, move across groups) with a native
    /// hover highlight as session feedback.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DockStripViewIvars]
    struct DockStripView;

    impl DockStripView {
        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            update_drop_highlight_layer(self, self.ivars().hovering.get());
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            // SAFETY: Re-resolving the highlight color under the new
            // appearance is the documented updateLayer pattern.
            unsafe {
                let _: () = msg_send![super(self), viewDidChangeEffectiveAppearance];
                let _: () = msg_send![self, setNeedsDisplay: true];
            }
        }

        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, info: *mut AnyObject) -> usize {
            let operation = self.session_operation(info);
            self.set_hovering(operation != DRAG_OPERATION_NONE);
            operation
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, info: *mut AnyObject) -> usize {
            self.session_operation(info)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _info: *mut AnyObject) {
            self.set_hovering(false);
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_for_drag_operation(&self, info: *mut AnyObject) -> bool {
            self.session_operation(info) != DRAG_OPERATION_NONE
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, info: *mut AnyObject) -> bool {
            self.perform_strip_drop(info)
        }
    }
);

impl DockStripView {
    /// Delivers a completed strip drop as a semantic move request.
    fn perform_strip_drop(&self, info: *mut AnyObject) -> bool {
        self.set_hovering(false);
        let Some(tab) = dock_dragged_tab(info) else {
            return false;
        };
        // Resolve everything before emitting: delivery re-renders
        // synchronously and no layout borrow may be held across it.
        let resolved = {
            let layout = self.ivars().layout.borrow();
            layout
                .group_of_tab(&tab)
                .map(|source| (source.id.clone(), self.ivars().group_id.borrow().clone()))
        };
        let Some((from_group, to_group)) = resolved else {
            return false;
        };
        let index = self.drop_index(info);
        self.ivars().events.emit_dock(DockEvent::MoveTab {
            tab,
            from_group,
            to_group,
            index,
        })
    }

    fn new(
        mtm: MainThreadMarker,
        events: EventBindings,
        layout: Rc<RefCell<DockLayout>>,
        group_id: &str,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(DockStripViewIvars {
            events,
            layout,
            group_id: RefCell::new(group_id.to_owned()),
            stack: RefCell::new(None),
            hovering: Cell::new(false),
        });
        // SAFETY: initWithFrame: is NSView's designated initializer and the
        // ivars were initialized above on the main thread.
        let view: Retained<Self> = unsafe { msg_send![super(object), initWithFrame: Rect::default()] };
        register_dock_drag_types(&view);
        view
    }

    fn set_hovering(&self, hovering: bool) {
        if self.ivars().hovering.replace(hovering) != hovering {
            // SAFETY: Marking a live main-thread view dirty re-runs
            // updateLayer with the new highlight state.
            unsafe {
                let _: () = msg_send![self, setNeedsDisplay: true];
            }
        }
    }

    /// Returns the session operation: a move for a tab of this dock's
    /// layout, nothing for every other payload.
    fn session_operation(&self, info: *mut AnyObject) -> usize {
        let Some(tab) = dock_dragged_tab(info) else {
            return DRAG_OPERATION_NONE;
        };
        if self.ivars().layout.borrow().contains_tab(&tab) {
            DRAG_OPERATION_MOVE
        } else {
            DRAG_OPERATION_NONE
        }
    }

    /// Computes the strip insertion index from the session location against
    /// the arranged tab item midpoints.
    fn drop_index(&self, info: *mut AnyObject) -> usize {
        let stack = self.ivars().stack.borrow();
        let Some(stack) = stack.as_ref() else {
            return 0;
        };
        // SAFETY: The dragging info and the retained stack are live for the
        // duration of the callback; geometry reads are main-thread only.
        unsafe {
            let Some(info) = NonNull::new(info) else {
                return 0;
            };
            let location: Point = msg_send![info.as_ref(), draggingLocation];
            let local: Point = msg_send![stack.as_object(),
                convertPoint: location,
                fromView: std::ptr::null::<AnyObject>()
            ];
            let arranged: *mut AnyObject = msg_send![stack.as_object(), arrangedSubviews];
            let count: usize = msg_send![arranged, count];
            for index in 0..count {
                let item: *mut AnyObject = msg_send![arranged, objectAtIndex: index];
                let frame: Rect = msg_send![item, frame];
                if local.x < frame.origin.x + frame.size.width / 2.0 {
                    return index;
                }
            }
            count
        }
    }
}

struct DockContentHostViewIvars {
    events: EventBindings,
    layout: Rc<RefCell<DockLayout>>,
    group_id: RefCell<String>,
    hovering: Cell<bool>,
}

define_class!(
    /// One group's content host: presents the active tab's content and
    /// serves content drops — an edge drop requests a split, a center drop
    /// moves the tab to the end of this group's strip.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DockContentHostViewIvars]
    struct DockContentHostView;

    impl DockContentHostView {
        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            update_drop_highlight_layer(self, self.ivars().hovering.get());
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            // SAFETY: Re-resolving the highlight color under the new
            // appearance is the documented updateLayer pattern.
            unsafe {
                let _: () = msg_send![super(self), viewDidChangeEffectiveAppearance];
                let _: () = msg_send![self, setNeedsDisplay: true];
            }
        }

        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, info: *mut AnyObject) -> usize {
            let operation = self.session_operation(info);
            self.set_hovering(operation != DRAG_OPERATION_NONE);
            operation
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, info: *mut AnyObject) -> usize {
            self.session_operation(info)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _info: *mut AnyObject) {
            self.set_hovering(false);
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_for_drag_operation(&self, info: *mut AnyObject) -> bool {
            self.session_operation(info) != DRAG_OPERATION_NONE
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, info: *mut AnyObject) -> bool {
            self.perform_content_drop(info)
        }
    }
);

impl DockContentHostView {
    /// Delivers a completed content drop: an edge requests a split, the
    /// center requests a move to the end of this group's strip.
    fn perform_content_drop(&self, info: *mut AnyObject) -> bool {
        self.set_hovering(false);
        let Some(tab) = dock_dragged_tab(info) else {
            return false;
        };
        // Resolve against the layout, then drop the borrow: delivery
        // re-renders synchronously.
        let resolved = {
            let layout = self.ivars().layout.borrow();
            let target_group = self.ivars().group_id.borrow().clone();
            match (layout.group_of_tab(&tab), layout.find_group(&target_group)) {
                (Some(source), Some(target)) => Some((
                    source.id.clone(),
                    target_group,
                    target.tabs.len(),
                    source.id == target.id && source.tabs.len() == 1,
                )),
                _ => None,
            }
        };
        let Some((from_group, target_group, group_len, sole_tab_of_target)) = resolved else {
            return false;
        };
        let event = match self.drop_edge(info) {
            Some(edge) => {
                if sole_tab_of_target {
                    // Splitting a group by its own only tab would leave an
                    // empty group; the session refuses the no-op.
                    return false;
                }
                DockEvent::SplitGroup {
                    tab,
                    from_group,
                    target_group,
                    edge,
                }
            }
            None => DockEvent::MoveTab {
                tab,
                from_group,
                to_group: target_group,
                index: group_len,
            },
        };
        self.ivars().events.emit_dock(event)
    }

    fn new(
        mtm: MainThreadMarker,
        events: EventBindings,
        layout: Rc<RefCell<DockLayout>>,
        group_id: &str,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(DockContentHostViewIvars {
            events,
            layout,
            group_id: RefCell::new(group_id.to_owned()),
            hovering: Cell::new(false),
        });
        // SAFETY: initWithFrame: is NSView's designated initializer and the
        // ivars were initialized above on the main thread.
        let view: Retained<Self> = unsafe { msg_send![super(object), initWithFrame: Rect::default()] };
        register_dock_drag_types(&view);
        view
    }

    fn set_hovering(&self, hovering: bool) {
        if self.ivars().hovering.replace(hovering) != hovering {
            // SAFETY: Marking a live main-thread view dirty re-runs
            // updateLayer with the new highlight state.
            unsafe {
                let _: () = msg_send![self, setNeedsDisplay: true];
            }
        }
    }

    fn session_operation(&self, info: *mut AnyObject) -> usize {
        let Some(tab) = dock_dragged_tab(info) else {
            return DRAG_OPERATION_NONE;
        };
        let layout = self.ivars().layout.borrow();
        if !layout.contains_tab(&tab) {
            return DRAG_OPERATION_NONE;
        }
        // A group's only tab dropped anywhere on its own content is a no-op
        // (an edge split would empty the group; a center move keeps it).
        let group_id = self.ivars().group_id.borrow().clone();
        let sole_tab = layout
            .group_of_tab(&tab)
            .is_some_and(|source| source.id == group_id && source.tabs.len() == 1);
        if sole_tab {
            DRAG_OPERATION_NONE
        } else {
            DRAG_OPERATION_MOVE
        }
    }

    /// Resolves the dropped edge band, or `None` for the center region.
    fn drop_edge(&self, info: *mut AnyObject) -> Option<DockEdge> {
        // SAFETY: The dragging info and receiver are live for the duration
        // of the callback; the helper converts into local top-left space.
        let (position, bounds) = unsafe {
            let position = dragging_local_position(self, info);
            let bounds: Rect = msg_send![self, bounds];
            (position, bounds)
        };
        let width = bounds.size.width.max(1.0);
        let height = bounds.size.height.max(1.0);
        if position.x < width * DOCK_EDGE_BAND {
            Some(DockEdge::Leading)
        } else if position.x > width * (1.0 - DOCK_EDGE_BAND) {
            Some(DockEdge::Trailing)
        } else if position.y < height * DOCK_EDGE_BAND {
            Some(DockEdge::Top)
        } else if position.y > height * (1.0 - DOCK_EDGE_BAND) {
            Some(DockEdge::Bottom)
        } else {
            None
        }
    }
}

struct DockWeightApplierIvars {
    dock: RefCell<Option<std::rc::Weak<HandleInner>>>,
    attempts: Cell<usize>,
}

define_class!(
    /// Deferred split-weight application: divider positions require laid-out
    /// geometry, so weights apply on the next run-loop turn and retry until
    /// the split views have extent.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DockWeightApplierIvars]
    struct DockWeightApplier;

    // SAFETY: NSObjectProtocol adds no invariants beyond NSObject.
    unsafe impl NSObjectProtocol for DockWeightApplier {}

    impl DockWeightApplier {
        #[unsafe(method(applyDockWeights:))]
        fn apply_dock_weights(&self, _sender: *mut AnyObject) {
            let Some(inner) = self
                .ivars()
                .dock
                .borrow()
                .as_ref()
                .and_then(std::rc::Weak::upgrade)
            else {
                return;
            };
            let all_applied = {
                let state = inner.dock.borrow();
                let Some(state) = state.as_ref() else {
                    return;
                };
                state.splits.iter().all(apply_split_weights)
            };
            if !all_applied && self.ivars().attempts.get() < 60 {
                self.ivars().attempts.set(self.ivars().attempts.get() + 1);
                // SAFETY: The delayed selector re-runs this method on the
                // main thread once layout has had a chance to settle.
                unsafe {
                    let _: () = msg_send![self,
                        performSelector: sel!(applyDockWeights:),
                        withObject: std::ptr::null::<AnyObject>(),
                        afterDelay: 0.05_f64
                    ];
                }
            }
        }
    }
);

impl DockWeightApplier {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(DockWeightApplierIvars {
            dock: RefCell::new(None),
            attempts: Cell::new(0),
        });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }

    fn schedule(&self) {
        self.ivars().attempts.set(0);
        // SAFETY: The queued selector runs after the current reconciliation
        // pass, when Auto Layout has produced split geometry; superseded
        // requests are cancelled first.
        unsafe {
            let _: () = msg_send![objc2::class!(NSObject),
                cancelPreviousPerformRequestsWithTarget: self,
                selector: sel!(applyDockWeights:),
                object: std::ptr::null::<AnyObject>()
            ];
            let _: () = msg_send![self,
                performSelector: sel!(applyDockWeights:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.0_f64
            ];
        }
    }
}

/// Applies one split view's weights as divider positions. Returns whether
/// the view had usable geometry.
fn apply_split_weights(split: &DockSplitRealization) -> bool {
    // SAFETY: The retained split view is queried and positioned on the main
    // thread through public NSSplitView API.
    unsafe {
        let bounds: Rect = msg_send![split.view.as_object(), bounds];
        let extent = match split.axis {
            Axis::Horizontal => bounds.size.width,
            Axis::Vertical => bounds.size.height,
        };
        if extent < 1.0 {
            return false;
        }
        let divider: f64 = msg_send![split.view.as_object(), dividerThickness];
        let count = split.weights.len();
        let usable = extent - divider * count.saturating_sub(1) as f64;
        let total: f64 = split.weights.iter().sum();
        if usable <= 0.0 || total <= 0.0 {
            return true;
        }
        let mut cumulative = 0.0;
        for (index, weight) in split.weights.iter().take(count.saturating_sub(1)).enumerate() {
            cumulative += weight;
            let position = usable * (cumulative / total) + divider * index as f64;
            let _: () = msg_send![split.view.as_object(),
                setPosition: position,
                ofDividerAtIndex: index as isize
            ];
        }
        true
    }
}

/// Registers a dock drop view for the private payload transport only, so it
/// never swallows file or other payload sessions.
fn register_dock_drag_types<V: objc2::Message>(view: &Retained<V>) {
    // SAFETY: registerForDraggedTypes: is public NSView API on a live
    // main-thread view.
    unsafe {
        let types = ns_array(&[payload_pasteboard_type()]);
        let _: () = msg_send![&**view, registerForDraggedTypes: types.as_object()];
    }
}

/// Reads a dragged dock tab id from the session, if the session carries one.
fn dock_dragged_tab(info: *mut AnyObject) -> Option<String> {
    // SAFETY: The dragging info is live for the duration of the callback.
    unsafe {
        let pasteboard = dragging_pasteboard(info)?;
        let payload = pasteboard_drag_payload(pasteboard.as_ref())?;
        (payload.payload_type() == DOCK_TAB_PAYLOAD_TYPE).then(|| payload.id().to_owned())
    }
}

/// A `CGColorRef` with its Core Graphics type encoding, so message sends to
/// `CALayer.backgroundColor` type-check against the real signature.
#[repr(transparent)]
#[derive(Clone, Copy)]
struct CGColorRef(*const std::ffi::c_void);

// SAFETY: CGColorRef is a pointer to the opaque CGColor struct; this is the
// public Core Graphics ABI encoding (`^{CGColor=}`).
unsafe impl objc2::Encode for CGColorRef {
    const ENCODING: objc2::Encoding =
        objc2::Encoding::Pointer(&objc2::Encoding::Struct("CGColor", &[]));
}

/// Applies the appearance-resolved drop highlight to a dock drop view.
fn update_drop_highlight_layer<V: objc2::Message>(view: &V, hovering: bool) {
    // SAFETY: The receiver is a live layer-backed main-thread view; the
    // color is resolved under the view's current effective appearance each
    // time updateLayer runs, which keeps it correct across light and dark.
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        let Some(layer) = NonNull::new(layer) else {
            return;
        };
        if hovering {
            let color: *mut AnyObject =
                msg_send![objc2::class!(NSColor), selectedContentBackgroundColor];
            let color: *mut AnyObject = msg_send![color, colorWithAlphaComponent: 0.18_f64];
            let cg_color: CGColorRef = msg_send![color, CGColor];
            let _: () = msg_send![layer.as_ref(), setBackgroundColor: cg_color];
        } else {
            let _: () = msg_send![
                layer.as_ref(),
                setBackgroundColor: CGColorRef(std::ptr::null())
            ];
        }
    }
}

/// Renders a live snapshot of a view for the drag image.
fn snapshot_view_image<V: objc2::Message>(view: &V, bounds: Rect) -> Option<Id> {
    // SAFETY: The caching representation is created and filled by the view
    // itself on the main thread; the image copies the representation.
    unsafe {
        let representation: *mut AnyObject =
            msg_send![view, bitmapImageRepForCachingDisplayInRect: bounds];
        let representation = NonNull::new(representation)?;
        let _: () = msg_send![view,
            cacheDisplayInRect: bounds,
            toBitmapImageRep: representation.as_ref()
        ];
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSImage), alloc];
        let image: *mut AnyObject = msg_send![allocated, initWithSize: bounds.size];
        let image = Id::from_owned(image);
        let _: () = msg_send![image.as_object(), addRepresentation: representation.as_ref()];
        Some(image)
    }
}

/// Creates the retained native dock for one mounted element.
fn create_dock(
    mtm: MainThreadMarker,
    layout: &DockLayout,
    accessibility_label: &str,
    events: EventBindings,
) -> Result<AppKitHandle, AppKitError> {
    let view = new_view(objc2::class!(NSView));
    set_string(
        view.as_object(),
        SET_ACCESSIBILITY_LABEL,
        accessibility_label,
    );
    // The dock is a document area: it claims surplus window extent.
    configure_growth(view.as_object(), true, true);
    // SAFETY: The dimension guards keep collapse transitions non-negative.
    unsafe {
        let _ = nonnegative_dimension_constraint(msg_send![view.as_object(), widthAnchor]);
        let _ = nonnegative_dimension_constraint(msg_send![view.as_object(), heightAnchor]);
    }
    let handle = AppKitHandle::new(view, HostKind::Element(ElementKind::Dock), None, Vec::new());
    let applier = DockWeightApplier::new(mtm);
    *applier.ivars().dock.borrow_mut() = Some(Rc::downgrade(&handle.0));
    *handle.0.dock.borrow_mut() = Some(DockState {
        layout: Rc::new(RefCell::new(layout.clone())),
        events,
        groups: HashMap::new(),
        content: HashMap::new(),
        root_view: None,
        root_constraints: Vec::new(),
        splits: Vec::new(),
        signature: String::new(),
        applier,
    });
    reconcile_dock(mtm, &handle, layout, None)?;
    Ok(handle)
}

/// Reconciles the retained native dock onto the next declarative layout and
/// per-tab menus: a structural change rebuilds the split skeleton around the
/// retained group hosts, a weight change reapplies divider positions, and
/// every pass reconciles the strips and re-homes the content views.
fn reconcile_dock(
    mtm: MainThreadMarker,
    handle: &AppKitHandle,
    layout: &DockLayout,
    menus: Option<&DockTabMenus>,
) -> Result<(), AppKitError> {
    let dock_view = handle.0.view.clone();
    let mut state_slot = handle.0.dock.borrow_mut();
    let state = state_slot
        .as_mut()
        .ok_or_else(|| AppKitError("dock handle has no retained dock state".to_owned()))?;
    *state.layout.borrow_mut() = layout.clone();

    let signature = dock_structure_signature(layout);
    if signature != state.signature {
        rebuild_dock_skeleton(mtm, dock_view.as_object(), state, layout)?;
        state.signature = signature;
        state.applier.schedule();
    } else {
        let next_weights = collect_layout_weights(layout);
        let changed = state
            .splits
            .iter()
            .map(|split| &split.weights)
            .ne(next_weights.iter().map(|(_, weights)| weights));
        if changed {
            for (realization, (_, weights)) in state.splits.iter_mut().zip(next_weights) {
                realization.weights = weights;
            }
            state.applier.schedule();
        }
    }

    for group in layout.groups() {
        let events = state.events.clone();
        let layout_shared = state.layout.clone();
        let host = state
            .groups
            .get_mut(&group.id)
            .ok_or_else(|| AppKitError("dock skeleton is missing a group host".to_owned()))?;
        reconcile_group_strip(mtm, host, group, menus, &events, &layout_shared);
    }
    refresh_dock_content(state);
    Ok(())
}

/// Rebuilds the split skeleton for a structural layout change, reusing every
/// surviving group host so tab strips and content keep native identity.
fn rebuild_dock_skeleton(
    mtm: MainThreadMarker,
    dock_view: &AnyObject,
    state: &mut DockState,
    layout: &DockLayout,
) -> Result<(), AppKitError> {
    deactivate_constraints(&state.root_constraints);
    state.root_constraints.clear();
    state.splits.clear();
    if let Some(root) = state.root_view.take() {
        // SAFETY: The previous skeleton root is a live subview of the dock.
        unsafe {
            let _: () = msg_send![root.as_object(), removeFromSuperview];
        }
    }
    // Detach surviving group containers so the new skeleton can re-adopt
    // them, and tear down hosts whose groups disappeared.
    let live: std::collections::HashSet<String> = layout
        .groups()
        .into_iter()
        .map(|group| group.id.clone())
        .collect();
    state.groups.retain(|group_id, host| {
        // SAFETY: The container is a live view; removal detaches it and its
        // constraints from the abandoned skeleton.
        unsafe {
            let _: () = msg_send![host.container.as_object(), removeFromSuperview];
        }
        let survives = live.contains(group_id);
        if !survives {
            deactivate_constraints(&host.constraints);
        }
        survives
    });
    for group in layout.groups() {
        if !state.groups.contains_key(&group.id) {
            let host = create_group_host(mtm, state, &group.id);
            state.groups.insert(group.id.clone(), host);
        }
    }

    let root = build_dock_node(state, layout.root())?;
    // SAFETY: The fresh skeleton root is pinned to the dock element view.
    unsafe {
        let _: () = msg_send![
            root.as_object(),
            setTranslatesAutoresizingMaskIntoConstraints: false
        ];
        let _: () = msg_send![dock_view, addSubview: root.as_object()];
        state.root_constraints.extend([
            equal_anchor(
                msg_send![root.as_object(), leadingAnchor],
                msg_send![dock_view, leadingAnchor],
            ),
            equal_anchor(
                msg_send![dock_view, trailingAnchor],
                msg_send![root.as_object(), trailingAnchor],
            ),
            equal_anchor(
                msg_send![root.as_object(), topAnchor],
                msg_send![dock_view, topAnchor],
            ),
            equal_anchor(
                msg_send![dock_view, bottomAnchor],
                msg_send![root.as_object(), bottomAnchor],
            ),
        ]);
    }
    state.root_view = Some(root);
    Ok(())
}

/// Builds the native view realizing one layout node, recording split
/// realizations in depth-first order.
fn build_dock_node(state: &mut DockState, node: &DockNode) -> Result<Id, AppKitError> {
    match node {
        DockNode::Group(group) => {
            let host = state
                .groups
                .get(&group.id)
                .ok_or_else(|| AppKitError("dock group host was not prepared".to_owned()))?;
            Ok(host.container.clone())
        }
        DockNode::Split(split) => {
            let view = new_view(objc2::class!(NSSplitView));
            // SAFETY: The receiver is a live NSSplitView; a "vertical" split
            // view lays children side by side, which realizes the semantic
            // horizontal axis.
            unsafe {
                let _: () = msg_send![
                    view.as_object(),
                    setVertical: split.axis == Axis::Horizontal
                ];
                let _: () =
                    msg_send![view.as_object(), setDividerStyle: SPLIT_DIVIDER_STYLE_THIN];
            }
            state.splits.push(DockSplitRealization {
                view: view.clone(),
                axis: split.axis,
                weights: split.items.iter().map(|item| item.weight).collect(),
            });
            for item in &split.items {
                let child = build_dock_node(state, &item.node)?;
                // SAFETY: Arranged-subview insertion is public NSSplitView
                // API; the child is a live view detached above.
                unsafe {
                    let _: () = msg_send![
                        child.as_object(),
                        setTranslatesAutoresizingMaskIntoConstraints: false
                    ];
                    let _: () =
                        msg_send![view.as_object(), addArrangedSubview: child.as_object()];
                }
            }
            Ok(view)
        }
    }
}

/// Creates the retained native views of one tab group.
fn create_group_host(mtm: MainThreadMarker, state: &DockState, group_id: &str) -> DockGroupHost {
    let container = new_view(objc2::class!(NSView));
    let strip = DockStripView::new(
        mtm,
        state.events.clone(),
        state.layout.clone(),
        group_id,
    );
    let content_host = DockContentHostView::new(
        mtm,
        state.events.clone(),
        state.layout.clone(),
        group_id,
    );
    let scroll = new_view(objc2::class!(NSScrollView));
    let stack = new_view(objc2::class!(NSStackView));
    let separator = new_view(objc2::class!(NSBox));
    let mut constraints = Vec::new();
    // SAFETY: All receivers are live main-thread views created above; the
    // layout is strip (fixed height) over separator over content host.
    unsafe {
        let strip_view: *mut AnyObject = Retained::as_ptr(&strip).cast::<AnyObject>().cast_mut();
        let content_view: *mut AnyObject =
            Retained::as_ptr(&content_host).cast::<AnyObject>().cast_mut();
        for view in [strip_view, content_view] {
            let _: () = msg_send![view, setWantsLayer: true];
            let _: () = msg_send![view, setTranslatesAutoresizingMaskIntoConstraints: false];
            let _: () = msg_send![container.as_object(), addSubview: view];
        }

        // Horizontal tab row inside an overflow scroller.
        let _: () = msg_send![stack.as_object(), setOrientation: 0_isize];
        let _: () = msg_send![stack.as_object(), setSpacing: 2.0_f64];
        let _: () = msg_send![stack.as_object(), setTranslatesAutoresizingMaskIntoConstraints: false];
        let _: () = msg_send![scroll.as_object(), setDocumentView: stack.as_object()];
        let _: () = msg_send![scroll.as_object(), setDrawsBackground: false];
        let _: () = msg_send![scroll.as_object(), setHasHorizontalScroller: true];
        let _: () = msg_send![scroll.as_object(), setHasVerticalScroller: false];
        let _: () = msg_send![scroll.as_object(), setAutohidesScrollers: true];
        let _: () = msg_send![scroll.as_object(), setTranslatesAutoresizingMaskIntoConstraints: false];
        let _: () = msg_send![strip_view, addSubview: scroll.as_object()];
        let clip: *mut AnyObject = msg_send![scroll.as_object(), contentView];
        constraints.extend([
            equal_anchor(
                msg_send![scroll.as_object(), leadingAnchor],
                msg_send![strip_view, leadingAnchor],
            ),
            equal_anchor(
                msg_send![strip_view, trailingAnchor],
                msg_send![scroll.as_object(), trailingAnchor],
            ),
            equal_anchor(
                msg_send![scroll.as_object(), topAnchor],
                msg_send![strip_view, topAnchor],
            ),
            equal_anchor(
                msg_send![strip_view, bottomAnchor],
                msg_send![scroll.as_object(), bottomAnchor],
            ),
            equal_anchor(
                msg_send![stack.as_object(), leadingAnchor],
                msg_send![clip, leadingAnchor],
            ),
            equal_anchor(
                msg_send![stack.as_object(), topAnchor],
                msg_send![clip, topAnchor],
            ),
            equal_anchor(
                msg_send![stack.as_object(), heightAnchor],
                msg_send![clip, heightAnchor],
            ),
        ]);

        // Native separator line between strip and content.
        let _: () = msg_send![separator.as_object(), setBoxType: 2_isize];
        let _: () = msg_send![separator.as_object(), setTranslatesAutoresizingMaskIntoConstraints: false];
        let _: () = msg_send![container.as_object(), addSubview: separator.as_object()];

        constraints.extend([
            equal_anchor(
                msg_send![strip_view, leadingAnchor],
                msg_send![container.as_object(), leadingAnchor],
            ),
            equal_anchor(
                msg_send![container.as_object(), trailingAnchor],
                msg_send![strip_view, trailingAnchor],
            ),
            equal_anchor(
                msg_send![strip_view, topAnchor],
                msg_send![container.as_object(), topAnchor],
            ),
            dimension_constant_constraint(
                msg_send![strip_view, heightAnchor],
                DOCK_STRIP_HEIGHT,
                1000.0,
            ),
            equal_anchor(
                msg_send![separator.as_object(), leadingAnchor],
                msg_send![container.as_object(), leadingAnchor],
            ),
            equal_anchor(
                msg_send![container.as_object(), trailingAnchor],
                msg_send![separator.as_object(), trailingAnchor],
            ),
            equal_anchor(
                msg_send![separator.as_object(), topAnchor],
                msg_send![strip_view, bottomAnchor],
            ),
            dimension_constant_constraint(
                msg_send![separator.as_object(), heightAnchor],
                1.0,
                1000.0,
            ),
            equal_anchor(
                msg_send![content_view, leadingAnchor],
                msg_send![container.as_object(), leadingAnchor],
            ),
            equal_anchor(
                msg_send![container.as_object(), trailingAnchor],
                msg_send![content_view, trailingAnchor],
            ),
            equal_anchor(
                msg_send![content_view, topAnchor],
                msg_send![separator.as_object(), bottomAnchor],
            ),
            equal_anchor(
                msg_send![container.as_object(), bottomAnchor],
                msg_send![content_view, bottomAnchor],
            ),
            nonnegative_dimension_constraint(msg_send![container.as_object(), widthAnchor]),
            nonnegative_dimension_constraint(msg_send![container.as_object(), heightAnchor]),
        ]);
    }
    *strip.ivars().stack.borrow_mut() = Some(stack.clone());
    configure_growth(container.as_object(), true, true);
    DockGroupHost {
        container,
        strip,
        strip_stack: stack,
        content_host,
        tabs: Vec::new(),
        constraints,
    }
}

/// Reconciles one group's strip: items are reused by tab id, ordered to the
/// declared strip order, updated in place, and the active tab is scrolled
/// into view.
fn reconcile_group_strip(
    mtm: MainThreadMarker,
    host: &mut DockGroupHost,
    group: &DockGroup,
    menus: Option<&DockTabMenus>,
    events: &EventBindings,
    layout: &Rc<RefCell<DockLayout>>,
) {
    let mut existing = std::mem::take(&mut host.tabs);
    let mut next_hosts = Vec::with_capacity(group.tabs.len());
    for tab in &group.tabs {
        let item_host = match existing.iter().position(|item| item.tab_id == tab.id) {
            Some(index) => existing.remove(index),
            None => create_tab_item(mtm, events, layout, &group.id, &tab.id),
        };
        update_tab_item(&item_host, tab, tab.id == group.active, menus, mtm);
        next_hosts.push(item_host);
    }
    // Remove the leftovers from the row.
    for removed in existing {
        // SAFETY: The item is a live arranged subview of the strip stack.
        unsafe {
            let _: () = msg_send![host.strip_stack.as_object(), removeArrangedSubview: &*removed.item];
            let _: () = msg_send![&*removed.item, removeFromSuperview];
        }
    }
    // Enforce the declared order with minimal churn.
    for (index, item_host) in next_hosts.iter().enumerate() {
        // SAFETY: Arranged-subview queries and insertion are public
        // NSStackView API; inserting a view already arranged moves it.
        unsafe {
            let arranged: *mut AnyObject =
                msg_send![host.strip_stack.as_object(), arrangedSubviews];
            let count: usize = msg_send![arranged, count];
            let current = (0..count).find(|candidate| {
                let view: *mut AnyObject = msg_send![arranged, objectAtIndex: *candidate];
                std::ptr::eq(
                    view.cast::<DockTabItemView>(),
                    Retained::as_ptr(&item_host.item),
                )
            });
            if current != Some(index) {
                if current.is_some() {
                    let _: () = msg_send![
                        host.strip_stack.as_object(),
                        removeArrangedSubview: &*item_host.item
                    ];
                }
                let _: () = msg_send![host.strip_stack.as_object(),
                    insertArrangedSubview: &*item_host.item,
                    atIndex: index
                ];
            }
        }
    }
    host.tabs = next_hosts;
    // Keep the active tab visible in an overflowing strip.
    if let Some(active) = host.tabs.iter().find(|item| item.tab_id == group.active) {
        // SAFETY: Layout then scroll-to-visible on live main-thread views.
        unsafe {
            let strip: *mut AnyObject =
                Retained::as_ptr(&host.strip).cast::<AnyObject>().cast_mut();
            let _: () = msg_send![strip, layoutSubtreeIfNeeded];
            let bounds: Rect = msg_send![&*active.item, bounds];
            let _: bool = msg_send![&*active.item, scrollRectToVisible: bounds];
        }
    }
}

/// Creates the retained views and targets of one tab item.
fn create_tab_item(
    mtm: MainThreadMarker,
    events: &EventBindings,
    _layout: &Rc<RefCell<DockLayout>>,
    group_id: &str,
    tab_id: &str,
) -> DockTabItemHost {
    let item = DockTabItemView::new(mtm, events.clone(), group_id, tab_id);
    // Select button: the real recessed toggle carrying title and state.
    // SAFETY: The custom button subclass is instantiated through NSView's
    // designated initializer and configured with public NSButton API.
    let button = unsafe {
        let allocated = DockTabButton::alloc(mtm).set_ivars(());
        let button: Retained<DockTabButton> =
            msg_send![super(allocated), initWithFrame: Rect::default()];
        let _: () = msg_send![&*button, setBezelStyle: BEZEL_STYLE_RECESSED];
        let _: () = msg_send![&*button, setButtonType: BUTTON_TYPE_PUSH_ON_PUSH_OFF];
        let _: () = msg_send![&*button, setControlSize: 1_isize];
        let _: () = msg_send![&*button, setShowsBorderOnlyWhileMouseInside: true];
        let cell: *mut AnyObject = msg_send![&*button, cell];
        let _: () = msg_send![cell, setLineBreakMode: LINE_BREAK_TRUNCATING_TAIL];
        Id::from_borrowed(Retained::as_ptr(&button).cast::<AnyObject>().cast_mut())
    };
    let select_item = item.clone();
    let select_target = ActionTarget::new(
        mtm,
        EventBindings::activate(Rc::new(move || select_item.perform_select())),
        TargetKind::Activate,
    );
    // SAFETY: NSControl target/action wiring; the target is retained by the
    // item host because NSControl holds it weakly.
    unsafe {
        let _: () = msg_send![button.as_object(), setTarget: &*select_target];
        let _: () = msg_send![button.as_object(), setAction: sel!(performAction:)];
    }

    // Indicator slot: dirty dot and hover close share one fixed-size well.
    let slot = new_view(objc2::class!(NSView));
    let dot = label_view("\u{25CF}", TextRole::Secondary);
    let close_item = item.clone();
    let close_target = ActionTarget::new(
        mtm,
        EventBindings::activate(Rc::new(move || close_item.perform_close())),
        TargetKind::Activate,
    );
    // SAFETY: The close control is a real borderless NSButton created by a
    // class convenience constructor; its target is retained by the host.
    let close = unsafe {
        let pointer: *mut AnyObject = match system_image_named("xmark") {
            Some(image) => msg_send![objc2::class!(NSButton),
                buttonWithImage: image.as_object(),
                target: &*close_target,
                action: sel!(performAction:)
            ],
            None => msg_send![objc2::class!(NSButton),
                buttonWithTitle: ns_string("\u{2715}").as_object(),
                target: &*close_target,
                action: sel!(performAction:)
            ],
        };
        let close = Id::from_borrowed(pointer);
        let _: () = msg_send![close.as_object(), setBordered: false];
        let _: () = msg_send![close.as_object(), setControlSize: 2_isize];
        close
    };
    set_string(close.as_object(), SET_ACCESSIBILITY_LABEL, "Close Tab");
    set_string(close.as_object(), "setToolTip:", "Close Tab");

    // SAFETY: All receivers are live main-thread views; the item hosts the
    // button and the indicator slot with plain anchor constraints.
    unsafe {
        for view in [button.as_object(), slot.as_object()] {
            let _: () = msg_send![view, setTranslatesAutoresizingMaskIntoConstraints: false];
            let _: () = msg_send![&*item, addSubview: view];
        }
        for view in [dot.as_object(), close.as_object()] {
            let _: () = msg_send![view, setTranslatesAutoresizingMaskIntoConstraints: false];
            let _: () = msg_send![view, setHidden: true];
            let _: () = msg_send![slot.as_object(), addSubview: view];
            let _ = equal_anchor(
                msg_send![view, centerXAnchor],
                msg_send![slot.as_object(), centerXAnchor],
            );
            let _ = equal_anchor(
                msg_send![view, centerYAnchor],
                msg_send![slot.as_object(), centerYAnchor],
            );
        }
        let _ = dimension_constant_constraint(msg_send![slot.as_object(), widthAnchor], 16.0, 1000.0);
        let _ = dimension_constant_constraint(msg_send![slot.as_object(), heightAnchor], 16.0, 1000.0);
        let item_view: *mut AnyObject = Retained::as_ptr(&item).cast::<AnyObject>().cast_mut();
        let _ = equal_anchor_with_priority(
            msg_send![button.as_object(), leadingAnchor],
            msg_send![item_view, leadingAnchor],
            1000.0,
        );
        let _ = equal_anchor(
            msg_send![slot.as_object(), leadingAnchor],
            msg_send![button.as_object(), trailingAnchor],
        );
        let _ = equal_anchor(
            msg_send![item_view, trailingAnchor],
            msg_send![slot.as_object(), trailingAnchor],
        );
        let _ = equal_anchor(
            msg_send![button.as_object(), centerYAnchor],
            msg_send![item_view, centerYAnchor],
        );
        let _ = equal_anchor(
            msg_send![slot.as_object(), centerYAnchor],
            msg_send![item_view, centerYAnchor],
        );
        let _ = dimension_constant_constraint(
            msg_send![item_view, heightAnchor],
            DOCK_STRIP_HEIGHT - 4.0,
            1000.0,
        );
        // The anatomy contract: a minimum tab width with overflow scrolling
        // and a maximum beyond which titles truncate.
        let min_pointer: *mut AnyObject = msg_send![item_view, widthAnchor];
        let min_constraint: *mut AnyObject =
            msg_send![min_pointer, constraintGreaterThanOrEqualToConstant: DOCK_TAB_MIN_WIDTH];
        let _ = activate_constraint(min_constraint);
        let max_pointer: *mut AnyObject = msg_send![item_view, widthAnchor];
        let max_constraint: *mut AnyObject =
            msg_send![max_pointer, constraintLessThanOrEqualToConstant: DOCK_TAB_MAX_WIDTH];
        let _ = activate_constraint(max_constraint);
        // Titles compress into truncation instead of widening the strip.
        let _: () = msg_send![button.as_object(),
            setContentCompressionResistancePriority: 250.0_f32,
            forOrientation: 0_isize
        ];
        let _: () = msg_send![button.as_object(),
            setContentHuggingPriority: 1.0_f32,
            forOrientation: 0_isize
        ];
    }
    *item.ivars().select_button.borrow_mut() = Some(button);
    *item.ivars().dot_label.borrow_mut() = Some(dot);
    *item.ivars().close_button.borrow_mut() = Some(close);
    DockTabItemHost {
        tab_id: tab_id.to_owned(),
        item,
        _select_target: select_target,
        _close_target: close_target,
    }
}

/// Updates one retained tab item to the declared chrome.
fn update_tab_item(
    host: &DockTabItemHost,
    tab: &DockTab,
    active: bool,
    menus: Option<&DockTabMenus>,
    mtm: MainThreadMarker,
) {
    let ivars = host.item.ivars();
    ivars.active.set(active);
    ivars.dirty.set(tab.dirty);
    ivars.closeable.set(tab.closeable);
    if let Some(button) = ivars.select_button.borrow().as_ref() {
        set_string(button.as_object(), SET_TITLE, &tab.title);
        set_string(button.as_object(), SET_ACCESSIBILITY_LABEL, &tab.title);
        set_string(button.as_object(), "setToolTip:", &tab.title);
    }
    host.item.refresh_select_state();
    host.item.refresh_indicator();
    let dock_events = ivars.events.clone();
    let tab_id = tab.id.clone();
    reconcile_dock_tab_menu(
        mtm,
        // SAFETY: The item is a live retained view serving as the menu
        // anchor for its whole extent, the button included.
        unsafe { &*Retained::as_ptr(&host.item).cast::<AnyObject>() },
        &ivars.menu_model,
        menus.and_then(|menus| menus.menu_for(&tab.id)),
        &dock_events,
        &tab_id,
    );
}

/// Realizes, updates, or removes one tab's context menu on its item view.
///
/// This mirrors `reconcile_view_context_menu`, but activation dispatches
/// through the dock binding's per-tab menu channel so handlers stay current
/// across renders.
fn reconcile_dock_tab_menu(
    mtm: MainThreadMarker,
    view: &AnyObject,
    stored: &RefCell<Option<ContextMenu>>,
    next: Option<&ContextMenu>,
    dock_events: &EventBindings,
    tab_id: &str,
) {
    let mut stored = stored.borrow_mut();
    let build = |menu: &ContextMenu| {
        let native = create_ns_menu("");
        let mut targets = Vec::new();
        append_ns_menu_entries(&native, &menu.entries, true, &mut targets, &{
            let dock_events = dock_events.clone();
            let tab_id = tab_id.to_owned();
            move |item: &MenuItem| {
                let dock_events = dock_events.clone();
                let tab_id = tab_id.clone();
                let item_id = item.id.clone();
                ActionTarget::new(
                    mtm,
                    EventBindings::activate(Rc::new(move || {
                        let _ = dock_events.emit_dock_tab_menu_activation(&tab_id, &item_id);
                    })),
                    TargetKind::Activate,
                )
            }
        });
        // The items retain their targets through representedObject.
        drop(targets);
        native
    };
    match (stored.as_ref(), next) {
        (None, None) => {}
        (Some(_), None) => {
            // SAFETY: A nil menu removes the contextual interaction.
            unsafe {
                let _: () = msg_send![view, setMenu: std::ptr::null::<AnyObject>()];
            }
            *stored = None;
        }
        (None, Some(menu)) => {
            let native = build(menu);
            // SAFETY: The receiver is a live NSView and retains its menu.
            unsafe {
                let _: () = msg_send![view, setMenu: native.as_object()];
            }
            *stored = Some(menu.clone());
        }
        (Some(current), Some(menu)) => {
            if current == menu {
                return;
            }
            if menu_structure_matches(&current.entries, &menu.entries) {
                // SAFETY: The retained menu was realized from the stored
                // model, whose structure matches the next model.
                unsafe {
                    let native: *mut AnyObject = msg_send![view, menu];
                    if let Some(native) = NonNull::new(native) {
                        refresh_ns_menu_items(native.as_ref(), &menu.entries, true);
                    }
                }
            } else {
                let native = build(menu);
                // SAFETY: The receiver is a live NSView and retains its menu.
                unsafe {
                    let _: () = msg_send![view, setMenu: native.as_object()];
                }
            }
            *stored = Some(menu.clone());
        }
    }
}

/// Homes every mounted content view: the active tab of each group is
/// installed in that group's content host; every other content is detached
/// but retained, exactly like a native tab view's unselected pages.
fn refresh_dock_content(state: &mut DockState) {
    let layout = state.layout.borrow().clone();
    for (tab_id, entry) in &mut state.content {
        let target_group = layout
            .group_of_tab(tab_id)
            .filter(|group| group.active == *tab_id)
            .map(|group| group.id.clone());
        if let (Some((current, _)), Some(target)) = (&entry.hooked, &target_group)
            && current == target
        {
            continue;
        }
        if let Some((_, constraints)) = entry.hooked.take() {
            deactivate_constraints(&constraints);
            // SAFETY: The content view is a live subview of its old host.
            unsafe {
                let _: () = msg_send![entry.view.as_object(), removeFromSuperview];
            }
        }
        let Some(target) = target_group else {
            continue;
        };
        let Some(host) = state.groups.get(&target) else {
            continue;
        };
        // SAFETY: The content view is installed edge to edge in the live
        // content host on the main thread.
        let constraints = unsafe {
            let host_view: *mut AnyObject = Retained::as_ptr(&host.content_host)
                .cast::<AnyObject>()
                .cast_mut();
            let _: () = msg_send![
                entry.view.as_object(),
                setTranslatesAutoresizingMaskIntoConstraints: false
            ];
            let _: () = msg_send![host_view, addSubview: entry.view.as_object()];
            vec![
                equal_anchor(
                    msg_send![entry.view.as_object(), leadingAnchor],
                    msg_send![host_view, leadingAnchor],
                ),
                equal_anchor(
                    msg_send![host_view, trailingAnchor],
                    msg_send![entry.view.as_object(), trailingAnchor],
                ),
                equal_anchor(
                    msg_send![entry.view.as_object(), topAnchor],
                    msg_send![host_view, topAnchor],
                ),
                equal_anchor(
                    msg_send![host_view, bottomAnchor],
                    msg_send![entry.view.as_object(), bottomAnchor],
                ),
            ]
        };
        entry.hooked = Some((target, constraints));
    }
}

/// Registers one mounted content child under its tab id and homes it.
fn dock_attach_content(
    parent: &AppKitHandle,
    tab_id: &str,
    child: &AppKitHandle,
) -> Result<(), AppKitError> {
    let mut state_slot = parent.0.dock.borrow_mut();
    let state = state_slot
        .as_mut()
        .ok_or_else(|| AppKitError("dock handle has no retained dock state".to_owned()))?;
    state.content.insert(
        tab_id.to_owned(),
        DockContentEntry {
            view: child.0.view.clone(),
            hooked: None,
        },
    );
    refresh_dock_content(state);
    Ok(())
}

/// Unhooks and forgets one removed content child.
fn dock_detach_content(parent: &AppKitHandle, child: &AppKitHandle) -> Result<(), AppKitError> {
    let mut state_slot = parent.0.dock.borrow_mut();
    let state = state_slot
        .as_mut()
        .ok_or_else(|| AppKitError("dock handle has no retained dock state".to_owned()))?;
    let tab_id = state
        .content
        .iter()
        .find_map(|(tab_id, entry)| {
            (entry.view.as_ptr() == child.0.view.as_ptr()).then(|| tab_id.clone())
        })
        .ok_or_else(|| AppKitError("removed dock child is not a registered content".to_owned()))?;
    if let Some(mut entry) = state.content.remove(&tab_id)
        && let Some((_, constraints)) = entry.hooked.take()
    {
        deactivate_constraints(&constraints);
        // SAFETY: The content view is a live subview of its host.
        unsafe {
            let _: () = msg_send![entry.view.as_object(), removeFromSuperview];
        }
    }
    Ok(())
}

/// Structure signature deciding when the split skeleton must be rebuilt:
/// axes, arities, and group identities — weights and tab chrome excluded.
fn dock_structure_signature(layout: &DockLayout) -> String {
    fn write_node(node: &DockNode, output: &mut String) {
        match node {
            DockNode::Group(group) => {
                output.push_str("G(");
                output.push_str(&group.id);
                output.push(')');
            }
            DockNode::Split(split) => {
                output.push_str(match split.axis {
                    Axis::Horizontal => "Sh(",
                    Axis::Vertical => "Sv(",
                });
                for item in &split.items {
                    write_node(&item.node, output);
                    output.push(',');
                }
                output.push(')');
            }
        }
    }
    let mut signature = String::new();
    write_node(layout.root(), &mut signature);
    signature
}

/// Collects split weights in the same depth-first order the skeleton build
/// records realizations.
fn collect_layout_weights(layout: &DockLayout) -> Vec<(Axis, Vec<f64>)> {
    fn walk(node: &DockNode, output: &mut Vec<(Axis, Vec<f64>)>) {
        if let DockNode::Split(split) = node {
            output.push((
                split.axis,
                split.items.iter().map(|item| item.weight).collect(),
            ));
            for item in &split.items {
                walk(&item.node, output);
            }
        }
    }
    let mut weights = Vec::new();
    walk(layout.root(), &mut weights);
    weights
}
