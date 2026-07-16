#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourceWidthProbe {
    all_rows_fit: bool,
    all_widths_resolved: bool,
    any_width_capped: bool,
}

fn registered_visible_source_widths(registries: &RefCell<Vec<ListRegistry>>) -> SourceWidthProbe {
    let mut result = SourceWidthProbe {
        all_rows_fit: true,
        all_widths_resolved: true,
        any_width_capped: false,
    };
    for handle in registered_list_handles(registries) {
        let is_source = handle
            .0
            .table_delegate
            .borrow()
            .as_ref()
            .is_some_and(|delegate| {
                *delegate.ivars().pattern.borrow() == CollectionPattern::NavigationSidebar
            });
        if !is_source {
            continue;
        }
        let sidebar = semantic_sidebar_parent(&handle);
        let sidebar_collapsed = sidebar.as_ref().is_some_and(|sidebar| {
            let presentations = sidebar.0.presentations.borrow();
            let Some(item) = presentations
                .first()
                .and_then(|presentation| presentation.owner.as_ref())
            else {
                return false;
            };
            // SAFETY: The semantic Source list and its retained native split
            // item are queried on AppKit's main thread.
            unsafe { msg_send![item.as_object(), isCollapsed] }
        });
        if sidebar_collapsed {
            // A collapsed Source pane has no visible row-width obligation.
            // Its native content is measured again after expansion settles.
            continue;
        }
        // SAFETY: Registry handles own live NSOutlineView instances and
        // the transition probe runs on AppKit's main thread.
        let rows_fit = unsafe {
            native_source_row_fit(handle.host_view()).is_none_or(|(_, rows_fit)| rows_fit)
        };
        let width_capped =
            sidebar.is_some_and(|sidebar| sidebar.0.content_fit_source_width_capped.get());
        result.all_rows_fit &= rows_fit;
        result.all_widths_resolved &= rows_fit || width_capped;
        result.any_width_capped |= width_capped;
    }
    result
}

/// Returns the AppKit font realizing one semantic text role.
///
/// # Safety
///
/// Must be called on the AppKit main thread; the returned object follows
/// class-property lifetime conventions.
unsafe fn text_role_font(role: TextRole) -> *mut AnyObject {
    // SAFETY: Guaranteed by the caller; every arm names a public NSFont API.
    unsafe {
        match role {
            TextRole::Title => msg_send![objc2::class!(NSFont),
                preferredFontForTextStyle: FONT_TEXT_STYLE_TITLE1,
                options: std::ptr::null::<AnyObject>()
            ],
            TextRole::Heading => msg_send![objc2::class!(NSFont),
                preferredFontForTextStyle: FONT_TEXT_STYLE_HEADLINE,
                options: std::ptr::null::<AnyObject>()
            ],
            TextRole::Body => msg_send![objc2::class!(NSFont),
                preferredFontForTextStyle: FONT_TEXT_STYLE_BODY,
                options: std::ptr::null::<AnyObject>()
            ],
            TextRole::Secondary => msg_send![objc2::class!(NSFont),
                preferredFontForTextStyle: FONT_TEXT_STYLE_FOOTNOTE,
                options: std::ptr::null::<AnyObject>()
            ],
            TextRole::Monospace => {
                msg_send![objc2::class!(NSFont), monospacedSystemFontOfSize: 0.0_f64, weight: 0.0_f64]
            }
        }
    }
}

fn configure_label(view: &AnyObject, role: TextRole, selectable: bool) {
    // SAFETY: The receiver is an NSTextField label created above.
    unsafe {
        let _: () = msg_send![view, setSelectable: selectable];
        let _: () = msg_send![view, setLineBreakMode: 0_isize];
        let _: () = msg_send![view, setUsesSingleLineMode: false];
        let font = text_role_font(role);
        let _: () = msg_send![view, setFont: font];
        if role == TextRole::Secondary {
            let color: *mut AnyObject = msg_send![objc2::class!(NSColor), secondaryLabelColor];
            let _: () = msg_send![view, setTextColor: color];
        }
    }
}

