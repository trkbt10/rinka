fn label_view(text: &str, role: TextRole) -> Id {
    let value = ns_string(text);
    // SAFETY: AppKit returns a live autoreleased label; the convenience
    // constructor instantiates the receiver class, here the menu-aware label.
    unsafe {
        let pointer: *mut AnyObject =
            msg_send![context_menu_label_class(), labelWithString: value.as_object()];
        let view = Id::from_borrowed(pointer);
        configure_label(view.as_object(), role, false);
        view
    }
}

fn apply_patch(
    mtm: MainThreadMarker,
    handle: &AppKitHandle,
    patch: &PropertyPatch,
) -> Result<(), AppKitError> {
    match patch.props() {
        Props::Label {
            text,
            role,
            selectable,
        } => {
            set_string(handle.view(), SET_STRING_VALUE, text);
            configure_label(handle.view(), *role, *selectable);
        }
        Props::Button {
            label,
            role,
            size,
            material,
            enabled,
            tooltip,
            accessibility_label,
        } => {
            set_string(handle.view(), SET_TITLE, label);
            configure_button(
                handle.view(),
                *role,
                *size,
                *material,
                *enabled,
                tooltip.as_deref(),
                accessibility_label,
            );
        }
        Props::Input {
            value,
            placeholder,
            enabled,
            accessibility_label,
            ..
        } => {
            set_string(handle.view(), SET_STRING_VALUE, value);
            set_string(handle.view(), SET_PLACEHOLDER_STRING, placeholder);
            set_string(handle.view(), SET_ACCESSIBILITY_LABEL, accessibility_label);
            // SAFETY: The receiver is an NSTextField or NSSearchField.
            unsafe {
                let _: () = msg_send![handle.view(), setEnabled: *enabled];
            }
        }
        Props::Toggle {
            label,
            value,
            size,
            enabled,
            accessibility_label,
        } => {
            set_string(handle.view(), SET_TITLE, label);
            set_string(handle.view(), SET_ACCESSIBILITY_LABEL, accessibility_label);
            // SAFETY: The receiver is an NSButton checkbox.
            unsafe {
                let _: () = msg_send![handle.view(), setState: isize::from(*value)];
                let _: () = msg_send![handle.view(), setControlSize: control_size(*size)];
                let _: () = msg_send![handle.view(), setEnabled: *enabled];
            }
        }
        Props::Progress {
            fraction,
            accessibility_label,
        } => {
            // SAFETY: The receiver is a determinate NSProgressIndicator.
            unsafe {
                let _: () = msg_send![handle.view(), setDoubleValue: *fraction];
            }
            set_string(handle.view(), SET_ACCESSIBILITY_LABEL, accessibility_label);
        }
        Props::Image {
            content,
            scaling,
            accessibility_label,
        } => {
            apply_image(handle, content, *scaling, accessibility_label)?;
        }
        Props::Separator { axis } => {
            // SAFETY: NSView autoresizing flags are a stable bitmask.
            unsafe {
                let _: () = msg_send![handle.view(), setAutoresizingMask: separator_mask(*axis)];
            }
        }
        Props::Stack {
            axis,
            spacing,
            padding,
            align,
            justify,
        } => {
            *handle.0.stack_layout.borrow_mut() = Some(StackLayout {
                axis: *axis,
                spacing: *spacing,
                padding: *padding,
                align: *align,
                justify: *justify,
            });
            refresh_stack_container_constraints(handle);
            refresh_stack_constraints(handle);
        }
        Props::Spacer {
            horizontal,
            vertical,
        } => configure_growth(handle.view(), *horizontal, *vertical),
        Props::Scroll { axis } => {
            // SAFETY: The receiver is an NSScrollView.
            unsafe {
                let _: () =
                    msg_send![handle.view(), setHasVerticalScroller: *axis == Axis::Vertical];
                let _: () =
                    msg_send![handle.view(), setHasHorizontalScroller: *axis == Axis::Horizontal];
            }
        }
        Props::Pattern { pattern } => {
            *handle.0.pattern.borrow_mut() = Some(*pattern);
            refresh_split_item_configuration(handle);
        }
        Props::List {
            accessibility_label,
            pattern,
            columns,
        } => {
            set_string(handle.view(), SET_ACCESSIBILITY_LABEL, accessibility_label);
            set_string(
                handle.host_view(),
                SET_ACCESSIBILITY_LABEL,
                accessibility_label,
            );
            if let Some(delegate) = handle.0.table_delegate.borrow().as_ref() {
                *delegate.ivars().pattern.borrow_mut() = *pattern;
                *delegate.ivars().columns.borrow_mut() =
                    effective_table_columns(*pattern, columns);
            }
            let columns = effective_table_columns(*pattern, columns);
            // SAFETY: A List handle's child host is its NSTableView.
            unsafe {
                install_table_columns(handle.host_view(), *pattern, &columns);
                if matches!(
                    *pattern,
                    CollectionPattern::NavigationSidebar
                        | CollectionPattern::Outline
                        | CollectionPattern::DataTable
                ) {
                    configure_outline_column(handle.host_view());
                }
            }
            configure_collection_pattern(handle.view(), handle.host_view(), *pattern);
            reload_native_list(handle)?;
        }
        Props::ListRow {
            title,
            subtitle,
            cells,
            role,
            expanded,
            symbol,
            selected,
            disclosure,
            accessibility_label,
        } => {
            set_string(handle.view(), SET_ACCESSIBILITY_LABEL, accessibility_label);
            if let Some(record) = handle.0.list_row.borrow().as_ref() {
                let mut record = record.borrow_mut();
                record.title.clone_from(title);
                record.subtitle.clone_from(subtitle);
                record.cells.clone_from(cells);
                record.role = *role;
                record.expanded = *expanded;
                record.symbol = *symbol;
                record.selected = *selected;
                record.disclosure = *disclosure;
                record.accessibility_label.clone_from(accessibility_label);
                record.context_menu = patch.context_menu().cloned();
            }
            if let Some(list) = list_ancestor(handle) {
                reload_native_list(&list)?;
            }
        }
        Props::Canvas {
            size,
            scene,
            accessibility_label,
        } => {
            set_string(handle.view(), SET_ACCESSIBILITY_LABEL, accessibility_label);
            let canvas = handle.0.canvas_view.borrow();
            let canvas = canvas
                .as_ref()
                .ok_or_else(|| AppKitError("canvas handle has no native canvas view".to_owned()))?;
            canvas.apply_content(*size, scene);
        }
        Props::Status { title, message, .. } => {
            if let Some(title_view) = handle.0.auxiliaries.first() {
                set_string(title_view.as_object(), SET_STRING_VALUE, title);
            }
            if let Some(message_view) = handle.0.auxiliaries.get(1) {
                set_string(message_view.as_object(), SET_STRING_VALUE, message);
            }
            // SAFETY: Status layout constraints are width then height and the
            // NSStackView recomputes its fitting size from native text metrics.
            unsafe {
                let constraints = handle.0.layout_constraints.borrow();
                for constraint in constraints.iter() {
                    let _: () = msg_send![constraint.as_object(), setActive: false];
                }
                let fitting: Size = msg_send![handle.view(), fittingSize];
                if let Some(width) = constraints.first() {
                    let _: () = msg_send![width.as_object(), setConstant: fitting.width];
                }
                if let Some(height) = constraints.get(1) {
                    let _: () = msg_send![height.as_object(), setConstant: fitting.height];
                }
                for constraint in constraints.iter() {
                    let _: () = msg_send![constraint.as_object(), setActive: true];
                }
            }
        }
    }
    if handle.element_kind() != Some(ElementKind::ListRow)
        && let Some(events) = handle.0.events.borrow().clone()
    {
        reconcile_view_context_menu(
            mtm,
            context_menu_owner_view(handle),
            &handle.0.context_menu,
            patch.context_menu(),
            &events,
        );
    }
    refresh_ancestor_stacks(handle);
    Ok(())
}
fn refresh_ancestor_stacks(handle: &AppKitHandle) {
    let mut parent = handle.0.parent.borrow().as_ref().and_then(Weak::upgrade);
    while let Some(inner) = parent {
        let ancestor = AppKitHandle(inner.clone());
        if ancestor.element_kind() == Some(ElementKind::Stack) {
            refresh_stack_container_constraints(&ancestor);
            refresh_stack_constraints(&ancestor);
        }
        parent = inner.parent.borrow().as_ref().and_then(Weak::upgrade);
    }
}

