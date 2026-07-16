struct TableRowRecord {
    title: String,
    subtitle: Option<String>,
    cells: Vec<String>,
    role: ListRowRole,
    expanded: bool,
    symbol: Option<Symbol>,
    selected: bool,
    disclosure: bool,
    accessibility_label: String,
    /// Declarative context menu realized on every cell this row produces.
    context_menu: Option<ContextMenu>,
    /// Declarative drop-target model, kept beside the record so the table's
    /// dragged-type registration follows the declared state exactly (the
    /// stable binding is only current after the whole render settles).
    drop_target: Option<DropTarget>,
    events: EventBindings,
    children: RefCell<Vec<Rc<RefCell<TableRowRecord>>>>,
    outline_identity: Id,
    table: RefCell<Option<Id>>,
}

struct TableDelegateIvars {
    rows: RefCell<Vec<Rc<RefCell<TableRowRecord>>>>,
    pattern: RefCell<CollectionPattern>,
    columns: RefCell<Vec<TableColumn>>,
    events: EventBindings,
    /// The list element's own declarative drop-target model, mirrored here
    /// for the same registration purpose as the per-row copy.
    list_drop_target: RefCell<Option<DropTarget>>,
    suppress_selection: RefCell<bool>,
    suppress_expansion: RefCell<bool>,
    suppress_split_expansion: RefCell<bool>,
    suppress_sort: RefCell<bool>,
}

