fn effective_table_columns(pattern: CollectionPattern, columns: &[TableColumn]) -> Vec<TableColumn> {
    if pattern.presents_columns() && !columns.is_empty() {
        columns.to_vec()
    } else {
        vec![TableColumn::new("primary", "Name")]
    }
}

unsafe fn install_table_columns(
    table: &AnyObject,
    pattern: CollectionPattern,
    columns: &[TableColumn],
) {
    // SAFETY: The receiver is an NSTableView. Existing columns are copied
    // before removal so mutation never invalidates the enumerated NSArray.
    let existing: *mut AnyObject = unsafe { msg_send![table, tableColumns] };
    let existing: *mut AnyObject = unsafe { msg_send![existing, copy] };
    let count: usize = unsafe { msg_send![existing, count] };
    for index in 0..count {
        let column: *mut AnyObject = unsafe { msg_send![existing, objectAtIndex: index] };
        let _: () = unsafe { msg_send![table, removeTableColumn: column] };
    }
    let _: () = unsafe { msg_send![existing, release] };

    for column in columns {
        let identifier = ns_string(&table_column_identifier(column));
        let native = unsafe {
            let allocated: *mut AnyObject = msg_send![objc2::class!(NSTableColumn), alloc];
            let pointer: *mut AnyObject = msg_send![allocated,
                initWithIdentifier: identifier.as_object()
            ];
            Id::from_owned(pointer)
        };
        set_string(native.as_object(), SET_TITLE, &column.title);
        let _: () = unsafe { msg_send![native.as_object(), setResizingMask: 3_usize] };
        if column.sortable {
            let descriptor = create_sort_descriptor(
                &column.id,
                column.sort_direction.unwrap_or(SortDirection::Ascending),
            );
            let _: () = unsafe {
                msg_send![native.as_object(), setSortDescriptorPrototype: descriptor.as_object()]
            };
        }
        let _: () = unsafe { msg_send![table, addTableColumn: native.as_object()] };
        let _: () = unsafe { msg_send![native.as_object(), sizeToFit] };
        let width: f64 = unsafe { msg_send![native.as_object(), width] };
        let _: () = unsafe { msg_send![native.as_object(), setMinWidth: width] };
    }
    let autoresizing_style = if pattern.presents_columns() {
        5_usize
    } else {
        4_usize
    };
    let _: () = unsafe { msg_send![table, setColumnAutoresizingStyle: autoresizing_style] };
}

fn create_sort_descriptor(column_id: &str, direction: SortDirection) -> Id {
    let key = ns_string(column_id);
    // SAFETY: NSSortDescriptor copies its key and retains the comparison
    // selector used by AppKit for native header state.
    unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSSortDescriptor), alloc];
        let pointer: *mut AnyObject = msg_send![allocated,
            initWithKey: key.as_object(),
            ascending: direction == SortDirection::Ascending,
            selector: sel!(localizedStandardCompare:)
        ];
        Id::from_owned(pointer)
    }
}

unsafe fn configure_table_sort(table: &AnyObject, columns: &[TableColumn]) {
    // SAFETY: The receiver is NSTableView and copies the descriptor array.
    let descriptors: Vec<Id> = columns
        .iter()
        .filter_map(|column| {
            column
                .sort_direction
                .map(|direction| create_sort_descriptor(&column.id, direction))
        })
        .collect();
    let descriptors = ns_array(&descriptors);
    let _: () = unsafe { msg_send![table, setSortDescriptors: descriptors.as_object()] };
}

unsafe fn configure_outline_column(outline: &AnyObject) {
    // SAFETY: The receiver is an NSOutlineView with one installed primary
    // column. Source-list pattern owns indentation, row height, intercell
    // spacing, selection, and background metrics for the current user setting.
    let native_columns: *mut AnyObject = unsafe { msg_send![outline, tableColumns] };
    let primary: *mut AnyObject = unsafe { msg_send![native_columns, objectAtIndex: 0_usize] };
    let _: () = unsafe { msg_send![outline, setOutlineTableColumn: primary] };
}