fn list_ancestor(handle: &AppKitHandle) -> Option<AppKitHandle> {
    let mut current = Some(handle.0.clone());
    while let Some(inner) = current {
        let candidate = AppKitHandle(inner.clone());
        if candidate.element_kind() == Some(ElementKind::List) {
            return Some(candidate);
        }
        current = inner.parent.borrow().as_ref().and_then(Weak::upgrade);
    }
    None
}

fn insert_child(
    parent: &AppKitHandle,
    child: &AppKitHandle,
    index: usize,
) -> Result<(), AppKitError> {
    let mut presentations = parent.0.presentations.borrow_mut();
    if index > presentations.len() {
        return Err(AppKitError(format!(
            "cannot insert AppKit child at {index}; count is {}",
            presentations.len()
        )));
    }
    let mut presentation = Presentation {
        source: child.0.view.clone(),
        source_kind: child.element_kind(),
        view: child.0.view.clone(),
        // The outer semantic view owns padding and alignment constraints. Its
        // fitting size is therefore the only complete measurement a parent
        // may use; measuring the private child host would discard system
        // spacing and force padded content into an undersized frame.
        measurement: child.0.view.clone(),
        owner: None,
        system_minimum_thickness: None,
        constraints: Vec::new(),
    };
    // SAFETY: Each branch sends container selectors to the matching AppKit class.
    unsafe {
        match parent.0.host_kind {
            HostKind::Root => {
                if index != 0 || !presentations.is_empty() {
                    return Err(AppKitError(
                        "window host accepts exactly one root view".to_owned(),
                    ));
                }
                let bounds: Rect = msg_send![parent.view(), bounds];
                let _: () = msg_send![presentation.view.as_object(), setFrame: bounds];
                let _: () = msg_send![presentation.view.as_object(), setAutoresizingMask: 18_usize];
                let _: () = msg_send![parent.view(), addSubview: presentation.view.as_object()];
            }
            HostKind::Element(ElementKind::Stack) => {
                let _: () =
                    msg_send![parent.host_view(), addSubview: presentation.view.as_object()];
            }
            HostKind::Element(ElementKind::List) => {
                let record = child.0.list_row.borrow().as_ref().cloned().ok_or_else(|| {
                    AppKitError("a native list accepts only list-row children".to_owned())
                })?;
                let delegate = parent.0.table_delegate.borrow();
                let delegate = delegate
                    .as_ref()
                    .ok_or_else(|| AppKitError("native list has no table delegate".to_owned()))?;
                delegate
                    .ivars()
                    .rows
                    .borrow_mut()
                    .insert(index, record.clone());
                set_record_table(&record, parent.0.child_host.clone());
            }
            HostKind::Element(ElementKind::ListRow) => {
                let parent_record = parent
                    .0
                    .list_row
                    .borrow()
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| AppKitError("source parent has no row record".to_owned()))?;
                let child_record = child
                    .0
                    .list_row
                    .borrow()
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| AppKitError("source child has no row record".to_owned()))?;
                parent_record
                    .borrow()
                    .children
                    .borrow_mut()
                    .insert(index, child_record.clone());
                set_record_table(&child_record, parent_record.borrow().table.borrow().clone());
            }
            HostKind::Element(ElementKind::Scroll) => {
                if index != 0 || !presentations.is_empty() {
                    return Err(AppKitError(
                        "scroll view accepts exactly one child".to_owned(),
                    ));
                }
                let content_size: Size = msg_send![parent.view(), contentSize];
                let fitting_size: Size =
                    msg_send![presentation.measurement.as_object(), fittingSize];
                let frame = Rect {
                    origin: Point::default(),
                    size: Size {
                        width: valid_view_dimension(content_size.width)
                            .max(valid_view_dimension(fitting_size.width)),
                        height: valid_view_dimension(content_size.height)
                            .max(valid_view_dimension(fitting_size.height)),
                    },
                };
                let _: () = msg_send![presentation.view.as_object(), setFrame: frame];
                let _: () = msg_send![presentation.view.as_object(), setAutoresizingMask: 2_usize];
                let _: () =
                    msg_send![parent.view(), setDocumentView: presentation.view.as_object()];
            }
            HostKind::Element(ElementKind::Pattern) => {
                let view_controller = if child.element_kind() == Some(ElementKind::Pattern) {
                    child.0.auxiliaries.first().cloned().ok_or_else(|| {
                        AppKitError("nested split has no native controller".to_owned())
                    })?
                } else {
                    let controller = new_object(objc2::class!(NSViewController));
                    let pane = create_safe_area_pane(presentation.view.as_object());
                    if split_item_receives_surplus(parent, index) {
                        // The primary content pane owns surplus window extent
                        // regardless of the current scene's intrinsic size.
                        // Sidebar and inspector factories keep their native
                        // thickness behavior; empty/status content must not
                        // turn the enclosing window into a fitting panel.
                        configure_growth(presentation.source.as_object(), true, true);
                        configure_growth(pane.as_object(), true, true);
                    }
                    let _: () = msg_send![controller.as_object(), setView: pane.as_object()];
                    presentation.view = pane;
                    controller
                };
                let item = create_native_split_item(parent, index, view_controller.as_object())?;
                let system_minimum_thickness: f64 = msg_send![item.as_object(), minimumThickness];
                configure_split_item(parent, item.as_object(), index);
                let _: () = msg_send![parent.split_controller()?, insertSplitViewItem: item.as_object(), atIndex: index];
                presentation.owner = Some(item);
                presentation.system_minimum_thickness = Some(system_minimum_thickness);
            }
            HostKind::Element(kind) => {
                return Err(AppKitError(format!("{kind:?} cannot contain children")));
            }
        }
    }
    presentations.insert(index, presentation);
    *child.0.parent.borrow_mut() = Some(Rc::downgrade(&parent.0));
    let refresh_layout = parent.element_kind() == Some(ElementKind::Stack);
    let refresh_list = list_ancestor(parent);
    drop(presentations);
    if refresh_layout {
        refresh_stack_container_constraints(parent);
        refresh_stack_constraints(parent);
    }
    if let Some(list) = refresh_list {
        reload_native_list(&list)?;
    }
    Ok(())
}