impl fmt::Debug for TableDelegateIvars {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TableDelegateIvars")
            .field("row_count", &self.rows.borrow().len())
            .field("pattern", &self.pattern.borrow())
            .field("column_count", &self.columns.borrow().len())
            .finish()
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = TableDelegateIvars]
    struct TableDelegate;

    // SAFETY: NSObjectProtocol adds no invariants beyond NSObject.
    unsafe impl NSObjectProtocol for TableDelegate {}

    impl TableDelegate {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &AnyObject) -> isize {
            isize::try_from(self.ivars().rows.borrow().len()).unwrap_or(isize::MAX)
        }

        #[unsafe(method(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            _table: &AnyObject,
            column: *mut AnyObject,
            row: isize,
        ) -> *mut AnyObject {
            let Ok(index) = usize::try_from(row) else {
                return std::ptr::null_mut();
            };
            let rows = self.ivars().rows.borrow();
            let Some(record) = rows.get(index) else {
                return std::ptr::null_mut();
            };
            let pattern = *self.ivars().pattern.borrow();
            let column_index = table_column_index(column, &self.ivars().columns.borrow());
            create_table_cell(self.mtm(), &record.borrow(), pattern, column_index)
        }

        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        fn outline_number_of_children(
            &self,
            _outline: &AnyObject,
            item: *mut AnyObject,
        ) -> isize {
            let rows = self.ivars().rows.borrow();
            let count = if item.is_null() {
                rows.len()
            } else {
                find_outline_record(&rows, item)
                    .map(|record| record.borrow().children.borrow().len())
                    .unwrap_or(0)
            };
            isize::try_from(count).unwrap_or(isize::MAX)
        }

        #[unsafe(method(outlineView:child:ofItem:))]
        fn outline_child(
            &self,
            _outline: &AnyObject,
            index: isize,
            item: *mut AnyObject,
        ) -> *mut AnyObject {
            let Ok(index) = usize::try_from(index) else {
                return std::ptr::null_mut();
            };
            let rows = self.ivars().rows.borrow();
            if item.is_null() {
                return rows
                    .get(index)
                    .map_or(std::ptr::null_mut(), |record| {
                        record.borrow().outline_identity.as_ptr()
                    });
            }
            find_outline_record(&rows, item)
                .and_then(|record| record.borrow().children.borrow().get(index).cloned())
                .map_or(std::ptr::null_mut(), |record| {
                    record.borrow().outline_identity.as_ptr()
                })
        }

        #[unsafe(method(outlineView:isItemExpandable:))]
        fn outline_item_is_expandable(
            &self,
            _outline: &AnyObject,
            item: *mut AnyObject,
        ) -> bool {
            let rows = self.ivars().rows.borrow();
            find_outline_record(&rows, item)
                .is_some_and(|record| !record.borrow().children.borrow().is_empty())
        }

        #[unsafe(method(outlineView:objectValueForTableColumn:byItem:))]
        fn outline_object_value(
            &self,
            _outline: &AnyObject,
            _column: *mut AnyObject,
            item: *mut AnyObject,
        ) -> *mut AnyObject {
            let rows = self.ivars().rows.borrow();
            let Some(record) = find_outline_record(&rows, item) else {
                return std::ptr::null_mut();
            };
            autorelease_id(ns_string(&record.borrow().title))
        }

        #[unsafe(method(outlineView:viewForTableColumn:item:))]
        fn outline_view_for_item(
            &self,
            _outline: &AnyObject,
            column: *mut AnyObject,
            item: *mut AnyObject,
        ) -> *mut AnyObject {
            let rows = self.ivars().rows.borrow();
            let Some(record) = find_outline_record(&rows, item) else {
                return std::ptr::null_mut();
            };
            let pattern = *self.ivars().pattern.borrow();
            let column_index = table_column_index(column, &self.ivars().columns.borrow());
            create_table_cell(self.mtm(), &record.borrow(), pattern, column_index)
        }

        #[unsafe(method(outlineView:isGroupItem:))]
        fn outline_is_group_item(&self, _outline: &AnyObject, item: *mut AnyObject) -> bool {
            let rows = self.ivars().rows.borrow();
            find_outline_record(&rows, item)
                .is_some_and(|record| record.borrow().role == ListRowRole::Section)
        }

        #[unsafe(method(outlineView:shouldSelectItem:))]
        fn outline_should_select_item(
            &self,
            _outline: &AnyObject,
            item: *mut AnyObject,
        ) -> bool {
            let rows = self.ivars().rows.borrow();
            find_outline_record(&rows, item)
                .is_some_and(|record| record.borrow().role == ListRowRole::Item)
        }

        #[unsafe(method(outlineView:shouldExpandItem:))]
        fn outline_should_expand_item(
            &self,
            outline: &AnyObject,
            item: *mut AnyObject,
        ) -> bool {
            if *self.ivars().suppress_expansion.borrow()
                || *self.ivars().suppress_split_expansion.borrow()
            {
                return true.into();
            }
            if !outline_expansion_is_user_initiated(outline) {
                let rows = self.ivars().rows.borrow();
                let expanded = find_outline_record(&rows, item)
                    .is_some_and(|record| record.borrow().expanded);
                return expanded.into();
            }
            let events = {
                let rows = self.ivars().rows.borrow();
                let Some(record) = find_outline_record(&rows, item) else {
                    return false.into();
                };
                record.borrow_mut().expanded = true;
                record.borrow().events.clone()
            };
            // Reconciliation can mutate this outline's retained row records.
            // Release every RefCell borrow before dispatching consumer state.
            events.emit_toggle(true);
            true
        }

        #[unsafe(method(outlineView:shouldCollapseItem:))]
        fn outline_should_collapse_item(
            &self,
            outline: &AnyObject,
            item: *mut AnyObject,
        ) -> bool {
            if *self.ivars().suppress_expansion.borrow()
                || *self.ivars().suppress_split_expansion.borrow()
            {
                return true.into();
            }
            if !outline_expansion_is_user_initiated(outline) {
                let rows = self.ivars().rows.borrow();
                let collapsed = find_outline_record(&rows, item)
                    .is_some_and(|record| !record.borrow().expanded);
                return collapsed.into();
            }
            let events = {
                let rows = self.ivars().rows.borrow();
                let Some(record) = find_outline_record(&rows, item) else {
                    return false.into();
                };
                record.borrow_mut().expanded = false;
                record.borrow().events.clone()
            };
            events.emit_toggle(false);
            true
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_changed(&self, notification: &AnyObject) {
            if *self.ivars().suppress_selection.borrow() {
                return;
            }
            // SAFETY: NSTableView posts this notification with itself as object.
            let table: *mut AnyObject = unsafe { msg_send![notification, object] };
            let Some(table) = NonNull::new(table) else {
                return;
            };
            let selected: isize = unsafe { msg_send![table.as_ref(), selectedRow] };
            let Ok(index) = usize::try_from(selected) else {
                return;
            };
            let events = {
                let rows = self.ivars().rows.borrow();
                clear_record_selection(&rows);
                let outline = matches!(
                    *self.ivars().pattern.borrow(),
                    CollectionPattern::NavigationSidebar | CollectionPattern::Outline | CollectionPattern::DataTable
                );
                let selected_record = if outline {
                    // SAFETY: The notification object is the active NSOutlineView.
                    let item: *mut AnyObject = unsafe {
                        msg_send![table.as_ref(), itemAtRow: index]
                    };
                    find_outline_record(&rows, item)
                } else {
                    rows.get(index).cloned()
                };
                selected_record.map(|record| {
                    record.borrow_mut().selected = true;
                    record.borrow().events.clone()
                })
            };
            if let Some(events) = events {
                events.emit_activate();
            }
        }

        #[unsafe(method(tableView:sortDescriptorsDidChange:))]
        fn sort_descriptors_changed(&self, table: &AnyObject, _old: &AnyObject) {
            if *self.ivars().suppress_sort.borrow() {
                return;
            }
            // SAFETY: The receiver is the delegate's NSTableView. The first
            // descriptor represents Rinka's single active sort contract.
            unsafe {
                let descriptors: *mut AnyObject = msg_send![table, sortDescriptors];
                let count: usize = msg_send![descriptors, count];
                if count == 0 {
                    return;
                }
                let descriptor: *mut AnyObject = msg_send![descriptors, objectAtIndex: 0_usize];
                let key: *mut AnyObject = msg_send![descriptor, key];
                let ascending: bool = msg_send![descriptor, ascending];
                self.ivars().events.emit_sort(TableSort {
                    column_id: rust_string(key),
                    direction: if ascending {
                        SortDirection::Ascending
                    } else {
                        SortDirection::Descending
                    },
                });
            }
        }

        #[unsafe(method(tableView:pasteboardWriterForRow:))]
        fn table_pasteboard_writer_for_row(
            &self,
            _table: &AnyObject,
            row: isize,
        ) -> *mut AnyObject {
            let record = usize::try_from(row)
                .ok()
                .and_then(|index| self.ivars().rows.borrow().get(index).cloned());
            let Some(record) = record else {
                return std::ptr::null_mut();
            };
            row_pasteboard_writer(self.mtm(), &record)
        }

        #[unsafe(method(outlineView:pasteboardWriterForItem:))]
        fn outline_pasteboard_writer_for_item(
            &self,
            _outline: &AnyObject,
            item: *mut AnyObject,
        ) -> *mut AnyObject {
            let record = find_outline_record(&self.ivars().rows.borrow(), item);
            let Some(record) = record else {
                return std::ptr::null_mut();
            };
            row_pasteboard_writer(self.mtm(), &record)
        }

        #[unsafe(method(tableView:validateDrop:proposedRow:proposedDropOperation:))]
        fn table_validate_drop(
            &self,
            table: &AnyObject,
            info: *mut AnyObject,
            row: isize,
            operation: usize,
        ) -> usize {
            let proposed = (operation == TABLE_DROP_ON)
                .then(|| {
                    usize::try_from(row)
                        .ok()
                        .and_then(|index| self.ivars().rows.borrow().get(index).cloned())
                })
                .flatten();
            let Some(resolved) = self.resolve_table_drop(table, info, proposed) else {
                return DRAG_OPERATION_NONE;
            };
            // Retargeting realizes the native highlight on the actual
            // recipient: the accepted row, or the whole list.
            // SAFETY: setDropRow:dropOperation: is public NSTableView API.
            unsafe {
                match resolved.routed_record.and_then(|record| {
                    table_delegate_row_index(table, &record)
                        .and_then(|index| isize::try_from(index).ok())
                }) {
                    Some(index) => {
                        let _: () =
                            msg_send![table, setDropRow: index, dropOperation: TABLE_DROP_ON];
                    }
                    None => {
                        let _: () =
                            msg_send![table, setDropRow: -1_isize, dropOperation: TABLE_DROP_ON];
                    }
                }
            }
            resolved.operation
        }

        #[unsafe(method(tableView:acceptDrop:row:dropOperation:))]
        fn table_accept_drop(
            &self,
            table: &AnyObject,
            info: *mut AnyObject,
            row: isize,
            operation: usize,
        ) -> bool {
            let proposed = (operation == TABLE_DROP_ON)
                .then(|| {
                    usize::try_from(row)
                        .ok()
                        .and_then(|index| self.ivars().rows.borrow().get(index).cloned())
                })
                .flatten();
            let Some(resolved) = self.resolve_table_drop(table, info, proposed) else {
                return false.into();
            };
            // Every RefCell borrow is released: delivery may reconcile.
            self.deliver_table_drop(table, info, &resolved.route)
        }

        #[unsafe(method(outlineView:validateDrop:proposedItem:proposedChildIndex:))]
        fn outline_validate_drop(
            &self,
            outline: &AnyObject,
            info: *mut AnyObject,
            item: *mut AnyObject,
            _index: isize,
        ) -> usize {
            let proposed = find_outline_record(&self.ivars().rows.borrow(), item);
            let Some(resolved) = self.resolve_table_drop(outline, info, proposed) else {
                return DRAG_OPERATION_NONE;
            };
            // SAFETY: setDropItem:dropChildIndex: is public NSOutlineView API;
            // the routed item is the record's live outline identity.
            unsafe {
                match resolved.routed_record {
                    Some(record) => {
                        let identity = record.borrow().outline_identity.clone();
                        let _: () = msg_send![outline,
                            setDropItem: identity.as_object(),
                            dropChildIndex: OUTLINE_DROP_ON_ITEM_INDEX
                        ];
                    }
                    None => {
                        let _: () = msg_send![outline,
                            setDropItem: std::ptr::null::<AnyObject>(),
                            dropChildIndex: OUTLINE_DROP_ON_ITEM_INDEX
                        ];
                    }
                }
            }
            resolved.operation
        }

        #[unsafe(method(outlineView:acceptDrop:item:childIndex:))]
        fn outline_accept_drop(
            &self,
            outline: &AnyObject,
            info: *mut AnyObject,
            item: *mut AnyObject,
            _index: isize,
        ) -> bool {
            let proposed = find_outline_record(&self.ivars().rows.borrow(), item);
            let Some(resolved) = self.resolve_table_drop(outline, info, proposed) else {
                return false.into();
            };
            // Every RefCell borrow is released: delivery may reconcile.
            self.deliver_table_drop(outline, info, &resolved.route)
        }

        #[unsafe(method(clearSelectionSuppression))]
        fn clear_selection_suppression(&self) {
            *self.ivars().suppress_selection.borrow_mut() = false;
            *self.ivars().suppress_expansion.borrow_mut() = false;
            *self.ivars().suppress_sort.borrow_mut() = false;
        }

        #[unsafe(method(clearSplitExpansionSuppression))]
        fn clear_split_expansion_suppression(&self) {
            *self.ivars().suppress_split_expansion.borrow_mut() = false;
        }
    }
);