fn configure_collection_pattern(scroll: &AnyObject, table: &AnyObject, pattern: CollectionPattern) {
    let native_style = match pattern {
        CollectionPattern::ContentList => 2_isize,
        CollectionPattern::NavigationSidebar => 3_isize,
        CollectionPattern::Outline => 1_isize,
        CollectionPattern::DataTable => 1_isize,
        CollectionPattern::EmbeddedList => 4_isize,
    };
    // SAFETY: Values map directly to public NSTableViewStyle and
    // NSTableViewRowSizeStyle constants. The visual metrics remain AppKit-owned.
    unsafe {
        let _: () = msg_send![table, setStyle: native_style];
        let automatic_row_heights = matches!(
            pattern,
            CollectionPattern::ContentList | CollectionPattern::EmbeddedList
        );
        let _: () = msg_send![table, setUsesAutomaticRowHeights: automatic_row_heights];
        match pattern {
            CollectionPattern::NavigationSidebar => {
                let _: () = msg_send![table, setRowSizeStyle: -1_isize];
            }
            CollectionPattern::Outline | CollectionPattern::DataTable => {
                // A dense multi-column list uses AppKit's tested small table
                // metric. Source lists continue to follow the user's system
                // sidebar-size preference through the default native style.
                let _: () = msg_send![table, setRowSizeStyle: 1_isize];
            }
            CollectionPattern::ContentList | CollectionPattern::EmbeddedList => {}
        }
        let _: () = msg_send![table,
            setUsesAlternatingRowBackgroundColors: pattern.presents_columns()
        ];
        let _: () =
            msg_send![scroll, setDrawsBackground: pattern != CollectionPattern::NavigationSidebar];
        if pattern.presents_columns() {
            let _: () = msg_send![scroll, setHasHorizontalScroller: true];
            let header = new_view(objc2::class!(NSTableHeaderView));
            let _: () = msg_send![table, setHeaderView: header.as_object()];
            let columns: *mut AnyObject = msg_send![table, tableColumns];
            let column: *mut AnyObject = msg_send![columns, objectAtIndex: 0_usize];
            let header_cell: *mut AnyObject = msg_send![column, headerCell];
            let cell_size: Size = msg_send![header_cell, cellSize];
            let bounds: Rect = msg_send![table, bounds];
            let _: () = msg_send![header.as_object(), setFrame: Rect {
                origin: Point::default(),
                size: Size {
                    width: bounds.size.width,
                    height: cell_size.height,
                },
            }];
        } else {
            let _: () = msg_send![scroll, setHasHorizontalScroller: false];
            let _: () = msg_send![table, setHeaderView: std::ptr::null::<AnyObject>()];
        }
        let _: () = msg_send![scroll, tile];
    }
}

fn reload_native_list(handle: &AppKitHandle) -> Result<(), AppKitError> {
    let delegate = handle.0.table_delegate.borrow();
    let delegate = delegate
        .as_ref()
        .ok_or_else(|| AppKitError("native list has no table delegate".to_owned()))?;
    *delegate.ivars().suppress_selection.borrow_mut() = true;
    *delegate.ivars().suppress_expansion.borrow_mut() = true;
    *delegate.ivars().suppress_sort.borrow_mut() = true;
    // SAFETY: A List handle's child host is its NSTableView.
    unsafe {
        configure_table_sort(handle.host_view(), &delegate.ivars().columns.borrow());
        let _: () = msg_send![handle.host_view(), reloadData];
        let outline = matches!(
            *delegate.ivars().pattern.borrow(),
            CollectionPattern::NavigationSidebar
                | CollectionPattern::Outline
                | CollectionPattern::DataTable
        );
        if outline {
            apply_outline_expansion(handle.host_view(), &delegate.ivars().rows.borrow());
        }
        size_native_table_columns(handle.host_view(), delegate);
        let rows = delegate.ivars().rows.borrow();
        let selected = find_selected_record(&rows);
        let selected_index = selected.and_then(|record| {
            if outline {
                let row: isize = msg_send![handle.host_view(),
                    rowForItem: record.borrow().outline_identity.as_object()
                ];
                usize::try_from(row).ok()
            } else {
                rows.iter()
                    .position(|candidate| Rc::ptr_eq(candidate, &record))
            }
        });
        if let Some(index) = selected_index {
            let indexes: *mut AnyObject = msg_send![objc2::class!(NSIndexSet),
                indexSetWithIndex: index
            ];
            let _: () = msg_send![handle.host_view(),
                selectRowIndexes: indexes,
                byExtendingSelection: false
            ];
        } else {
            let _: () = msg_send![handle.host_view(),
                deselectAll: std::ptr::null::<AnyObject>()
            ];
        }
        layout_scroll_documents(handle.view());
    }
    // Selection notifications are delivered after the table completes its
    // reload. Keep programmatic synchronization silent through that run-loop
    // turn so mounting a declarative tree never invokes user actions.
    unsafe {
        let _: () = msg_send![&**delegate,
            performSelector: sel!(clearSelectionSuppression),
            withObject: std::ptr::null::<AnyObject>(),
            afterDelay: 0.0_f64
        ];
    }
    Ok(())
}