fn create_safe_area_pane(content: &AnyObject) -> Id {
    let pane = new_view(objc2::class!(NSView));
    // SAFETY: The wrapper is the view-controller root. Its content follows the
    // native safe-area guide supplied by the enclosing split-view item.
    unsafe {
        let _: () = msg_send![content, setTranslatesAutoresizingMaskIntoConstraints: false];
        let _: () = msg_send![pane.as_object(), addSubview: content];
        let safe_area: *mut AnyObject = msg_send![pane.as_object(), safeAreaLayoutGuide];
        let _ = nonnegative_dimension_constraint(msg_send![pane.as_object(), widthAnchor]);
        let _ = nonnegative_dimension_constraint(msg_send![pane.as_object(), heightAnchor]);
        let _ = nonnegative_dimension_constraint(msg_send![content, widthAnchor]);
        let _ = nonnegative_dimension_constraint(msg_send![content, heightAnchor]);
        let _ = equal_anchor(
            msg_send![content, leadingAnchor],
            msg_send![safe_area, leadingAnchor],
        );
        let _ = equal_anchor(
            msg_send![safe_area, trailingAnchor],
            msg_send![content, trailingAnchor],
        );
        let _ = equal_anchor(
            msg_send![content, topAnchor],
            msg_send![safe_area, topAnchor],
        );
        let _ = equal_anchor(
            msg_send![safe_area, bottomAnchor],
            msg_send![content, bottomAnchor],
        );
    }
    pane
}