fn configure_button(
    view: &AnyObject,
    role: ButtonRole,
    size: ControlSize,
    material: ButtonMaterial,
    enabled: bool,
    tooltip: Option<&str>,
    accessibility_label: &str,
) {
    // SAFETY: The receiver is an NSButton and these are public setters.
    unsafe {
        let _: () = msg_send![view, setEnabled: enabled];
        let _: () = msg_send![view, setControlSize: control_size(size)];
        let _: () = msg_send![view, setBorderShape: 0_isize];
        let bezel_style = match material {
            ButtonMaterial::Automatic => 0_isize,
            ButtonMaterial::Glass => 16_isize,
        };
        let _: () = msg_send![view, setBezelStyle: bezel_style];
        let _: () = msg_send![view,
            setContentHuggingPriority: 1000.0_f32,
            forOrientation: 1_isize
        ];
        let _: () = msg_send![view, setBezelColor: std::ptr::null::<AnyObject>()];
        let _: () = msg_send![view, setKeyEquivalent: ns_string("").as_object()];
        match role {
            ButtonRole::Standard => {
                let _: () = msg_send![view, setTintProminence: 0_isize];
            }
            ButtonRole::Primary => {
                let _: () = msg_send![view, setKeyEquivalent: ns_string("\r").as_object()];
                let color: *mut AnyObject = msg_send![objc2::class!(NSColor), controlAccentColor];
                let _: () = msg_send![view, setBezelColor: color];
                let _: () = msg_send![view, setTintProminence: 2_isize];
            }
            ButtonRole::Destructive => {
                let color: *mut AnyObject = msg_send![objc2::class!(NSColor), systemRedColor];
                let _: () = msg_send![view, setBezelColor: color];
                let _: () = msg_send![view, setTintProminence: 3_isize];
            }
            ButtonRole::Toolbar => {
                let _: () = msg_send![view, setTintProminence: 0_isize];
            }
        }
    }
    if let Some(tooltip) = tooltip {
        set_string(view, "setToolTip:", tooltip);
    }
    set_string(view, SET_ACCESSIBILITY_LABEL, accessibility_label);
}

fn configure_growth(view: &AnyObject, horizontal: bool, vertical: bool) {
    // SAFETY: NSView exposes content hugging and compression priorities.
    unsafe {
        let horizontal_priority = if horizontal { 1.0_f32 } else { 750.0_f32 };
        let vertical_priority = if vertical { 1.0_f32 } else { 750.0_f32 };
        let _: () = msg_send![view, setContentHuggingPriority: horizontal_priority, forOrientation: 0_isize];
        let _: () =
            msg_send![view, setContentHuggingPriority: vertical_priority, forOrientation: 1_isize];
    }
}

fn create_stack_handle(
    host_kind: HostKind,
    layout: StackLayout,
    auxiliaries: Vec<Id>,
) -> AppKitHandle {
    let view = new_view(objc2::class!(NSView));
    let child_host = new_view(objc2::class!(NSView));
    // SAFETY: The inner layout host is owned by the outer semantic container.
    unsafe {
        let _: () =
            msg_send![child_host.as_object(), setTranslatesAutoresizingMaskIntoConstraints: false];
        let _: () = msg_send![view.as_object(), addSubview: child_host.as_object()];
    }
    // Containers preserve their content size. Parent constraints supply the
    // cross-axis fill; only Scroll and Spacer opt into surplus main-axis room.
    configure_growth(view.as_object(), false, false);
    configure_growth(child_host.as_object(), false, false);
    let handle = AppKitHandle::new_container(view, child_host, host_kind, None, auxiliaries);
    *handle.0.stack_layout.borrow_mut() = Some(layout);
    refresh_stack_container_constraints(&handle);
    handle
}

fn activate_constraint(pointer: *mut AnyObject) -> Id {
    // SAFETY: NSLayoutAnchor returns a live constraint and activation retains
    // it in the common ancestor. Id owns an additional balanced retain.
    unsafe {
        let constraint = Id::from_borrowed(pointer);
        let _: () = msg_send![constraint.as_object(), setActive: true];
        constraint
    }
}