fn reapply_mounted_native_list_state(node: &MountedNode<AppKitHandle>) -> Result<(), AppKitError> {
    if node.handle().0.table_delegate.borrow().is_some() {
        reload_native_list(node.handle())?;
    }
    for child in node.children() {
        reapply_mounted_native_list_state(child)?;
    }
    Ok(())
}

fn list_registry_handles(registry: &ListRegistry) -> Vec<AppKitHandle> {
    let mut handles = Vec::new();
    registry.borrow_mut().retain(|registered| {
        let Some(inner) = registered.upgrade() else {
            return false;
        };
        handles.push(AppKitHandle(inner));
        true
    });
    handles
}

fn registered_list_handles(registries: &RefCell<Vec<ListRegistry>>) -> Vec<AppKitHandle> {
    let registries = registries.borrow();
    let mut handles = Vec::new();
    for registry in registries.iter() {
        handles.extend(list_registry_handles(registry));
    }
    handles
}

fn registered_outline_state_is_settled(registries: &RefCell<Vec<ListRegistry>>) -> bool {
    registered_list_handles(registries)
        .into_iter()
        .all(|handle| {
            let delegate = handle.0.table_delegate.borrow();
            let Some(delegate) = delegate.as_ref() else {
                return true;
            };
            if !matches!(
                *delegate.ivars().pattern.borrow(),
                CollectionPattern::NavigationSidebar
                    | CollectionPattern::Outline
                    | CollectionPattern::DataTable
            ) {
                return true;
            }
            if *delegate.ivars().suppress_split_expansion.borrow() {
                return false;
            }
            // SAFETY: Registered list handles own live NSOutlineView objects,
            // and this read occurs on AppKit's main thread.
            unsafe {
                outline_expansion_matches(handle.host_view(), &delegate.ivars().rows.borrow())
            }
        })
}

unsafe fn apply_outline_expansion(table: &AnyObject, rows: &[Rc<RefCell<TableRowRecord>>]) {
    for record in rows {
        let item = record.borrow().outline_identity.clone();
        if record.borrow().expanded {
            let _: () = unsafe { msg_send![table, expandItem: item.as_object()] };
            let children = record.borrow().children.borrow().clone();
            unsafe { apply_outline_expansion(table, &children) };
        } else {
            let _: () = unsafe { msg_send![table, collapseItem: item.as_object()] };
        }
    }
}

unsafe fn outline_expansion_matches(
    table: &AnyObject,
    rows: &[Rc<RefCell<TableRowRecord>>],
) -> bool {
    for record in rows {
        let record = record.borrow();
        let actual: bool =
            unsafe { msg_send![table, isItemExpanded: record.outline_identity.as_object()] };
        if actual != record.expanded {
            return false;
        }
        // AppKit does not expose a stable expansion state for descendants of
        // a collapsed item. Their controlled state becomes observable when
        // the ancestor is expanded, so only validate the visible branch now.
        if actual && !unsafe { outline_expansion_matches(table, &record.children.borrow()) } {
            return false;
        }
    }
    true
}

