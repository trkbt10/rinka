fn set_string(receiver: &AnyObject, selector_name: &str, value: &str) {
    let value = ns_string(value);
    // SAFETY: Every match arm names a public one-NSString-argument AppKit setter.
    unsafe {
        match selector_name {
            SET_STRING_VALUE => {
                let _: () = msg_send![receiver, setStringValue: value.as_object()];
            }
            SET_PLACEHOLDER_STRING => {
                let _: () = msg_send![receiver, setPlaceholderString: value.as_object()];
            }
            SET_ACCESSIBILITY_LABEL => {
                let _: () = msg_send![receiver, setAccessibilityLabel: value.as_object()];
            }
            SET_TITLE => {
                let _: () = msg_send![receiver, setTitle: value.as_object()];
            }
            "setToolTip:" => {
                let _: () = msg_send![receiver, setToolTip: value.as_object()];
            }
            "setLabel:" => {
                let _: () = msg_send![receiver, setLabel: value.as_object()];
            }
            "setPaletteLabel:" => {
                let _: () = msg_send![receiver, setPaletteLabel: value.as_object()];
            }
            _ => panic!("unregistered AppKit string setter: {selector_name}"),
        }
    }
}

const fn control_size(size: ControlSize) -> usize {
    match size {
        ControlSize::Regular => 0,
        ControlSize::Small => 1,
        ControlSize::Mini => 2,
        ControlSize::Large => 3,
        ControlSize::ExtraLarge => 4,
    }
}

const fn separator_mask(axis: Axis) -> usize {
    match axis {
        Axis::Horizontal => 2,
        Axis::Vertical => 16,
    }
}

/// Semantic spacing is expressed in ordered multiples of AppKit's contextual
/// system spacing. Content insets are resolved through layoutMarginsGuide.
const fn spacing_multiplier(spacing: Spacing) -> f64 {
    match spacing {
        Spacing::Joined => 0.0,
        Spacing::Compact => 0.5,
        Spacing::Related => 1.0,
        Spacing::Section => 2.0,
        Spacing::Content => 1.0,
    }
}

fn system_image(symbol: Symbol) -> Option<Id> {
    system_image_named(match symbol {
        Symbol::Back => "chevron.left",
        Symbol::Forward => "chevron.right",
        Symbol::Add => "plus",
        Symbol::Refresh => "arrow.clockwise",
        Symbol::Search => "magnifyingglass",
        Symbol::Home => "house",
        Symbol::Folder => "folder",
        Symbol::File => "doc",
        Symbol::Code => "chevron.left.forwardslash.chevron.right",
        Symbol::Image => "photo",
        Symbol::Terminal => "terminal",
        Symbol::Settings => "gearshape",
        Symbol::More => "ellipsis",
        Symbol::Grid => "square.grid.2x2",
        Symbol::List => "list.bullet",
        Symbol::Columns => "rectangle.split.3x1",
        Symbol::Gallery => "square.stack",
        Symbol::Sort => "arrow.up.arrow.down",
        Symbol::Share => "square.and.arrow.up",
        Symbol::Tag => "tag",
        Symbol::Disclosure => "chevron.right",
        Symbol::Warning => "exclamationmark.triangle",
    })
}

fn system_image_named(symbol_name: &str) -> Option<Id> {
    let name = ns_string(symbol_name);
    // SAFETY: imageWithSystemSymbolName returns nil only when the OS lacks the symbol.
    unsafe {
        let pointer: *mut AnyObject = msg_send![objc2::class!(NSImage),
            imageWithSystemSymbolName: name.as_object(),
            accessibilityDescription: std::ptr::null::<AnyObject>()
        ];
        NonNull::new(pointer).map(|pointer| Id::from_borrowed(pointer.as_ptr()))
    }
}

const fn native_toolbar_group_display(display: ToolbarGroupDisplay) -> isize {
    match display {
        ToolbarGroupDisplay::Automatic => 0,
        ToolbarGroupDisplay::Expanded => 1,
        ToolbarGroupDisplay::Collapsed => 2,
    }
}

