fn refresh_stack_constraints(stack: &AppKitHandle) {
    let Some(layout) = *stack.0.stack_layout.borrow() else {
        return;
    };
    {
        let mut constraints = stack.0.justification_constraints.borrow_mut();
        deactivate_constraints(&constraints);
        constraints.clear();
    }
    {
        let mut views = stack.0.justification_views.borrow_mut();
        // SAFETY: These views were created by the stack and remain attached to
        // its private layout host until the justification mode is refreshed.
        unsafe {
            for view in views.iter() {
                let _: () = msg_send![view.as_object(), removeFromSuperview];
            }
        }
        views.clear();
    }
    let mut presentations = stack.0.presentations.borrow_mut();
    for presentation in presentations.iter_mut() {
        deactivate_constraints(&presentation.constraints);
        presentation.constraints.clear();
    }
    let count = presentations.len();
    let main_orientation = match layout.axis {
        Axis::Horizontal => 0_isize,
        Axis::Vertical => 1_isize,
    };
    let main_axis_flexible = presentations.iter().any(|presentation| unsafe {
        let priority: f32 = msg_send![presentation.view.as_object(),
            contentHuggingPriorityForOrientation: main_orientation
        ];
        priority < 250.0
    });
    let flexible_spacer_indices = presentations
        .iter()
        .enumerate()
        .filter_map(|(index, presentation)| {
            if presentation.source_kind != Some(ElementKind::Spacer) {
                return None;
            }
            let priority: f32 = unsafe {
                msg_send![presentation.view.as_object(),
                    contentHuggingPriorityForOrientation: main_orientation
                ]
            };
            (priority < 250.0).then_some(index)
        })
        .collect::<Vec<_>>();
    configure_growth(
        stack.view(),
        layout.axis == Axis::Horizontal && (main_axis_flexible || layout.justify != Justify::Start),
        layout.axis == Axis::Vertical && (main_axis_flexible || layout.justify != Justify::Start),
    );
    let preferred_cross_index = if layout.align == Align::Stretch {
        None
    } else {
        presentations
            .iter()
            .enumerate()
            .map(|(index, presentation)| {
                let fitting: Size =
                    unsafe { msg_send![presentation.measurement.as_object(), fittingSize] };
                let cross = match layout.axis {
                    Axis::Horizontal => fitting.height,
                    Axis::Vertical => fitting.width,
                };
                (index, cross)
            })
            .max_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(index, _)| index)
    };
    for index in 0..count {
        let mut constraints = cross_axis_constraints(
            layout,
            stack.host_view(),
            presentations[index].view.as_object(),
        );
        constraints.extend(unsafe {
            [
                nonnegative_dimension_constraint(msg_send![
                    presentations[index].view.as_object(),
                    widthAnchor
                ]),
                nonnegative_dimension_constraint(msg_send![
                    presentations[index].view.as_object(),
                    heightAnchor
                ]),
            ]
        });
        if presentations[index].source_kind == Some(ElementKind::Spacer)
            && layout.align != Align::Stretch
        {
            // A spacer has no intrinsic cross-axis extent. Non-stretch
            // alignment supplies only its position, so complete that axis
            // without constraining the stack's flexible main-axis behavior.
            constraints.push(unsafe {
                dimension_constant_constraint(
                    match layout.axis {
                        Axis::Horizontal => {
                            msg_send![presentations[index].view.as_object(), heightAnchor]
                        }
                        Axis::Vertical => {
                            msg_send![presentations[index].view.as_object(), widthAnchor]
                        }
                    },
                    0.0,
                    1000.0,
                )
            });
        }
        let main_hugging: f32 = unsafe {
            msg_send![presentations[index].view.as_object(),
                contentHuggingPriorityForOrientation: main_orientation
            ]
        };
        if main_hugging >= 250.0 {
            let fitting: Size =
                unsafe { msg_send![presentations[index].measurement.as_object(), fittingSize] };
            let main_extent = if presentations[index].source_kind == Some(ElementKind::Separator) {
                1.0
            } else {
                match layout.axis {
                    Axis::Horizontal => fitting.width,
                    Axis::Vertical => fitting.height,
                }
            };
            if main_extent > 0.0 {
                let fitting_priority =
                    if presentations[index].source_kind == Some(ElementKind::Separator) {
                        1000.0
                    } else {
                        750.0
                    };
                constraints.push(unsafe {
                    dimension_constant_constraint(
                        match layout.axis {
                            Axis::Horizontal => {
                                msg_send![presentations[index].view.as_object(), widthAnchor]
                            }
                            Axis::Vertical => {
                                msg_send![presentations[index].view.as_object(), heightAnchor]
                            }
                        },
                        main_extent,
                        fitting_priority,
                    )
                });
            }
        }
        if preferred_cross_index == Some(index) {
            // A plain NSView has no intrinsic content size. This soft equality
            // makes a non-stretch stack hug its tallest or widest child while
            // still allowing a required parent constraint to enlarge it.
            constraints.push(unsafe {
                match layout.axis {
                    Axis::Horizontal => equal_anchor_with_priority(
                        msg_send![stack.host_view(), heightAnchor],
                        msg_send![presentations[index].view.as_object(), heightAnchor],
                        751.0,
                    ),
                    Axis::Vertical => equal_anchor_with_priority(
                        msg_send![stack.host_view(), widthAnchor],
                        msg_send![presentations[index].view.as_object(), widthAnchor],
                        751.0,
                    ),
                }
            });
        }
        // SAFETY: The main-axis anchors all belong to direct children of host.
        unsafe {
            match layout.axis {
                Axis::Horizontal => {
                    let current_leading: *mut AnyObject =
                        msg_send![presentations[index].view.as_object(), leadingAnchor];
                    if index == 0 {
                        match layout.justify {
                            Justify::Start => constraints.push(equal_anchor(
                                current_leading,
                                msg_send![stack.host_view(), leadingAnchor],
                            )),
                            Justify::Center => {}
                            Justify::End => constraints.push(greater_equal_anchor(
                                current_leading,
                                msg_send![stack.host_view(), leadingAnchor],
                            )),
                        }
                    } else {
                        let previous_trailing: *mut AnyObject =
                            msg_send![presentations[index - 1].view.as_object(), trailingAnchor];
                        constraints.push(horizontal_system_spacing_at_least_with_priority(
                            current_leading,
                            previous_trailing,
                            layout.spacing,
                            1000.0,
                        ));
                        constraints.push(horizontal_system_spacing_with_priority(
                            current_leading,
                            previous_trailing,
                            layout.spacing,
                            750.0,
                        ));
                    }
                    if index + 1 == count {
                        match layout.justify {
                            Justify::Start => constraints.push(equal_anchor(
                                msg_send![stack.host_view(), trailingAnchor],
                                msg_send![presentations[index].view.as_object(), trailingAnchor],
                            )),
                            Justify::Center => {}
                            Justify::End => constraints.push(equal_anchor(
                                msg_send![stack.host_view(), trailingAnchor],
                                msg_send![presentations[index].view.as_object(), trailingAnchor],
                            )),
                        }
                    }
                }
                Axis::Vertical => {
                    let current_top: *mut AnyObject =
                        msg_send![presentations[index].view.as_object(), topAnchor];
                    if index == 0 {
                        match layout.justify {
                            Justify::Start => constraints.push(equal_anchor(
                                current_top,
                                msg_send![stack.host_view(), topAnchor],
                            )),
                            Justify::Center => {}
                            Justify::End => constraints.push(greater_equal_anchor(
                                current_top,
                                msg_send![stack.host_view(), topAnchor],
                            )),
                        }
                    } else {
                        let previous_bottom: *mut AnyObject =
                            msg_send![presentations[index - 1].view.as_object(), bottomAnchor];
                        constraints.push(vertical_system_spacing_at_least_with_priority(
                            current_top,
                            previous_bottom,
                            layout.spacing,
                            1000.0,
                        ));
                        constraints.push(vertical_system_spacing_with_priority(
                            current_top,
                            previous_bottom,
                            layout.spacing,
                            750.0,
                        ));
                    }
                    if index + 1 == count {
                        match layout.justify {
                            Justify::Start => constraints.push(equal_anchor(
                                msg_send![stack.host_view(), bottomAnchor],
                                msg_send![presentations[index].view.as_object(), bottomAnchor],
                            )),
                            Justify::Center => {}
                            Justify::End => constraints.push(equal_anchor(
                                msg_send![stack.host_view(), bottomAnchor],
                                msg_send![presentations[index].view.as_object(), bottomAnchor],
                            )),
                        }
                    }
                }
            }
        }
        presentations[index].constraints = constraints;
    }
    if let Some((&first_index, remaining_indices)) = flexible_spacer_indices.split_first() {
        for &index in remaining_indices {
            // Multiple declarative spacers on the same axis divide the
            // available extent evenly. Low hugging alone leaves AppKit free
            // to choose any distribution and therefore produces ambiguous
            // geometry for layouts such as spacer-button-spacer.
            let constraint = unsafe {
                match layout.axis {
                    Axis::Horizontal => equal_anchor(
                        msg_send![presentations[index].view.as_object(), widthAnchor],
                        msg_send![presentations[first_index].view.as_object(), widthAnchor],
                    ),
                    Axis::Vertical => equal_anchor(
                        msg_send![presentations[index].view.as_object(), heightAnchor],
                        msg_send![presentations[first_index].view.as_object(), heightAnchor],
                    ),
                }
            };
            presentations[index].constraints.push(constraint);
        }
    }
    if count == 0 || layout.justify != Justify::Center {
        return;
    }

    let before = new_view(objc2::class!(NSView));
    let after = new_view(objc2::class!(NSView));
    // Two private, non-rendering views model equal surplus space on both sides
    // of the arranged content. This keeps centering independent of window size
    // while native fitting sizes and system spacing determine content extent.
    unsafe {
        for spacer in [&before, &after] {
            let _: () =
                msg_send![spacer.as_object(), setTranslatesAutoresizingMaskIntoConstraints: false];
            let _: () = msg_send![spacer.as_object(), setAccessibilityElement: false];
            let _: () = msg_send![stack.host_view(), addSubview: spacer.as_object()];
        }
    }
    let first = presentations[0].view.as_object();
    let last = presentations[count - 1].view.as_object();
    let mut justification_constraints = Vec::new();
    // SAFETY: The private spacer views and content views share the stack host,
    // and each constraint pairs anchors from the same axis.
    unsafe {
        match layout.axis {
            Axis::Horizontal => justification_constraints.extend([
                equal_anchor(
                    msg_send![before.as_object(), leadingAnchor],
                    msg_send![stack.host_view(), leadingAnchor],
                ),
                equal_anchor(
                    msg_send![before.as_object(), trailingAnchor],
                    msg_send![first, leadingAnchor],
                ),
                equal_anchor(
                    msg_send![after.as_object(), leadingAnchor],
                    msg_send![last, trailingAnchor],
                ),
                equal_anchor(
                    msg_send![after.as_object(), trailingAnchor],
                    msg_send![stack.host_view(), trailingAnchor],
                ),
                equal_anchor(
                    msg_send![before.as_object(), widthAnchor],
                    msg_send![after.as_object(), widthAnchor],
                ),
                nonnegative_dimension_constraint(msg_send![before.as_object(), widthAnchor]),
                nonnegative_dimension_constraint(msg_send![after.as_object(), widthAnchor]),
                equal_anchor(
                    msg_send![before.as_object(), centerYAnchor],
                    msg_send![stack.host_view(), centerYAnchor],
                ),
                dimension_constant_constraint(
                    msg_send![before.as_object(), heightAnchor],
                    0.0,
                    1000.0,
                ),
                equal_anchor(
                    msg_send![after.as_object(), centerYAnchor],
                    msg_send![stack.host_view(), centerYAnchor],
                ),
                dimension_constant_constraint(
                    msg_send![after.as_object(), heightAnchor],
                    0.0,
                    1000.0,
                ),
            ]),
            Axis::Vertical => justification_constraints.extend([
                equal_anchor(
                    msg_send![before.as_object(), topAnchor],
                    msg_send![stack.host_view(), topAnchor],
                ),
                equal_anchor(
                    msg_send![before.as_object(), bottomAnchor],
                    msg_send![first, topAnchor],
                ),
                equal_anchor(
                    msg_send![after.as_object(), topAnchor],
                    msg_send![last, bottomAnchor],
                ),
                equal_anchor(
                    msg_send![after.as_object(), bottomAnchor],
                    msg_send![stack.host_view(), bottomAnchor],
                ),
                equal_anchor(
                    msg_send![before.as_object(), heightAnchor],
                    msg_send![after.as_object(), heightAnchor],
                ),
                nonnegative_dimension_constraint(msg_send![before.as_object(), heightAnchor]),
                nonnegative_dimension_constraint(msg_send![after.as_object(), heightAnchor]),
                equal_anchor(
                    msg_send![before.as_object(), centerXAnchor],
                    msg_send![stack.host_view(), centerXAnchor],
                ),
                dimension_constant_constraint(
                    msg_send![before.as_object(), widthAnchor],
                    0.0,
                    1000.0,
                ),
                equal_anchor(
                    msg_send![after.as_object(), centerXAnchor],
                    msg_send![stack.host_view(), centerXAnchor],
                ),
                dimension_constant_constraint(
                    msg_send![after.as_object(), widthAnchor],
                    0.0,
                    1000.0,
                ),
            ]),
        }
    }
    drop(presentations);
    stack
        .0
        .justification_views
        .borrow_mut()
        .extend([before, after]);
    *stack.0.justification_constraints.borrow_mut() = justification_constraints;
}