fn find_selected_record(
    rows: &[Rc<RefCell<TableRowRecord>>],
) -> Option<Rc<RefCell<TableRowRecord>>> {
    for record in rows {
        if record.borrow().selected {
            return Some(record.clone());
        }
        let children = record.borrow().children.borrow().clone();
        if let Some(selected) = find_selected_record(&children) {
            return Some(selected);
        }
    }
    None
}

unsafe fn size_native_table_columns(table: &AnyObject, delegate: &TableDelegate) {
    if *delegate.ivars().pattern.borrow() != CollectionPattern::DataTable {
        return;
    }
    // SAFETY: The receiver is the delegate's NSTableView. Widths come from
    // AppKit header and cell fitting metrics for the current declarative data.
    let columns: *mut AnyObject = unsafe { msg_send![table, tableColumns] };
    let column_count: usize = unsafe { msg_send![columns, count] };
    let intercell: Size = unsafe { msg_send![table, intercellSpacing] };
    let indentation: f64 = unsafe { msg_send![table, indentationPerLevel] };
    let rows = delegate.ivars().rows.borrow();
    unsafe { configure_primary_header_alignment(table, &rows) };
    for column_index in 0..column_count {
        let column: *mut AnyObject = unsafe { msg_send![columns, objectAtIndex: column_index] };
        let header_cell: *mut AnyObject = unsafe { msg_send![column, headerCell] };
        let header_size: Size = unsafe { msg_send![header_cell, cellSize] };
        let mut preferred_width = header_size.width;
        for row in rows.iter() {
            preferred_width = preferred_width.max(table_record_tree_width(
                row,
                column_index,
                intercell.width,
                indentation,
                0,
            ));
        }
        // Every column retains the widest current native header/cell fitting
        // width. Narrow panes scroll the table as one surface instead of
        // compressing only the primary column until adjacent values overlap.
        let _: () = unsafe { msg_send![column, setMinWidth: preferred_width] };
        let _: () = unsafe { msg_send![column, setWidth: preferred_width] };
    }
}