impl TableDelegate {
    fn new(
        mtm: MainThreadMarker,
        pattern: CollectionPattern,
        columns: Vec<TableColumn>,
        events: EventBindings,
        list_drop_target: Option<DropTarget>,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(TableDelegateIvars {
            rows: RefCell::new(Vec::new()),
            pattern: RefCell::new(pattern),
            columns: RefCell::new(columns),
            events,
            list_drop_target: RefCell::new(list_drop_target),
            suppress_selection: RefCell::new(false),
            suppress_expansion: RefCell::new(false),
            suppress_split_expansion: RefCell::new(false),
            suppress_sort: RefCell::new(false),
        });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }
}

fn outline_expansion_is_user_initiated(outline: &AnyObject) -> bool {
    const LEFT_MOUSE_DOWN: isize = 1;
    const LEFT_MOUSE_UP: isize = 2;
    const KEY_DOWN: isize = 10;

    // SAFETY: This is evaluated synchronously from an NSOutlineView delegate
    // callback on the AppKit main thread. currentEvent is borrowed only for
    // the duration of the callback.
    unsafe {
        let application: *mut AnyObject =
            msg_send![objc2::class!(NSApplication), sharedApplication];
        let event: *mut AnyObject = msg_send![application, currentEvent];
        let Some(event) = NonNull::new(event) else {
            // Accessibility actions do not require an NSEvent. Programmatic
            // reconciliation and split layout own independent suppression,
            // so an unsuppressed eventless request is an external action.
            return true;
        };
        let event_type: isize = msg_send![event.as_ref(), type];
        match event_type {
            LEFT_MOUSE_DOWN | LEFT_MOUSE_UP => {
                let event_window: *mut AnyObject = msg_send![event.as_ref(), window];
                let outline_window: *mut AnyObject = msg_send![outline, window];
                if !std::ptr::eq(event_window, outline_window) || event_window.is_null() {
                    return false;
                }
                let location: Point = msg_send![event.as_ref(), locationInWindow];
                let local: Point = msg_send![outline,
                    convertPoint: location,
                    fromView: std::ptr::null::<AnyObject>()
                ];
                let bounds: Rect = msg_send![outline, bounds];
                local.x >= bounds.origin.x
                    && local.y >= bounds.origin.y
                    && local.x <= bounds.origin.x + bounds.size.width
                    && local.y <= bounds.origin.y + bounds.size.height
            }
            KEY_DOWN => outline_is_first_responder(outline),
            _ => false,
        }
    }
}