struct ListRowConfig<'a> {
    title: &'a str,
    subtitle: Option<&'a str>,
    cells: &'a [String],
    role: ListRowRole,
    expanded: bool,
    symbol: Option<Symbol>,
    selected: bool,
    disclosure: bool,
    accessibility_label: &'a str,
}

fn create_list_row(
    _mtm: MainThreadMarker,
    events: EventBindings,
    config: ListRowConfig<'_>,
) -> Result<AppKitHandle, AppKitError> {
    let view = new_view(objc2::class!(NSView));
    set_string(
        view.as_object(),
        SET_ACCESSIBILITY_LABEL,
        config.accessibility_label,
    );
    let record = Rc::new(RefCell::new(TableRowRecord {
        title: config.title.to_owned(),
        subtitle: config.subtitle.map(ToOwned::to_owned),
        cells: config.cells.to_vec(),
        role: config.role,
        expanded: config.expanded,
        symbol: config.symbol,
        selected: config.selected,
        disclosure: config.disclosure,
        accessibility_label: config.accessibility_label.to_owned(),
        context_menu: None,
        events,
        children: RefCell::new(Vec::new()),
        outline_identity: new_object(objc2::class!(NSObject)),
        table: RefCell::new(None),
    }));
    let handle = AppKitHandle::new(
        view,
        HostKind::Element(ElementKind::ListRow),
        None,
        Vec::new(),
    );
    *handle.0.list_row.borrow_mut() = Some(record);
    Ok(handle)
}
fn create_status(
    title: &str,
    message: &str,
    tone: StatusTone,
) -> Result<AppKitHandle, AppKitError> {
    let title_view = label_view(title, TextRole::Heading);
    let message_view = label_view(message, TextRole::Secondary);
    let mut children = vec![title_view.clone(), message_view.clone()];
    let mut auxiliaries = vec![title_view.clone(), message_view.clone()];
    if tone == StatusTone::Busy {
        let spinner = new_view(objc2::class!(NSProgressIndicator));
        // SAFETY: Spinning style is native and animation is managed by AppKit.
        unsafe {
            let _: () = msg_send![spinner.as_object(), setIndeterminate: true];
            let _: () = msg_send![spinner.as_object(), setStyle: 1_usize];
            let _: () =
                msg_send![spinner.as_object(), startAnimation: std::ptr::null::<AnyObject>()];
        }
        children.insert(0, spinner.clone());
        auxiliaries.push(spinner);
    } else if tone == StatusTone::Error
        && let Some(symbol) = system_image(Symbol::Warning)
    {
        let image = unsafe {
            let pointer: *mut AnyObject = msg_send![objc2::class!(NSImageView),
                imageViewWithImage: symbol.as_object()
            ];
            Id::from_borrowed(pointer)
        };
        children.insert(0, image.clone());
        auxiliaries.push(image);
    }

    let child_array = ns_array(&children);
    let content = unsafe {
        let pointer: *mut AnyObject = msg_send![objc2::class!(NSStackView),
            stackViewWithViews: child_array.as_object()
        ];
        let stack = Id::from_borrowed(pointer);
        let _: () = msg_send![stack.as_object(), setOrientation: 1_isize];
        let _: () = msg_send![stack.as_object(), setAlignment: 9_isize];
        stack
    };
    // SAFETY: NSStackView owns the native fitting size used by a surrounding
    // semantic stack to place the complete status group as one unit.
    unsafe {
        let _: () = msg_send![message_view.as_object(), setAlignment: 1_usize];
    }
    configure_growth(content.as_object(), false, false);
    unsafe {
        let _: () = msg_send![content.as_object(),
            setContentHuggingPriority: 1000.0_f32,
            forOrientation: 1_isize
        ];
    }
    let fitting: Size = unsafe { msg_send![content.as_object(), fittingSize] };
    let size_constraints = unsafe {
        vec![
            dimension_constant_constraint(
                msg_send![content.as_object(), widthAnchor],
                fitting.width,
                999.0,
            ),
            dimension_constant_constraint(
                msg_send![content.as_object(), heightAnchor],
                fitting.height,
                999.0,
            ),
        ]
    };
    auxiliaries.push(content.clone());
    let handle = AppKitHandle::new(
        content,
        HostKind::Element(ElementKind::Status),
        None,
        auxiliaries,
    );
    *handle.0.layout_constraints.borrow_mut() = size_constraints;
    Ok(handle)
}