unsafe fn layout_scroll_documents(view: &AnyObject) {
    // SAFETY: The traversal stays on the AppKit main thread and only inspects
    // NSView descendants. Scroll document geometry is updated after the
    // enclosing window has its final initial size.
    let is_scroll: bool = unsafe { msg_send![view, isKindOfClass: objc2::class!(NSScrollView)] };
    if is_scroll {
        let document: *mut AnyObject = unsafe { msg_send![view, documentView] };
        // NSTextView documents own their geometry through the standard
        // vertically-resizable text-container recipe; imposing a frame here
        // would fight native text layout and reset the scroll position.
        let is_text_document = unsafe {
            NonNull::new(document).is_some_and(|document| {
                msg_send![document.as_ref(), isKindOfClass: objc2::class!(NSTextView)]
            })
        };
        if let Some(document) = NonNull::new(document).filter(|_| !is_text_document) {
            let content_size: Size = unsafe { msg_send![view, contentSize] };
            let fitting_size: Size = unsafe { msg_send![document.as_ref(), fittingSize] };
            let content_size = Size {
                width: valid_view_dimension(content_size.width),
                height: valid_view_dimension(content_size.height),
            };
            let fitting_size = Size {
                width: valid_view_dimension(fitting_size.width),
                height: valid_view_dimension(fitting_size.height),
            };
            let vertical: bool = unsafe { msg_send![view, hasVerticalScroller] };
            let is_table: bool =
                unsafe { msg_send![document.as_ref(), isKindOfClass: objc2::class!(NSTableView)] };
            let document_width = if is_table {
                valid_view_dimension(unsafe { native_table_content_width(document.as_ref()) })
            } else {
                fitting_size.width
            };
            let frame = Rect {
                origin: Point::default(),
                size: Size {
                    width: if vertical {
                        content_size.width.max(document_width)
                    } else {
                        document_width
                    },
                    height: if vertical {
                        if is_table {
                            // NSTableView owns row placement and selection.
                            // Filling a short viewport leaves its empty region
                            // after the rows without changing native row metrics.
                            content_size.height.max(fitting_size.height)
                        } else {
                            // Stack documents keep their content height so
                            // surplus room is not distributed into fixed rows.
                            fitting_size.height
                        }
                    } else {
                        content_size.height.max(fitting_size.height)
                    },
                },
            };
            unsafe {
                let _: () = msg_send![document.as_ref(), setFrame: frame];
                let _: () = msg_send![document.as_ref(), layoutSubtreeIfNeeded];
                let clip: *mut AnyObject = msg_send![view, contentView];
                let origin = Point {
                    x: 0.0,
                    // NSTableView has its own row coordinate semantics. Other
                    // NSView documents use the default non-flipped coordinates.
                    y: if vertical && !is_table {
                        frame.size.height - content_size.height
                    } else {
                        0.0
                    },
                };
                if !is_table {
                    let _: () = msg_send![clip, scrollToPoint: origin];
                    let _: () = msg_send![view, reflectScrolledClipView: clip];
                }
            }
        }
    }

    let subviews: *mut AnyObject = unsafe { msg_send![view, subviews] };
    let count: usize = unsafe { msg_send![subviews, count] };
    for index in 0..count {
        let child: *mut AnyObject = unsafe { msg_send![subviews, objectAtIndex: index] };
        if let Some(child) = NonNull::new(child) {
            unsafe { layout_scroll_documents(child.as_ref()) };
        }
    }
}

fn valid_view_dimension(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn new_object(class: &objc2::runtime::AnyClass) -> Id {
    // SAFETY: Every caller passes an NSObject subclass with init.
    unsafe {
        let allocated: *mut AnyObject = msg_send![class, alloc];
        let pointer: *mut AnyObject = msg_send![allocated, init];
        Id::from_owned(pointer)
    }
}