fn create_native_split_item(
    parent: &AppKitHandle,
    index: usize,
    view_controller: &AnyObject,
) -> Result<Id, AppKitError> {
    let configuration = parent
        .0
        .pattern
        .borrow()
        .ok_or_else(|| AppKitError("split host has no semantic configuration".to_owned()))?;
    // SAFETY: Each factory takes a live view controller and returns an
    // autoreleased NSSplitViewItem with the corresponding system behavior.
    let pointer: *mut AnyObject = unsafe {
        match (configuration, index) {
            (UiPattern::NavigationSplit { .. } | UiPattern::NavigationWorkspace { .. }, 0) => {
                msg_send![objc2::class!(NSSplitViewItem),
                    sidebarWithViewController: view_controller
                ]
            }
            (UiPattern::UtilitySplit { .. }, 1) | (UiPattern::NavigationWorkspace { .. }, 2) => {
                msg_send![objc2::class!(NSSplitViewItem),
                    inspectorWithViewController: view_controller
                ]
            }
            _ => msg_send![objc2::class!(NSSplitViewItem),
                splitViewItemWithViewController: view_controller
            ],
        }
    };
    Ok(unsafe { Id::from_borrowed(pointer) })
}

fn split_item_receives_surplus(parent: &AppKitHandle, index: usize) -> bool {
    match *parent.0.pattern.borrow() {
        Some(UiPattern::NavigationSplit { .. }) => index == 1,
        Some(UiPattern::UtilitySplit { .. }) => index == 0,
        Some(UiPattern::NavigationWorkspace { .. }) => index == 1,
        None => false,
    }
}