unsafe fn configure_primary_header_alignment(
    table: &AnyObject,
    rows: &[Rc<RefCell<TableRowRecord>>],
) {
    // SAFETY: The receiver is an outline table with at least its primary
    // column. The first native row cell and its native header cell provide the
    // two leading positions; a paragraph style carries only their measured
    // difference into the standard sortable header cell.
    let columns: *mut AnyObject = unsafe { msg_send![table, tableColumns] };
    let count: usize = unsafe { msg_send![columns, count] };
    if count == 0 {
        return;
    }
    let primary: *mut AnyObject = unsafe { msg_send![columns, objectAtIndex: 0_usize] };
    let header: *mut AnyObject = unsafe { msg_send![primary, headerCell] };
    let reference = rows
        .iter()
        .find(|row| !row.borrow().children.borrow().is_empty())
        .or_else(|| rows.iter().find(|row| row.borrow().symbol.is_some()));
    let Some(reference) = reference else {
        return;
    };
    let row: isize =
        unsafe { msg_send![table, rowForItem: reference.borrow().outline_identity.as_object()] };
    if row < 0 {
        return;
    }
    let _: () = unsafe { msg_send![table, layoutSubtreeIfNeeded] };
    let cell: *mut AnyObject =
        unsafe { msg_send![table, viewAtColumn: 0_isize, row: row, makeIfNecessary: true] };
    let Some(cell) = NonNull::new(cell) else {
        return;
    };
    let text_field: *mut AnyObject = unsafe { msg_send![cell.as_ref(), textField] };
    let Some(text_field) = NonNull::new(text_field) else {
        return;
    };
    let text_cell: *mut AnyObject = unsafe { msg_send![text_field.as_ref(), cell] };
    let text_bounds: Rect = unsafe { msg_send![text_field.as_ref(), bounds] };
    let glyph_rect: Rect = unsafe { msg_send![text_cell, titleRectForBounds: text_bounds] };
    let row_text_origin: Point =
        unsafe { msg_send![text_field.as_ref(), convertPoint: glyph_rect.origin, toView: table] };
    let header_view: *mut AnyObject = unsafe { msg_send![table, headerView] };
    let Some(header_view) = NonNull::new(header_view) else {
        return;
    };
    let header_rect: Rect = unsafe { msg_send![header_view.as_ref(), headerRectOfColumn: 0_isize] };
    let title_rect: Rect = unsafe { msg_send![header, titleRectForBounds: header_rect] };
    let intercell: Size = unsafe { msg_send![table, intercellSpacing] };
    let measured_indent = (row_text_origin.x - title_rect.origin.x - intercell.width).max(0.0);
    let paragraph = new_object(objc2::class!(NSMutableParagraphStyle));
    unsafe {
        let _: () = msg_send![paragraph.as_object(), setFirstLineHeadIndent: measured_indent];
        let _: () = msg_send![paragraph.as_object(), setHeadIndent: measured_indent];
        let _: () = msg_send![header, setImage: std::ptr::null::<AnyObject>()];
        let title: *mut AnyObject = msg_send![header, stringValue];
        let attributes: *mut AnyObject = msg_send![objc2::class!(NSDictionary),
            dictionaryWithObject: paragraph.as_object(),
            forKey: PARAGRAPH_STYLE_ATTRIBUTE_NAME
        ];
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSAttributedString), alloc];
        let attributed: *mut AnyObject = msg_send![allocated,
            initWithString: title,
            attributes: attributes
        ];
        let _: () = msg_send![header, setAttributedStringValue: attributed];
        let _: () = msg_send![attributed, release];
    }
}

fn table_record_tree_width(
    record: &Rc<RefCell<TableRowRecord>>,
    column_index: usize,
    intercell_width: f64,
    indentation: f64,
    depth: usize,
) -> f64 {
    let record = record.borrow();
    let own = table_record_width(&record, column_index, intercell_width, indentation, depth);
    record
        .children
        .borrow()
        .iter()
        .map(|child| {
            table_record_tree_width(child, column_index, intercell_width, indentation, depth + 1)
        })
        .fold(own, f64::max)
}

fn table_record_width(
    record: &TableRowRecord,
    column_index: usize,
    intercell_width: f64,
    indentation: f64,
    depth: usize,
) -> f64 {
    let text = if column_index == 0 {
        record.title.as_str()
    } else {
        record
            .cells
            .get(column_index - 1)
            .map_or("", String::as_str)
    };
    let label = label_view(text, TextRole::Body);
    let label_size: Size = unsafe { msg_send![label.as_object(), fittingSize] };
    let image_width = if column_index == 0 {
        record
            .symbol
            .and_then(system_image)
            .map_or(0.0, |image| unsafe {
                let size: Size = msg_send![image.as_object(), size];
                size.width
            })
    } else {
        0.0
    };
    let disclosure_width = if column_index == 0 && !record.children.borrow().is_empty() {
        system_image(Symbol::Disclosure).map_or(0.0, |image| unsafe {
            let size: Size = msg_send![image.as_object(), size];
            size.width + intercell_width * 0.5
        })
    } else {
        0.0
    };
    let outline_indentation = if column_index == 0 {
        indentation * depth as f64
    } else {
        0.0
    };
    label_size.width + image_width + disclosure_width + outline_indentation + intercell_width
}

unsafe fn native_table_content_width(table: &AnyObject) -> f64 {
    // SAFETY: The receiver is an NSTableView. Column and intercell metrics are
    // public AppKit properties and already include the current appearance.
    let columns: *mut AnyObject = unsafe { msg_send![table, tableColumns] };
    let count: usize = unsafe { msg_send![columns, count] };
    let mut width = 0.0;
    for index in 0..count {
        let column: *mut AnyObject = unsafe { msg_send![columns, objectAtIndex: index] };
        let column_width: f64 = unsafe { msg_send![column, width] };
        width += column_width;
    }
    let spacing: Size = unsafe { msg_send![table, intercellSpacing] };
    width + spacing.width * count.saturating_sub(1) as f64
}