fn deactivate_constraints(constraints: &[Id]) {
    // SAFETY: Each object is an NSLayoutConstraint created by this backend.
    unsafe {
        for constraint in constraints {
            let _: () = msg_send![constraint.as_object(), setActive: false];
        }
    }
}

fn equal_anchor(first: *mut AnyObject, second: *mut AnyObject) -> Id {
    // SAFETY: Both anchors have the same axis and share a view hierarchy.
    unsafe { activate_constraint(msg_send![first, constraintEqualToAnchor: second]) }
}

fn equal_anchor_with_priority(first: *mut AnyObject, second: *mut AnyObject, priority: f32) -> Id {
    // SAFETY: Both anchors have the same axis and the returned constraint is
    // configured before activation.
    unsafe {
        let pointer: *mut AnyObject = msg_send![first, constraintEqualToAnchor: second];
        let constraint = Id::from_borrowed(pointer);
        let _: () = msg_send![constraint.as_object(), setPriority: priority];
        let _: () = msg_send![constraint.as_object(), setActive: true];
        constraint
    }
}

fn dimension_constant_constraint(dimension: *mut AnyObject, constant: f64, priority: f32) -> Id {
    // SAFETY: The receiver is an NSLayoutDimension and the returned constraint
    // is configured before it becomes active.
    unsafe {
        let pointer: *mut AnyObject = msg_send![dimension, constraintEqualToConstant: constant];
        let constraint = Id::from_borrowed(pointer);
        let _: () = msg_send![constraint.as_object(), setPriority: priority];
        let _: () = msg_send![constraint.as_object(), setActive: true];
        constraint
    }
}

fn nonnegative_dimension_constraint(dimension: *mut AnyObject) -> Id {
    // SAFETY: The receiver is an NSLayoutDimension and view dimensions cannot
    // become negative during split collapse or narrow-window transitions.
    unsafe {
        activate_constraint(msg_send![dimension, constraintGreaterThanOrEqualToConstant: 0.0_f64])
    }
}

fn greater_equal_anchor(first: *mut AnyObject, second: *mut AnyObject) -> Id {
    // SAFETY: Both anchors have the same axis and share a view hierarchy.
    unsafe { activate_constraint(msg_send![first, constraintGreaterThanOrEqualToAnchor: second]) }
}

fn horizontal_system_spacing_with_priority(
    after: *mut AnyObject,
    anchor: *mut AnyObject,
    spacing: Spacing,
    priority: f32,
) -> Id {
    // SAFETY: Both objects are NSLayoutXAxisAnchor instances and the
    // constraint is configured before activation.
    unsafe {
        let pointer: *mut AnyObject = msg_send![after,
            constraintEqualToSystemSpacingAfterAnchor: anchor,
            multiplier: spacing_multiplier(spacing)
        ];
        let constraint = Id::from_borrowed(pointer);
        let _: () = msg_send![constraint.as_object(), setPriority: priority];
        let _: () = msg_send![constraint.as_object(), setActive: true];
        constraint
    }
}

fn horizontal_system_spacing_at_least_with_priority(
    after: *mut AnyObject,
    anchor: *mut AnyObject,
    spacing: Spacing,
    priority: f32,
) -> Id {
    // SAFETY: Both objects are NSLayoutXAxisAnchor instances and the
    // constraint is configured before activation.
    unsafe {
        let pointer: *mut AnyObject = msg_send![after,
            constraintGreaterThanOrEqualToSystemSpacingAfterAnchor: anchor,
            multiplier: spacing_multiplier(spacing)
        ];
        let constraint = Id::from_borrowed(pointer);
        let _: () = msg_send![constraint.as_object(), setPriority: priority];
        let _: () = msg_send![constraint.as_object(), setActive: true];
        constraint
    }
}

fn vertical_system_spacing_with_priority(
    below: *mut AnyObject,
    anchor: *mut AnyObject,
    spacing: Spacing,
    priority: f32,
) -> Id {
    // SAFETY: Both objects are NSLayoutYAxisAnchor instances and the
    // constraint is configured before activation.
    unsafe {
        let pointer: *mut AnyObject = msg_send![below,
            constraintEqualToSystemSpacingBelowAnchor: anchor,
            multiplier: spacing_multiplier(spacing)
        ];
        let constraint = Id::from_borrowed(pointer);
        let _: () = msg_send![constraint.as_object(), setPriority: priority];
        let _: () = msg_send![constraint.as_object(), setActive: true];
        constraint
    }
}