unsafe fn outline_is_first_responder(outline: &AnyObject) -> bool {
    let window: *mut AnyObject = unsafe { msg_send![outline, window] };
    if window.is_null() {
        return false;
    }
    let responder: *mut AnyObject = unsafe { msg_send![window, firstResponder] };
    std::ptr::eq(responder, outline)
}

fn find_outline_record(
    rows: &[Rc<RefCell<TableRowRecord>>],
    item: *mut AnyObject,
) -> Option<Rc<RefCell<TableRowRecord>>> {
    if item.is_null() {
        return None;
    }
    for record in rows {
        if record.borrow().outline_identity.as_ptr() == item {
            return Some(record.clone());
        }
        let children = record.borrow().children.borrow().clone();
        if let Some(found) = find_outline_record(&children, item) {
            return Some(found);
        }
    }
    None
}

fn clear_record_selection(rows: &[Rc<RefCell<TableRowRecord>>]) {
    for record in rows {
        record.borrow_mut().selected = false;
        let children = record.borrow().children.borrow().clone();
        clear_record_selection(&children);
    }
}

fn set_record_table(record: &Rc<RefCell<TableRowRecord>>, table: Option<Id>) {
    *record.borrow().table.borrow_mut() = table.clone();
    let children = record.borrow().children.borrow().clone();
    for child in children {
        set_record_table(&child, table.clone());
    }
}