/// Returns the widest visible source-row fitting width and whether every
/// visible row currently receives that width from the outline view.
unsafe fn native_source_row_fit(table: &AnyObject) -> Option<(f64, bool)> {
    let hidden: bool = unsafe { msg_send![table, isHiddenOrHasHiddenAncestor] };
    if hidden {
        return None;
    }
    let _: () = unsafe { msg_send![table, layoutSubtreeIfNeeded] };
    let table_bounds: Rect = unsafe { msg_send![table, bounds] };
    let row_count: isize = unsafe { msg_send![table, numberOfRows] };
    let mut required_width = 0.0_f64;
    let mut all_rows_fit = true;
    for row in 0..row_count {
        let cell: *mut AnyObject =
            unsafe { msg_send![table, viewAtColumn: 0_isize, row: row, makeIfNecessary: true] };
        let Some(cell) = NonNull::new(cell) else {
            continue;
        };
        let _: () = unsafe { msg_send![cell.as_ref(), layoutSubtreeIfNeeded] };
        let frame: Rect = unsafe { msg_send![cell.as_ref(), frame] };
        let cell_bounds: Rect = unsafe { msg_send![cell.as_ref(), bounds] };
        let fitting: Size = unsafe { msg_send![cell.as_ref(), fittingSize] };
        let text_field: *mut AnyObject = unsafe { msg_send![cell.as_ref(), textField] };
        let text_fit = NonNull::new(text_field).map(|text_field| {
            let text_frame: Rect = unsafe { msg_send![text_field.as_ref(), frame] };
            let intrinsic: Size = unsafe { msg_send![text_field.as_ref(), intrinsicContentSize] };
            let text_cell: *mut AnyObject = unsafe { msg_send![text_field.as_ref(), cell] };
            let cell_size: Size = unsafe { msg_send![text_cell, cellSize] };
            let intrinsic_width =
                valid_view_dimension(intrinsic.width).max(valid_view_dimension(cell_size.width));
            let trailing =
                (cell_bounds.size.width - text_frame.origin.x - text_frame.size.width).max(0.0);
            let required = text_frame.origin.x + intrinsic_width + trailing;
            let visible = text_frame
                .size
                .width
                .min((cell_bounds.size.width - text_frame.origin.x).max(0.0));
            (required, visible + 0.5 >= intrinsic_width)
        });
        let fitting_width =
            valid_view_dimension(fitting.width).max(text_fit.map_or(0.0, |(required, _)| required));
        if fitting_width == 0.0 {
            continue;
        }
        let table_trailing = table_bounds.origin.x + table_bounds.size.width;
        let outline_trailing = (table_trailing - frame.origin.x - frame.size.width).max(0.0);
        required_width = required_width.max(frame.origin.x + fitting_width + outline_trailing);
        let visible_width = frame
            .size
            .width
            .min((table_trailing - frame.origin.x).max(0.0));
        all_rows_fit &= visible_width + 0.5 >= fitting_width;
        all_rows_fit &= text_fit.is_none_or(|(_, text_fits)| text_fits);
    }
    Some((required_width, all_rows_fit))
}

fn semantic_navigation_split_context(handle: &AppKitHandle) -> Option<(AppKitHandle, bool)> {
    let mut branch = handle.clone();
    loop {
        let parent = branch.0.parent.borrow().as_ref()?.upgrade()?;
        let parent = AppKitHandle(parent);
        if parent.element_kind() == Some(ElementKind::Pattern) {
            let semantic_navigation = matches!(
                *parent.0.pattern.borrow(),
                Some(UiPattern::NavigationWorkspace { .. } | UiPattern::NavigationSplit { .. })
            );
            if semantic_navigation {
                let is_sidebar =
                    parent
                        .0
                        .presentations
                        .borrow()
                        .first()
                        .is_some_and(|presentation| {
                            presentation.source.as_ptr() == branch.0.view.as_ptr()
                        });
                return Some((parent, is_sidebar));
            }
        }
        branch = parent;
    }
}