fn vertical_system_spacing_at_least_with_priority(
    below: *mut AnyObject,
    anchor: *mut AnyObject,
    spacing: Spacing,
    priority: f32,
) -> Id {
    // SAFETY: Both objects are NSLayoutYAxisAnchor instances and the
    // constraint is configured before activation.
    unsafe {
        let pointer: *mut AnyObject = msg_send![below,
            constraintGreaterThanOrEqualToSystemSpacingBelowAnchor: anchor,
            multiplier: spacing_multiplier(spacing)
        ];
        let constraint = Id::from_borrowed(pointer);
        let _: () = msg_send![constraint.as_object(), setPriority: priority];
        let _: () = msg_send![constraint.as_object(), setActive: true];
        constraint
    }
}

fn stack_has_flexible_child(stack: &AppKitHandle, axis: Axis) -> bool {
    let orientation = match axis {
        Axis::Horizontal => 0_isize,
        Axis::Vertical => 1_isize,
    };
    stack
        .0
        .presentations
        .borrow()
        .iter()
        .any(|presentation| unsafe {
            // SAFETY: Presentation views are NSView instances queried on main.
            let priority: f32 = msg_send![presentation.view.as_object(),
                contentHuggingPriorityForOrientation: orientation
            ];
            priority < 250.0
        })
}