fn table_column_identifier(column: &TableColumn) -> String {
    format!("jp.bunko.rinka.table.{}", column.id)
}

fn table_column_index(column: *mut AnyObject, columns: &[TableColumn]) -> usize {
    let Some(column) = NonNull::new(column) else {
        return 0;
    };
    // SAFETY: The table delegate receives an NSTableColumn owned by its table.
    let identifier: *mut AnyObject = unsafe { msg_send![column.as_ref(), identifier] };
    let identifier = rust_string(identifier);
    columns
        .iter()
        .position(|candidate| table_column_identifier(candidate) == identifier)
        .unwrap_or(0)
}

fn autorelease_id(object: Id) -> *mut AnyObject {
    let pointer = object.as_ptr();
    // SAFETY: The delegate callback returns a non-owning view. Scheduling the
    // owned retain for release transfers its temporary lifetime to AppKit's
    // surrounding autorelease pool.
    unsafe {
        let _: *mut AnyObject = msg_send![object.as_object(), autorelease];
    }
    std::mem::forget(object);
    pointer
}

fn create_table_cell(
    mtm: MainThreadMarker,
    record: &TableRowRecord,
    pattern: CollectionPattern,
    column_index: usize,
) -> *mut AnyObject {
    if pattern.presents_columns() && column_index > 0 {
        let cell = create_table_value_cell(
            record
                .cells
                .get(column_index - 1)
                .map_or("", String::as_str),
        );
        attach_record_context_menu(mtm, record, cell);
        return cell;
    }
    let cell = new_view(context_menu_cell_class());
    let title = label_view(&record.title, TextRole::Body);
    let subtitle = record
        .subtitle
        .as_deref()
        .map(|value| label_view(value, TextRole::Secondary));
    let text_stack = if matches!(
        pattern,
        CollectionPattern::ContentList | CollectionPattern::EmbeddedList
    ) && let Some(subtitle) = &subtitle
    {
        let text_array = ns_array(&[title.clone(), subtitle.clone()]);
        // SAFETY: NSStackView retains the arranged text fields. AppKit owns
        // the native metrics for the vertical gap.
        unsafe {
            let pointer: *mut AnyObject = msg_send![objc2::class!(NSStackView),
                stackViewWithViews: text_array.as_object()
            ];
            let stack = Id::from_borrowed(pointer);
            let _: () = msg_send![stack.as_object(), setOrientation: 1_isize];
            let _: () = msg_send![stack.as_object(), setAlignment: 5_isize];
            stack
        }
    } else {
        // A single native text field has an intrinsic compression contract.
        // Wrapping it in a leading-aligned stack lets its arranged width
        // extend beyond a narrow table column.
        title.clone()
    };

    // A source row is normally single-line. Supporting text remains available
    // to content and table presentations where metadata is part of the row.
    if matches!(pattern, CollectionPattern::NavigationSidebar) {
        unsafe {
            let _: () = msg_send![title.as_object(), setLineBreakMode: 4_isize];
            let _: () = msg_send![title.as_object(), setUsesSingleLineMode: true];
        }
    }

    let image = record.symbol.and_then(system_image).map(|symbol| unsafe {
        let pointer: *mut AnyObject = msg_send![objc2::class!(NSImageView),
            imageViewWithImage: symbol.as_object()
        ];
        Id::from_borrowed(pointer)
    });
    let disclosure = (!matches!(pattern, CollectionPattern::NavigationSidebar) && record.disclosure)
        .then(|| system_image(Symbol::Disclosure))
        .flatten()
        .map(|symbol| unsafe {
            let pointer: *mut AnyObject = msg_send![objc2::class!(NSImageView),
                imageViewWithImage: symbol.as_object()
            ];
            Id::from_borrowed(pointer)
        });

    if matches!(
        pattern,
        CollectionPattern::NavigationSidebar
            | CollectionPattern::Outline
            | CollectionPattern::DataTable
    ) {
        // NSTableCellView owns the standard single-line image and text
        // placement for its effective row-size style. Supplying the standard
        // outlets preserves the current macOS metrics and user preference.
        unsafe {
            let _: () = msg_send![cell.as_object(), setClipsToBounds: true];
            let _: () = msg_send![cell.as_object(), addSubview: title.as_object()];
            let _: () = msg_send![cell.as_object(), setTextField: title.as_object()];
            if let Some(image) = &image {
                let _: () = msg_send![cell.as_object(), addSubview: image.as_object()];
                let _: () = msg_send![cell.as_object(), setImageView: image.as_object()];
            }
            set_string(
                cell.as_object(),
                SET_ACCESSIBILITY_LABEL,
                &record.accessibility_label,
            );
        }
        attach_record_context_menu(mtm, record, cell.as_ptr());
        return autorelease_id(cell);
    }

    // SAFETY: Every child is an NSView. Auto Layout constraints are between
    // direct descendants of the cell and use AppKit's system-spacing anchors.
    unsafe {
        let _: () = msg_send![cell.as_object(), setClipsToBounds: true];
        let _: () = msg_send![cell.as_object(), addSubview: text_stack.as_object()];
        let _: () =
            msg_send![text_stack.as_object(), setTranslatesAutoresizingMaskIntoConstraints: false];
        let _: () = msg_send![title.as_object(), setLineBreakMode: 4_isize];
        let _: () = msg_send![title.as_object(), setUsesSingleLineMode: true];
        let _: () = msg_send![cell.as_object(), setTextField: title.as_object()];
        set_string(
            cell.as_object(),
            SET_ACCESSIBILITY_LABEL,
            &record.accessibility_label,
        );

        let cell_leading: *mut AnyObject = msg_send![cell.as_object(), leadingAnchor];
        let cell_trailing: *mut AnyObject = msg_send![cell.as_object(), trailingAnchor];
        let cell_top: *mut AnyObject = msg_send![cell.as_object(), topAnchor];
        let cell_bottom: *mut AnyObject = msg_send![cell.as_object(), bottomAnchor];
        let cell_center_y: *mut AnyObject = msg_send![cell.as_object(), centerYAnchor];
        let stack_leading: *mut AnyObject = msg_send![text_stack.as_object(), leadingAnchor];
        let stack_trailing: *mut AnyObject = msg_send![text_stack.as_object(), trailingAnchor];
        let stack_top: *mut AnyObject = msg_send![text_stack.as_object(), topAnchor];
        let stack_bottom: *mut AnyObject = msg_send![text_stack.as_object(), bottomAnchor];
        let stack_center_y: *mut AnyObject = msg_send![text_stack.as_object(), centerYAnchor];

        let _ = nonnegative_dimension_constraint(msg_send![text_stack.as_object(), widthAnchor]);
        let _ = nonnegative_dimension_constraint(msg_send![text_stack.as_object(), heightAnchor]);
        let _ = equal_anchor(stack_center_y, cell_center_y);
        let _ = greater_equal_anchor(stack_top, cell_top);
        let _ = greater_equal_anchor(cell_bottom, stack_bottom);
        let _ = vertical_system_spacing_at_least_with_priority(
            stack_top,
            cell_top,
            Spacing::Compact,
            750.0,
        );
        let _ = vertical_system_spacing_at_least_with_priority(
            cell_bottom,
            stack_bottom,
            Spacing::Compact,
            750.0,
        );

        if let Some(image) = &image {
            let _: () = msg_send![cell.as_object(), addSubview: image.as_object()];
            let _: () =
                msg_send![image.as_object(), setTranslatesAutoresizingMaskIntoConstraints: false];
            let _: () = msg_send![cell.as_object(), setImageView: image.as_object()];
            let _ = nonnegative_dimension_constraint(msg_send![image.as_object(), widthAnchor]);
            let _ = nonnegative_dimension_constraint(msg_send![image.as_object(), heightAnchor]);
            let image_leading: *mut AnyObject = msg_send![image.as_object(), leadingAnchor];
            let image_trailing: *mut AnyObject = msg_send![image.as_object(), trailingAnchor];
            let _ = greater_equal_anchor(image_leading, cell_leading);
            let _ = horizontal_system_spacing_with_priority(
                image_leading,
                cell_leading,
                Spacing::Related,
                750.0,
            );
            let _ = equal_anchor(msg_send![image.as_object(), centerYAnchor], cell_center_y);
            let _ = greater_equal_anchor(stack_leading, image_trailing);
            let _ = horizontal_system_spacing_with_priority(
                stack_leading,
                image_trailing,
                Spacing::Related,
                750.0,
            );
        } else {
            let _ = greater_equal_anchor(stack_leading, cell_leading);
            let _ = horizontal_system_spacing_with_priority(
                stack_leading,
                cell_leading,
                Spacing::Related,
                750.0,
            );
        }

        if let Some(disclosure) = &disclosure {
            let _: () = msg_send![cell.as_object(), addSubview: disclosure.as_object()];
            let _: () = msg_send![disclosure.as_object(), setTranslatesAutoresizingMaskIntoConstraints: false];
            let disclosure_leading: *mut AnyObject =
                msg_send![disclosure.as_object(), leadingAnchor];
            let disclosure_trailing: *mut AnyObject =
                msg_send![disclosure.as_object(), trailingAnchor];
            let _ =
                nonnegative_dimension_constraint(msg_send![disclosure.as_object(), widthAnchor]);
            let _ =
                nonnegative_dimension_constraint(msg_send![disclosure.as_object(), heightAnchor]);
            let _ = greater_equal_anchor(disclosure_leading, stack_trailing);
            let _ = horizontal_system_spacing_at_least_with_priority(
                disclosure_leading,
                stack_trailing,
                Spacing::Related,
                750.0,
            );
            let _ = greater_equal_anchor(cell_trailing, disclosure_trailing);
            let _ = horizontal_system_spacing_with_priority(
                cell_trailing,
                disclosure_trailing,
                Spacing::Related,
                750.0,
            );
            let _ = equal_anchor(
                msg_send![disclosure.as_object(), centerYAnchor],
                cell_center_y,
            );
        } else {
            let _ = greater_equal_anchor(cell_trailing, stack_trailing);
            let _ = horizontal_system_spacing_at_least_with_priority(
                cell_trailing,
                stack_trailing,
                Spacing::Related,
                750.0,
            );
        }
    }

    attach_record_context_menu(mtm, record, cell.as_ptr());
    autorelease_id(cell)
}