fn configure_split_item(parent: &AppKitHandle, item: &AnyObject, index: usize) {
    let Some(pattern) = *parent.0.pattern.borrow() else {
        return;
    };
    // SAFETY: System sidebar and inspector factories own physical metrics.
    // Rinka supplies only semantic collapse policy and marks the one
    // content item whose safe area follows overlay panes.
    unsafe {
        match (pattern, index) {
            (
                UiPattern::NavigationSplit {
                    sidebar_collapsible,
                },
                0,
            ) => {
                let _: () = msg_send![item, setCanCollapse: sidebar_collapsible];
                let _: () = msg_send![item, setCanCollapseFromWindowResize: false];
                let _: () = msg_send![item,
                    setCollapseBehavior: COLLAPSE_RESIZES_SIBLINGS_WITH_FIXED_SPLIT_VIEW
                ];
            }
            (
                UiPattern::UtilitySplit {
                    inspector_collapsible,
                },
                1,
            ) => {
                let _: () = msg_send![item, setCanCollapse: inspector_collapsible];
                let _: () = msg_send![item,
                    setCollapseBehavior: COLLAPSE_RESIZES_SIBLINGS_WITH_FIXED_SPLIT_VIEW
                ];
            }
            (
                UiPattern::NavigationWorkspace {
                    sidebar_collapsible,
                    ..
                },
                0,
            ) => {
                let _: () = msg_send![item, setCanCollapse: sidebar_collapsible];
                let _: () = msg_send![item, setCanCollapseFromWindowResize: false];
                let _: () = msg_send![item,
                    setCollapseBehavior: COLLAPSE_RESIZES_SIBLINGS_WITH_FIXED_SPLIT_VIEW
                ];
            }
            (
                UiPattern::NavigationWorkspace {
                    inspector_collapsible,
                    ..
                },
                2,
            ) => {
                let _: () = msg_send![item, setCanCollapse: inspector_collapsible];
                let _: () = msg_send![item,
                    setCollapseBehavior: COLLAPSE_RESIZES_SIBLINGS_WITH_FIXED_SPLIT_VIEW
                ];
            }
            (UiPattern::NavigationSplit { .. } | UiPattern::UtilitySplit { .. }, _)
            | (UiPattern::NavigationWorkspace { .. }, 1) => {
                let _: () = msg_send![item, setAutomaticallyAdjustsSafeAreaInsets: true];
            }
            _ => {}
        }
    }
}

fn refresh_split_item_configuration(handle: &AppKitHandle) {
    let presentations = handle.0.presentations.borrow();
    for (index, presentation) in presentations.iter().enumerate() {
        if let Some(item) = &presentation.owner {
            configure_split_item(handle, item.as_object(), index);
        }
    }
}