fn refresh_stack_container_constraints(stack: &AppKitHandle) {
    let Some(layout) = *stack.0.stack_layout.borrow() else {
        return;
    };
    let mut constraints = stack.0.layout_constraints.borrow_mut();
    deactivate_constraints(&constraints);
    constraints.clear();
    if stack.0.child_host.is_none() {
        return;
    }
    // SAFETY: The inner host is already attached to the outer view and all
    // corresponding anchors are compatible.
    unsafe {
        let content_guide: *mut AnyObject = if layout.padding == Some(Spacing::Content) {
            msg_send![stack.view(), layoutMarginsGuide]
        } else {
            std::ptr::null_mut()
        };
        let outer_leading: *mut AnyObject = if content_guide.is_null() {
            msg_send![stack.view(), leadingAnchor]
        } else {
            msg_send![content_guide, leadingAnchor]
        };
        let outer_trailing: *mut AnyObject = if content_guide.is_null() {
            msg_send![stack.view(), trailingAnchor]
        } else {
            msg_send![content_guide, trailingAnchor]
        };
        let outer_top: *mut AnyObject = if content_guide.is_null() {
            msg_send![stack.view(), topAnchor]
        } else {
            msg_send![content_guide, topAnchor]
        };
        let outer_bottom: *mut AnyObject = if content_guide.is_null() {
            msg_send![stack.view(), bottomAnchor]
        } else {
            msg_send![content_guide, bottomAnchor]
        };
        let inner_leading: *mut AnyObject = msg_send![stack.host_view(), leadingAnchor];
        let inner_trailing: *mut AnyObject = msg_send![stack.host_view(), trailingAnchor];
        let inner_top: *mut AnyObject = msg_send![stack.host_view(), topAnchor];
        let inner_bottom: *mut AnyObject = msg_send![stack.host_view(), bottomAnchor];
        constraints.extend([
            nonnegative_dimension_constraint(msg_send![stack.host_view(), widthAnchor]),
            nonnegative_dimension_constraint(msg_send![stack.host_view(), heightAnchor]),
        ]);
        let flexible =
            stack_has_flexible_child(stack, layout.axis) || layout.justify != Justify::Start;
        match (layout.axis, layout.padding) {
            (Axis::Vertical, Some(Spacing::Content)) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerXAnchor],
                    msg_send![stack.view(), centerXAnchor],
                ),
                equal_anchor(inner_leading, outer_leading),
                equal_anchor(outer_trailing, inner_trailing),
            ]),
            (Axis::Horizontal, Some(Spacing::Content)) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerYAnchor],
                    msg_send![stack.view(), centerYAnchor],
                ),
                equal_anchor(inner_top, outer_top),
                equal_anchor(outer_bottom, inner_bottom),
            ]),
            (Axis::Vertical, Some(padding)) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerXAnchor],
                    msg_send![stack.view(), centerXAnchor],
                ),
                horizontal_system_spacing_with_priority(
                    inner_leading,
                    outer_leading,
                    padding,
                    750.0,
                ),
                horizontal_system_spacing_with_priority(
                    outer_trailing,
                    inner_trailing,
                    padding,
                    750.0,
                ),
            ]),
            (Axis::Horizontal, Some(padding)) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerYAnchor],
                    msg_send![stack.view(), centerYAnchor],
                ),
                vertical_system_spacing_with_priority(inner_top, outer_top, padding, 750.0),
                vertical_system_spacing_with_priority(outer_bottom, inner_bottom, padding, 750.0),
            ]),
            (Axis::Vertical, None) => constraints.extend([
                equal_anchor(inner_leading, outer_leading),
                equal_anchor(outer_trailing, inner_trailing),
            ]),
            (Axis::Horizontal, None) => constraints.extend([
                equal_anchor(inner_top, outer_top),
                equal_anchor(outer_bottom, inner_bottom),
            ]),
        }
        if layout.padding.is_some() {
            constraints.extend([
                greater_equal_anchor(inner_leading, outer_leading),
                greater_equal_anchor(outer_trailing, inner_trailing),
                greater_equal_anchor(inner_top, outer_top),
                greater_equal_anchor(outer_bottom, inner_bottom),
            ]);
        }
        match (layout.axis, layout.padding, flexible, layout.justify) {
            (Axis::Vertical, Some(Spacing::Content), true, _) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerYAnchor],
                    msg_send![stack.view(), centerYAnchor],
                ),
                equal_anchor(inner_top, outer_top),
                equal_anchor(outer_bottom, inner_bottom),
            ]),
            (Axis::Vertical, Some(Spacing::Content), false, Justify::Start) => {
                constraints.extend([
                    equal_anchor(inner_top, outer_top),
                    greater_equal_anchor(outer_bottom, inner_bottom),
                ])
            }
            (Axis::Horizontal, Some(Spacing::Content), true, _) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerXAnchor],
                    msg_send![stack.view(), centerXAnchor],
                ),
                equal_anchor(inner_leading, outer_leading),
                equal_anchor(outer_trailing, inner_trailing),
            ]),
            (Axis::Horizontal, Some(Spacing::Content), false, Justify::Start) => constraints
                .extend([
                    equal_anchor(inner_leading, outer_leading),
                    greater_equal_anchor(outer_trailing, inner_trailing),
                ]),
            (Axis::Vertical, Some(padding), true, _) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerYAnchor],
                    msg_send![stack.view(), centerYAnchor],
                ),
                vertical_system_spacing_with_priority(inner_top, outer_top, padding, 750.0),
                vertical_system_spacing_with_priority(outer_bottom, inner_bottom, padding, 750.0),
            ]),
            (Axis::Vertical, Some(padding), false, Justify::Start) => constraints.extend([
                vertical_system_spacing_with_priority(inner_top, outer_top, padding, 751.0),
                vertical_system_spacing_at_least_with_priority(
                    outer_bottom,
                    inner_bottom,
                    padding,
                    750.0,
                ),
            ]),
            (Axis::Horizontal, Some(padding), true, _) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerXAnchor],
                    msg_send![stack.view(), centerXAnchor],
                ),
                horizontal_system_spacing_with_priority(
                    inner_leading,
                    outer_leading,
                    padding,
                    750.0,
                ),
                horizontal_system_spacing_with_priority(
                    outer_trailing,
                    inner_trailing,
                    padding,
                    750.0,
                ),
            ]),
            (Axis::Horizontal, Some(padding), false, Justify::Start) => constraints.extend([
                horizontal_system_spacing_with_priority(
                    inner_leading,
                    outer_leading,
                    padding,
                    751.0,
                ),
                horizontal_system_spacing_at_least_with_priority(
                    outer_trailing,
                    inner_trailing,
                    padding,
                    750.0,
                ),
            ]),
            (Axis::Vertical, None, true, _) => constraints.extend([
                equal_anchor(inner_top, outer_top),
                equal_anchor(outer_bottom, inner_bottom),
            ]),
            (Axis::Vertical, None, false, Justify::Start) => constraints.extend([
                equal_anchor(inner_top, outer_top),
                greater_equal_anchor(outer_bottom, inner_bottom),
            ]),
            (Axis::Horizontal, None, true, _) => constraints.extend([
                equal_anchor(inner_leading, outer_leading),
                equal_anchor(outer_trailing, inner_trailing),
            ]),
            (Axis::Horizontal, None, false, Justify::Start) => constraints.extend([
                equal_anchor(inner_leading, outer_leading),
                greater_equal_anchor(outer_trailing, inner_trailing),
            ]),
            (_, _, false, Justify::Center | Justify::End) => {
                unreachable!("non-start justification always enables flexible stack layout")
            }
        }
    }
}