/// Returns the table-cell class that also serves the accessibility
/// show-menu action.
///
/// Stock NSTableCellView does not implement `accessibilityPerformShowMenu`,
/// so an assistive client could not open a row's context menu without a
/// pointer. The subclass pops the cell's retained menu exactly like AppKit's
/// own contextual-click handling. rinka-macos binds AppKit dynamically, so
/// the subclass is registered once through the Objective-C runtime instead of
/// a static class declaration.
fn context_menu_cell_class() -> &'static objc2::runtime::AnyClass {
    static CLASS: std::sync::OnceLock<&'static objc2::runtime::AnyClass> =
        std::sync::OnceLock::new();
    CLASS.get_or_init(|| {
        let mut builder = objc2::runtime::ClassBuilder::new(
            c"RinkaContextMenuTableCellView",
            objc2::class!(NSTableCellView),
        )
        .expect("the context-menu cell class registers once per process");
        // SAFETY: The implementation matches the selector's public signature
        // (no arguments, BOOL return) declared by the NSAccessibility
        // protocol that NSView adopts.
        unsafe {
            builder.add_method(
                sel!(accessibilityPerformShowMenu),
                cell_accessibility_perform_show_menu as extern "C-unwind" fn(_, _) -> _,
            );
        }
        builder.register()
    })
}