fn remove_child(
    parent: &AppKitHandle,
    child: &AppKitHandle,
    index: usize,
) -> Result<(), AppKitError> {
    let mut presentations = parent.0.presentations.borrow_mut();
    let Some(presentation) = presentations.get(index) else {
        return Err(AppKitError(format!("no AppKit child at index {index}")));
    };
    if presentation.source.as_ptr() != child.0.view.as_ptr() {
        return Err(AppKitError(format!(
            "AppKit child mismatch at index {index}"
        )));
    }
    // SAFETY: Each branch sends removal selectors to the matching container.
    unsafe {
        for constraint in &presentation.constraints {
            let _: () = msg_send![constraint.as_object(), setActive: false];
        }
        match parent.0.host_kind {
            HostKind::Element(
                ElementKind::Stack
                | ElementKind::List
                | ElementKind::ListRow
                | ElementKind::Pattern,
            ) => {
                if parent.element_kind() == Some(ElementKind::Pattern) {
                    let item = presentation.owner.as_ref().ok_or_else(|| {
                        AppKitError("controller split child has no native item".to_owned())
                    })?;
                    let _: () = msg_send![parent.split_controller()?, removeSplitViewItem: item.as_object()];
                } else if parent.element_kind() == Some(ElementKind::Stack) {
                    let _: () = msg_send![presentation.view.as_object(), removeFromSuperview];
                }
            }
            HostKind::Element(ElementKind::Scroll) => {
                let _: () =
                    msg_send![parent.view(), setDocumentView: std::ptr::null::<AnyObject>()];
            }
            HostKind::Root => {
                let _: () = msg_send![presentation.view.as_object(), removeFromSuperview];
            }
            HostKind::Element(kind) => {
                return Err(AppKitError(format!("{kind:?} cannot remove children")));
            }
        }
    }
    if parent.element_kind() == Some(ElementKind::List) {
        let delegate = parent.0.table_delegate.borrow();
        let delegate = delegate
            .as_ref()
            .ok_or_else(|| AppKitError("native list has no table delegate".to_owned()))?;
        let record = delegate.ivars().rows.borrow_mut().remove(index);
        set_record_table(&record, None);
    } else if parent.element_kind() == Some(ElementKind::ListRow) {
        let record = parent
            .0
            .list_row
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| AppKitError("source parent has no row record".to_owned()))?
            .borrow()
            .children
            .borrow_mut()
            .remove(index);
        set_record_table(&record, None);
    }
    presentations.remove(index);
    *child.0.parent.borrow_mut() = None;
    let refresh_layout = parent.element_kind() == Some(ElementKind::Stack);
    let refresh_list = list_ancestor(parent);
    drop(presentations);
    if refresh_layout {
        refresh_stack_container_constraints(parent);
        refresh_stack_constraints(parent);
    }
    if let Some(list) = refresh_list {
        reload_native_list(&list)?;
    }
    Ok(())
}

fn move_child(
    parent: &AppKitHandle,
    child: &AppKitHandle,
    from: usize,
    to: usize,
) -> Result<(), AppKitError> {
    if from == to {
        return Ok(());
    }
    let mut presentations = parent.0.presentations.borrow_mut();
    if from >= presentations.len() || to >= presentations.len() {
        return Err(AppKitError(format!(
            "cannot move AppKit child from {from} to {to}; count is {}",
            presentations.len()
        )));
    }
    let presentation = presentations[from].clone();
    if presentation.source.as_ptr() != child.0.view.as_ptr() {
        return Err(AppKitError(format!(
            "AppKit child mismatch at index {from}"
        )));
    }
    match parent.element_kind() {
        Some(ElementKind::Stack) => {}
        Some(ElementKind::List) => {
            let delegate = parent.0.table_delegate.borrow();
            let delegate = delegate
                .as_ref()
                .ok_or_else(|| AppKitError("native list has no table delegate".to_owned()))?;
            let mut rows = delegate.ivars().rows.borrow_mut();
            let row = rows.remove(from);
            rows.insert(to, row);
        }
        Some(ElementKind::ListRow) => {
            let record = parent
                .0
                .list_row
                .borrow()
                .as_ref()
                .cloned()
                .ok_or_else(|| AppKitError("source parent has no row record".to_owned()))?;
            let binding = record.borrow();
            let mut rows = binding.children.borrow_mut();
            let row = rows.remove(from);
            rows.insert(to, row);
        }
        Some(ElementKind::Pattern) => {
            let item = presentation.owner.as_ref().ok_or_else(|| {
                AppKitError("controller split child has no native item".to_owned())
            })?;
            unsafe {
                let _: () =
                    msg_send![parent.split_controller()?, removeSplitViewItem: item.as_object()];
                let _: () = msg_send![parent.split_controller()?, insertSplitViewItem: item.as_object(), atIndex: to];
            }
        }
        kind => {
            return Err(AppKitError(format!(
                "{kind:?} does not support child moves"
            )));
        }
    }
    let moved = presentations.remove(from);
    presentations.insert(to, moved);
    let refresh_layout = parent.element_kind() == Some(ElementKind::Stack);
    let refresh_list = list_ancestor(parent);
    drop(presentations);
    if refresh_layout {
        refresh_stack_container_constraints(parent);
        refresh_stack_constraints(parent);
    }
    if let Some(list) = refresh_list {
        reload_native_list(&list)?;
    }
    Ok(())
}