fn cross_axis_constraints(layout: StackLayout, host: &AnyObject, child: &AnyObject) -> Vec<Id> {
    // SAFETY: Child and host are attached to the same hierarchy and the anchor
    // pair is selected from the layout axis.
    unsafe {
        let _: () = msg_send![child, setTranslatesAutoresizingMaskIntoConstraints: false];
        match (layout.axis, layout.align) {
            (Axis::Vertical, Align::Stretch) => vec![
                equal_anchor(
                    msg_send![child, leadingAnchor],
                    msg_send![host, leadingAnchor],
                ),
                equal_anchor(
                    msg_send![host, trailingAnchor],
                    msg_send![child, trailingAnchor],
                ),
            ],
            (Axis::Vertical, Align::Start) => {
                vec![
                    equal_anchor(
                        msg_send![child, leadingAnchor],
                        msg_send![host, leadingAnchor],
                    ),
                    greater_equal_anchor(
                        msg_send![host, widthAnchor],
                        msg_send![child, widthAnchor],
                    ),
                ]
            }
            (Axis::Vertical, Align::Center) => {
                vec![
                    equal_anchor(
                        msg_send![child, centerXAnchor],
                        msg_send![host, centerXAnchor],
                    ),
                    greater_equal_anchor(
                        msg_send![host, widthAnchor],
                        msg_send![child, widthAnchor],
                    ),
                ]
            }
            (Axis::Vertical, Align::End) => {
                vec![
                    equal_anchor(
                        msg_send![host, trailingAnchor],
                        msg_send![child, trailingAnchor],
                    ),
                    greater_equal_anchor(
                        msg_send![host, widthAnchor],
                        msg_send![child, widthAnchor],
                    ),
                ]
            }
            (Axis::Horizontal, Align::Stretch) => vec![
                equal_anchor(msg_send![child, topAnchor], msg_send![host, topAnchor]),
                equal_anchor(
                    msg_send![host, bottomAnchor],
                    msg_send![child, bottomAnchor],
                ),
            ],
            (Axis::Horizontal, Align::Start) => {
                vec![
                    equal_anchor(msg_send![child, topAnchor], msg_send![host, topAnchor]),
                    greater_equal_anchor(
                        msg_send![host, heightAnchor],
                        msg_send![child, heightAnchor],
                    ),
                ]
            }
            (Axis::Horizontal, Align::Center) => {
                vec![
                    equal_anchor(
                        msg_send![child, centerYAnchor],
                        msg_send![host, centerYAnchor],
                    ),
                    greater_equal_anchor(
                        msg_send![host, heightAnchor],
                        msg_send![child, heightAnchor],
                    ),
                ]
            }
            (Axis::Horizontal, Align::End) => {
                vec![
                    equal_anchor(
                        msg_send![host, bottomAnchor],
                        msg_send![child, bottomAnchor],
                    ),
                    greater_equal_anchor(
                        msg_send![host, heightAnchor],
                        msg_send![child, heightAnchor],
                    ),
                ]
            }
        }
    }
}