extern "C-unwind" fn cell_accessibility_perform_show_menu(
    cell: &AnyObject,
    _command: objc2::runtime::Sel,
) -> objc2::runtime::Bool {
    // SAFETY: AppKit delivers accessibility actions to a live view on the
    // main thread. Popping the retained menu runs the same native tracking
    // loop as AppKit's contextual-click handling, anchored at the cell.
    unsafe {
        let menu: *mut AnyObject = msg_send![cell, menu];
        let Some(menu) = NonNull::new(menu) else {
            return objc2::runtime::Bool::NO;
        };
        let bounds: Rect = msg_send![cell, bounds];
        let _: bool = msg_send![menu.as_ref(),
            popUpMenuPositioningItem: std::ptr::null::<AnyObject>(),
            atLocation: bounds.origin,
            inView: cell
        ];
        objc2::runtime::Bool::YES
    }
}

/// Realizes a row's context menu on one freshly built table cell.
///
/// Cells are rebuilt whenever the row reloads, so a menu state change on the
/// record reaches the native menu through the next reload. The cell retains
/// the menu, and each menu item retains its own dispatch target.
fn attach_record_context_menu(
    mtm: MainThreadMarker,
    record: &TableRowRecord,
    cell: *mut AnyObject,
) {
    let Some(menu) = record.context_menu.as_ref() else {
        return;
    };
    let Some(cell) = NonNull::new(cell) else {
        return;
    };
    let native = build_context_ns_menu(mtm, menu, &record.events);
    // SAFETY: The cell is a live NSTableCellView and retains its menu.
    // Descendant views reach it through NSView's responder-chain bubbling,
    // and the menu-aware label class resolves it for text fields, which do
    // not bubble contextual clicks on their own.
    unsafe {
        let _: () = msg_send![cell.as_ref(), setMenu: native.as_object()];
    }
}

fn create_table_value_cell(value: &str) -> *mut AnyObject {
    let cell = new_view(context_menu_cell_class());
    let text = label_view(value, TextRole::Body);
    // SAFETY: NSTableCellView lays out its standard text outlet according to
    // the table's effective row-size style.
    unsafe {
        let _: () = msg_send![cell.as_object(), setClipsToBounds: true];
        let _: () = msg_send![cell.as_object(), addSubview: text.as_object()];
        let _: () = msg_send![text.as_object(), setLineBreakMode: 4_isize];
        let _: () = msg_send![text.as_object(), setUsesSingleLineMode: true];
        let _: () = msg_send![cell.as_object(), setTextField: text.as_object()];
        set_string(cell.as_object(), SET_ACCESSIBILITY_LABEL, value);
    }
    autorelease_id(cell)
}