fn semantic_navigation_split_parent(handle: &AppKitHandle) -> Option<AppKitHandle> {
    semantic_navigation_split_context(handle).map(|(parent, _)| parent)
}

fn semantic_sidebar_parent(handle: &AppKitHandle) -> Option<AppKitHandle> {
    semantic_navigation_split_context(handle)
        .and_then(|(parent, is_sidebar)| is_sidebar.then_some(parent))
}

fn refresh_semantic_sidebar_content_fit(
    sidebar_handle: &AppKitHandle,
    list_handles: &[AppKitHandle],
) {
    // SAFETY: The list registry and semantic split retain every object used
    // here on AppKit's main thread. Row, font, pane, and factory dimensions
    // all come from the currently mounted native controls.
    unsafe {
        let presentations = sidebar_handle.0.presentations.borrow();
        let Some(sidebar) = presentations.first() else {
            return;
        };
        let Some(item) = sidebar.owner.as_ref() else {
            return;
        };
        let collapsed: bool = msg_send![item.as_object(), isCollapsed];
        if collapsed {
            sidebar_handle.0.content_fit_source_width_capped.set(false);
            return;
        }
        let controller: *mut AnyObject = msg_send![item.as_object(), viewController];
        let pane: *mut AnyObject = msg_send![controller, view];
        let pane_bounds: Rect = msg_send![pane, bounds];
        let Some(split_controller) = sidebar_handle.0.auxiliaries.first().map(Id::as_object) else {
            return;
        };
        let split_view: *mut AnyObject = msg_send![split_controller, splitView];
        let window: *mut AnyObject = msg_send![split_view, window];
        let split_bounds: Rect = msg_send![split_view, bounds];
        if window.is_null() || split_bounds.size.width <= 0.0 {
            // Renderer construction connects parent handles before AppKit has
            // installed the split in an NSWindow. The post-mount pass owns the
            // first native measurement; layout during insertion is not stable.
            return;
        }
        let system_minimum = sidebar.system_minimum_thickness.unwrap_or(0.0);
        let mut maximum: f64 = msg_send![item.as_object(), maximumThickness];
        if std::env::var_os("RINKA_APPKIT_CONTENT_FIT_PROBE").is_some() {
            maximum = 600.0;
            let _: () = msg_send![item.as_object(), setMaximumThickness: maximum];
        }
        let mut content_minimum = system_minimum;
        for handle in list_handles {
            let is_source = handle
                .0
                .table_delegate
                .borrow()
                .as_ref()
                .is_some_and(|delegate| {
                    *delegate.ivars().pattern.borrow() == CollectionPattern::NavigationSidebar
                });
            if !is_source
                || !semantic_sidebar_parent(handle)
                    .is_some_and(|candidate| Rc::ptr_eq(&candidate.0, &sidebar_handle.0))
            {
                continue;
            }
            let Some((row_width, _)) = native_source_row_fit(handle.host_view()) else {
                continue;
            };
            let source_content_size: Size = msg_send![handle.view(), contentSize];
            let surrounding_width = (pane_bounds.size.width - source_content_size.width).max(0.0);
            content_minimum = content_minimum.max((row_width + surrounding_width).ceil());
        }
        let content_view = presentations
            .get(1)
            .and_then(|presentation| presentation.owner.as_ref())
            .map(|content| {
                let controller: *mut AnyObject = msg_send![content.as_object(), viewController];
                let view: *mut AnyObject = msg_send![controller, view];
                view
            });
        let simultaneous_metrics = |content_view: *mut AnyObject| {
            let content_bounds: Rect = msg_send![content_view, bounds];
            let safe_area: *mut AnyObject = msg_send![content_view, safeAreaLayoutGuide];
            let safe_frame: Rect = msg_send![safe_area, frame];
            let right_inset =
                (content_bounds.size.width - safe_frame.origin.x - safe_frame.size.width).max(0.0);
            let sidebar_outer_width = (safe_frame.origin.x - pane_bounds.size.width).max(0.0);
            let content_limit =
                (split_bounds.size.width - right_inset - sidebar_outer_width).max(0.0);
            (content_limit, sidebar_outer_width, right_inset)
        };
        let (content_limit, sidebar_outer_width, live_inspector_width) = content_view
            .map(simultaneous_metrics)
            .unwrap_or((split_bounds.size.width, 0.0, 0.0));
        let inspector = presentations
            .get(2)
            .and_then(|presentation| presentation.owner.as_ref());
        let content_required_width = presentations.get(1).map_or(0.0, |presentation| {
            let fitting: Size = msg_send![presentation.measurement.as_object(), fittingSize];
            valid_view_dimension(fitting.width)
        });
        let inspector_open_width = inspector.map_or(0.0, |inspector| {
            let minimum: f64 = msg_send![inspector.as_object(), minimumThickness];
            live_inspector_width.max(valid_view_dimension(minimum))
        });
        let co_display_limit =
            (split_bounds.size.width - sidebar_outer_width - inspector_open_width).max(0.0);
        let requested_content_minimum = content_minimum;
        let native_width_request = if maximum >= 0.0 {
            requested_content_minimum.min(maximum.max(system_minimum))
        } else {
            requested_content_minimum
        };
        // Preserve the NSWindow frame and both semantic panes before fitting
        // Source content. Even while Inspector is hidden, its factory minimum
        // remains reserved so either native toggle can be reversed without a
        // synchronous window resize. A Source row wider than this stable
        // co-display extent uses the native single-line truncation behavior.
        let pane_limit = if inspector.is_some() {
            co_display_limit
        } else {
            content_limit
        };
        let stable_content_limit = (pane_limit - content_required_width).max(system_minimum);
        let available_extent = if maximum >= 0.0 {
            stable_content_limit
                .max(system_minimum)
                .min(maximum.max(system_minimum))
        } else {
            stable_content_limit.max(system_minimum)
        };
        let minimum = requested_content_minimum
            .min(available_extent)
            .min(native_width_request)
            .max(system_minimum);
        let source_width_capped = requested_content_minimum > minimum + 0.5;
        sidebar_handle
            .0
            .content_fit_source_width_capped
            .set(source_width_capped);
        let current: f64 = msg_send![item.as_object(), minimumThickness];
        if (current - minimum).abs() > 0.5 {
            let _: () = msg_send![item.as_object(), setMinimumThickness: minimum];
        }
        if source_width_capped && pane_bounds.size.width > minimum + 0.5 {
            // Lowering minimumThickness alone does not return an already
            // allocated Source extent before the next content layout. Move
            // the native divider in the same transaction so AppKit consumes
            // that released width instead of enlarging the NSWindow.
            let divider_position = minimum + sidebar_outer_width;
            let _: () = msg_send![split_view,
                setPosition: divider_position,
                ofDividerAtIndex: 0_usize
            ];
        }
    }
}

fn refresh_semantic_sidebar_for_handle(handle: &AppKitHandle, list_handles: &[AppKitHandle]) {
    if let Some(sidebar) = semantic_navigation_split_parent(handle) {
        refresh_semantic_sidebar_content_fit(&sidebar, list_handles);
    }
}

fn refresh_all_semantic_sidebar_content_fit(list_handles: &[AppKitHandle]) {
    let mut sidebars = Vec::new();
    for handle in list_handles {
        let Some(sidebar) = semantic_sidebar_parent(handle) else {
            continue;
        };
        if !sidebars
            .iter()
            .any(|candidate: &AppKitHandle| Rc::ptr_eq(&candidate.0, &sidebar.0))
        {
            sidebars.push(sidebar);
        }
    }
    for sidebar in sidebars {
        refresh_semantic_sidebar_content_fit(&sidebar, list_handles);
    }
}
