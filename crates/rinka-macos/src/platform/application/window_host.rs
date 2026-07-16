fn rect_matches(left: Rect, right: Rect) -> bool {
    const TOLERANCE: f64 = 0.01;
    (left.origin.x - right.origin.x).abs() <= TOLERANCE
        && (left.origin.y - right.origin.y).abs() <= TOLERANCE
        && (left.size.width - right.size.width).abs() <= TOLERANCE
        && (left.size.height - right.size.height).abs() <= TOLERANCE
}

fn rect_size_matches(rect: Rect, size: Size) -> bool {
    const TOLERANCE: f64 = 0.01;
    (rect.size.width - size.width).abs() <= TOLERANCE
        && (rect.size.height - size.height).abs() <= TOLERANCE
}

unsafe fn split_item_collapsed(controller: *mut AnyObject, index: usize) -> bool {
    // SAFETY: The probe is enabled only for the three-item Workspace fixture.
    let items: *mut AnyObject = unsafe { msg_send![controller, splitViewItems] };
    let item: *mut AnyObject = unsafe { msg_send![items, objectAtIndex: index] };
    unsafe { msg_send![item, isCollapsed] }
}

unsafe fn set_split_item_collapsed(controller: *mut AnyObject, index: usize, collapsed: bool) {
    // SAFETY: The probe is enabled only for the three-item Workspace fixture.
    let items: *mut AnyObject = unsafe { msg_send![controller, splitViewItems] };
    let item: *mut AnyObject = unsafe { msg_send![items, objectAtIndex: index] };
    let _: () = unsafe { msg_send![item, setCollapsed: collapsed] };
}

type BuiltWindow = (
    Id,
    WindowRuntime<AppKitBackend>,
    Retained<ToolbarDelegate>,
    ListRegistry,
    Vec<Id>,
);

fn build_window(
    mtm: MainThreadMarker,
    spec: &WindowSpec,
    split_restore_pending: Rc<Cell<bool>>,
) -> Result<BuiltWindow, AppKitError> {
    let frame = Rect {
        origin: Point::default(),
        size: Size {
            width: spec.initial_size.width,
            height: spec.initial_size.height,
        },
    };
    let class = match spec.kind {
        WindowKind::Panel(_) => objc2::class!(NSPanel),
        WindowKind::Main | WindowKind::Preferences => objc2::class!(NSWindow),
    };
    let full_height_content = !matches!(spec.kind, WindowKind::Panel(_));
    let style_mask =
        1_usize | 2_usize | 4_usize | 8_usize | if full_height_content { 32768_usize } else { 0 };
    // SAFETY: initWithContentRect is the designated NSWindow/NSPanel initializer.
    let window = unsafe {
        let allocated: *mut AnyObject = msg_send![class, alloc];
        let pointer: *mut AnyObject = msg_send![allocated,
            initWithContentRect: frame,
            styleMask: style_mask,
            backing: 2_usize,
            defer: false
        ];
        Id::from_owned(pointer)
    };
    set_string(window.as_object(), SET_TITLE, &spec.title);
    // SAFETY: Window geometry and Tahoe titlebar properties are public AppKit API.
    unsafe {
        let _: () = msg_send![window.as_object(), setReleasedWhenClosed: false];
        let _: () = msg_send![window.as_object(), setContentMinSize: Size {
            width: spec.minimum_size.width,
            height: spec.minimum_size.height,
        }];
        let _: () =
            msg_send![window.as_object(), setTitlebarAppearsTransparent: full_height_content];
        if full_height_content {
            let _: () = msg_send![window.as_object(), setToolbarStyle: 3_isize];
        }
    }

    if let WindowKind::Panel(behavior) = spec.kind {
        configure_panel(window.as_object(), behavior);
    }

    // SAFETY: Every NSWindow created above has a content view.
    let content: *mut AnyObject = unsafe { msg_send![window.as_object(), contentView] };
    // SAFETY: contentView is retained by its window; the backend takes another retain.
    let content = unsafe { Id::from_borrowed(content) };
    let list_registry = Rc::new(RefCell::new(Vec::new()));
    let renderer = Renderer::new(AppKitBackend::new(
        content.clone(),
        mtm,
        list_registry.clone(),
        split_restore_pending,
    ));
    // Dialog requests raised by this window's component present as sheets
    // on this window (window-modal, never app-modal). The registry unions
    // the pasteboard clipboard service with the per-window dialog presenter.
    let services =
        pasteboard_platform_services().with_dialog_service(AppKitWindowDialogService {
            window: window.clone(),
        });
    let runtime = WindowRuntime::mount(renderer, spec.content.clone(), services)
        .map_err(|error| AppKitError(error.to_string()))?;
    runtime.with_renderer(|renderer| {
        if let Some(root) = renderer.mounted() {
            refresh_mounted_stacks(root);
        }
    });
    let initial_content_size = Size {
        width: spec.initial_size.width,
        height: spec.initial_size.height,
    };
    let toolbar_delegate =
        runtime.with_renderer(|renderer| install_toolbar(window.as_object(), spec, mtm, renderer));
    let initial_extent_constraints = runtime.with_renderer(|renderer| {
        install_root_content_controller(window.as_object(), renderer, initial_content_size)
    })?;
    // Installing the retained content-view controller and toolbar allows
    // AppKit to resolve their native fitting sizes. Reassert the declarative
    // content size after that ownership graph is complete so Ready, Empty,
    // Busy, and Error cannot acquire different top-level window widths from
    // their scene-specific fitting content.
    unsafe {
        let _: () = msg_send![window.as_object(), setContentSize: Size {
            width: spec.initial_size.width,
            height: spec.initial_size.height,
        }];
    }
    // The content view may have changed when the retained native controller
    // became the window's content-view controller.
    let content: *mut AnyObject = unsafe { msg_send![window.as_object(), contentView] };
    let content = unsafe { Id::from_borrowed(content) };

    // SAFETY: Show and place the fully-rendered native window. The application
    // delegate assigns key status after every auxiliary panel is ordered.
    unsafe {
        let _: () = msg_send![window.as_object(), center];
        let _: () = msg_send![window.as_object(), orderFront: std::ptr::null::<AnyObject>()];
        let _: () = msg_send![content.as_object(), layoutSubtreeIfNeeded];
        layout_scroll_documents(content.as_object());
        let _: () = msg_send![content.as_object(), layoutSubtreeIfNeeded];
    }
    runtime.with_renderer(|renderer| {
        let root = renderer
            .mounted()
            .ok_or_else(|| AppKitError("window renderer has no mounted root".to_owned()))?;
        reapply_mounted_native_list_state(root)
    })?;
    let mounted_lists = list_registry_handles(&list_registry);
    refresh_all_semantic_sidebar_content_fit(&mounted_lists);
    unsafe {
        let _: () = msg_send![content.as_object(), layoutSubtreeIfNeeded];
        // Semantic Source fitting can move a native split divider after the
        // controller is installed. That transaction must consume the existing
        // content extent rather than adopting a scene-specific fitting width.
        let _: () = msg_send![window.as_object(), setContentSize: Size {
            width: spec.initial_size.width,
            height: spec.initial_size.height,
        }];
        let _: () = msg_send![content.as_object(), layoutSubtreeIfNeeded];
    }
    Ok((
        window,
        runtime,
        toolbar_delegate,
        list_registry,
        initial_extent_constraints,
    ))
}

fn install_root_content_controller(
    window: &AnyObject,
    renderer: &Renderer<AppKitBackend>,
    initial_content_size: Size,
) -> Result<Vec<Id>, AppKitError> {
    let root = renderer
        .mounted()
        .ok_or_else(|| AppKitError("window renderer has no mounted root".to_owned()))?;
    let handle = root.handle();
    // SAFETY: The temporary renderer host owns the mounted root only until a
    // native content-view controller takes over below.
    unsafe {
        let _: () = msg_send![handle.view(), removeFromSuperview];
    }
    let controller = if matches!(handle.element_kind(), Some(ElementKind::Pattern)) {
        handle
            .0
            .auxiliaries
            .first()
            .cloned()
            .ok_or_else(|| AppKitError("root split has no native controller".to_owned()))?
    } else {
        let controller = new_object(objc2::class!(NSViewController));
        let pane = create_safe_area_pane(handle.view());
        configure_growth(pane.as_object(), true, true);
        // SAFETY: The mounted root is retained inside a native container. The
        // controller owns that container while its child retains Rinka's
        // declarative extent independently from NSWindow's contentView frame.
        unsafe {
            let _: () = msg_send![controller.as_object(), setView: pane.as_object()];
        }
        controller
    };
    // SAFETY: NSWindow retains its content-view controller. Removing the root
    // from the temporary renderer host prevents dual view ownership. Declaring
    // the controller's intended content extent before attachment prevents
    // AppKit from deriving the top-level window size from scene-specific
    // intrinsic content during the ownership transfer.
    unsafe {
        let _: () =
            msg_send![controller.as_object(), setPreferredContentSize: initial_content_size];
        let _: () = msg_send![handle.view(), setFrameSize: initial_content_size];
        let _: () = msg_send![window, setContentViewController: controller.as_object()];
        let _: () = msg_send![window, setContentSize: initial_content_size];
        let content: *mut AnyObject = msg_send![window, contentView];
        let _: () = msg_send![content, setFrameSize: initial_content_size];
        // AppKit replaces the root ownership graph while assigning the
        // content-view controller and deactivates constraints attached to the
        // previous graph. Create the retained extent constraints only after
        // that transfer is complete.
        let sizing_view: *mut AnyObject =
            if matches!(handle.element_kind(), Some(ElementKind::Pattern)) {
                msg_send![controller.as_object(), splitView]
            } else {
                handle.0.view.as_ptr()
            };
        let initial_extent_constraints = vec![
            dimension_constant_constraint(
                msg_send![sizing_view, widthAnchor],
                initial_content_size.width,
                1000.0,
            ),
            dimension_constant_constraint(
                msg_send![sizing_view, heightAnchor],
                initial_content_size.height,
                1000.0,
            ),
        ];
        finalize_split_mount(handle);
        Ok(initial_extent_constraints)
    }
}

fn finalize_split_mount(handle: &AppKitHandle) {
    let Some(pattern) = *handle.0.pattern.borrow() else {
        return;
    };
    let presentations = handle.0.presentations.borrow();
    // SAFETY: Items are retained by the mounted NSSplitViewController. The
    // sidebar's automatic resize collapse is enabled only after the controller
    // has received its real window extent.
    unsafe {
        for (index, presentation) in presentations.iter().enumerate() {
            let Some(item) = &presentation.owner else {
                continue;
            };
            match (pattern, index) {
                (
                    UiPattern::NavigationSplit {
                        sidebar_collapsible,
                    },
                    0,
                ) => {
                    let _: () = msg_send![item.as_object(), setCollapsed: false];
                    let _: () = msg_send![item.as_object(), setCanCollapseFromWindowResize: sidebar_collapsible];
                }
                (
                    UiPattern::NavigationWorkspace {
                        sidebar_collapsible,
                        ..
                    },
                    0,
                ) => {
                    let _: () = msg_send![item.as_object(), setCollapsed: false];
                    let _: () = msg_send![item.as_object(), setCanCollapseFromWindowResize: sidebar_collapsible];
                }
                (UiPattern::UtilitySplit { .. }, 1)
                | (UiPattern::NavigationWorkspace { .. }, 2) => {
                    let _: () = msg_send![item.as_object(), setCollapsed: false];
                }
                _ => {}
            }
        }
        let _: () = msg_send![handle.view(), layoutSubtreeIfNeeded];
    }
}

fn configure_panel(panel: &AnyObject, behavior: PanelBehavior) {
    // SAFETY: The receiver is an NSPanel and the values come from PanelBehavior.
    unsafe {
        let _: () = msg_send![panel, setFloatingPanel: behavior.floating];
        let _: () = msg_send![panel, setHidesOnDeactivate: behavior.hides_when_inactive];
        let _: () = msg_send![panel, setBecomesKeyOnlyIfNeeded: !behavior.accepts_keyboard];
    }
}

unsafe fn panel_contract_is_valid(panel: &AnyObject) -> bool {
    // SAFETY: The caller supplies the retained auxiliary window on AppKit's
    // main thread and reads only public NSPanel/NSWindow properties.
    let is_panel: bool = unsafe { msg_send![panel, isKindOfClass: objc2::class!(NSPanel)] };
    let can_become_key: bool = unsafe { msg_send![panel, canBecomeKeyWindow] };
    let floating: bool = unsafe { msg_send![panel, isFloatingPanel] };
    let key_only_if_needed: bool = unsafe { msg_send![panel, becomesKeyOnlyIfNeeded] };
    let hides_on_deactivate: bool = unsafe { msg_send![panel, hidesOnDeactivate] };
    is_panel && can_become_key && floating && !key_only_if_needed && !hides_on_deactivate
}

fn install_toolbar(
    window: &AnyObject,
    spec: &WindowSpec,
    mtm: MainThreadMarker,
    renderer: &Renderer<AppKitBackend>,
) -> Retained<ToolbarDelegate> {
    let sidebar_controller = renderer
        .mounted()
        .and_then(|root| split_controller_for(root, PatternRegion::NavigationSidebar));
    let inspector_controller = renderer
        .mounted()
        .and_then(|root| split_controller_for(root, PatternRegion::Inspector));
    let delegate = ToolbarDelegate::new(
        mtm,
        spec.toolbar.clone(),
        sidebar_controller,
        inspector_controller,
    );
    let has_split_controls = delegate.ivars().sidebar_controller.is_some()
        || delegate.ivars().inspector_controller.is_some();
    if !should_install_toolbar(spec.kind, spec.toolbar.len(), has_split_controls) {
        return delegate;
    }
    let identifier = ns_string(&format!("jp.bunko.rinka.{}", spec.id.as_str()));
    // SAFETY: The delegate supplies native items for custom identifiers.
    // NSToolbar owns its items and NSWindow owns the toolbar; the host retains
    // the toolbar's weak delegate for the lifetime of the window.
    unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSToolbar), alloc];
        let toolbar: *mut AnyObject =
            msg_send![allocated, initWithIdentifier: identifier.as_object()];
        let _: () = msg_send![toolbar, setDelegate: &*delegate];
        let _: () = msg_send![toolbar, setAllowsUserCustomization: false];
        let _: () = msg_send![toolbar, setAutosavesConfiguration: false];
        let _: () = msg_send![toolbar,
            setDisplayMode: native_toolbar_display(spec.toolbar_display)
        ];
        let centered_identifiers = spec
            .toolbar
            .iter()
            .filter(|item| item.placement == ToolbarPlacement::Center)
            .map(|item| ns_string(&toolbar_identifier(&item.id)))
            .collect::<Vec<_>>();
        if !centered_identifiers.is_empty() {
            let identifiers = ns_array(&centered_identifiers);
            let set: *mut AnyObject = msg_send![objc2::class!(NSSet),
                setWithArray: identifiers.as_object()
            ];
            let _: () = msg_send![toolbar, setCenteredItemIdentifiers: set];
        }
        let _: () = msg_send![window, setToolbar: toolbar];
        let _: () = msg_send![toolbar, release];
    }
    delegate
}

fn should_install_toolbar(
    kind: WindowKind,
    custom_item_count: usize,
    has_split_controls: bool,
) -> bool {
    !matches!(kind, WindowKind::Panel(_)) && (custom_item_count > 0 || has_split_controls)
}

const fn native_toolbar_display(display: ToolbarDisplay) -> isize {
    match display {
        ToolbarDisplay::Automatic => 0,
        ToolbarDisplay::IconAndLabel => 1,
        ToolbarDisplay::IconOnly => 2,
        ToolbarDisplay::LabelOnly => 3,
    }
}

fn split_controller_for(node: &MountedNode<AppKitHandle>, region: PatternRegion) -> Option<Id> {
    if node
        .handle()
        .0
        .pattern
        .borrow()
        .is_some_and(|pattern| pattern.regions().contains(&region))
    {
        return node.handle().0.auxiliaries.first().cloned();
    }
    node.children()
        .iter()
        .find_map(|child| split_controller_for(child, region))
}

fn mounted_handle_for_key<'a>(
    node: &'a MountedNode<AppKitHandle>,
    key: &str,
) -> Option<&'a AppKitHandle> {
    if node
        .element()
        .key()
        .is_some_and(|candidate| candidate.as_str() == key)
    {
        return Some(node.handle());
    }
    node.children()
        .iter()
        .find_map(|child| mounted_handle_for_key(child, key))
}

/// Reads the mounted label text declared under `key`, if present.
fn mounted_label_text(node: &MountedNode<AppKitHandle>, key: &str) -> Option<String> {
    if node
        .element()
        .key()
        .is_some_and(|candidate| candidate.as_str() == key)
    {
        if let Props::Label { text, .. } = node.element().props() {
            return Some(text.clone());
        }
        return None;
    }
    node.children()
        .iter()
        .find_map(|child| mounted_label_text(child, key))
}

fn mounted_scene(node: &MountedNode<AppKitHandle>) -> Option<&'static str> {
    [
        ("file-list", "ready"),
        ("directory-empty", "empty"),
        ("directory-busy", "busy"),
        ("directory-error", "error"),
        ("canvas-pane", "canvas"),
        ("editor-pane", "editor"),
    ]
    .into_iter()
    .find_map(|(key, scene)| mounted_handle_for_key(node, key).map(|_| scene))
}

/// Renders a window's content view into a PNG at its backing scale.
///
/// # Safety
///
/// The caller supplies a live NSWindow retained by the application delegate
/// and invokes this helper on AppKit's main thread.
unsafe fn write_window_content_png(window: &AnyObject, path: &std::path::Path) -> bool {
    // SAFETY: The content view is a live NSView owned by the retained window.
    unsafe {
        let content: *mut AnyObject = msg_send![window, contentView];
        let Some(content) = NonNull::new(content) else {
            return false;
        };
        write_view_png(content.as_ref(), path)
    }
}

/// Renders one view hierarchy into a PNG at its backing scale.
///
/// # Safety
///
/// The caller supplies a live NSView attached to a window and invokes this
/// helper on AppKit's main thread.
unsafe fn write_view_png(view: &AnyObject, path: &std::path::Path) -> bool {
    // SAFETY: The view caches its own display into a bitmap rep that AppKit
    // sizes for the window's backing scale; every receiver below is a live
    // object returned by the previous call on the main thread.
    unsafe {
        let bounds: Rect = msg_send![view, bounds];
        let representation: *mut AnyObject =
            msg_send![view, bitmapImageRepForCachingDisplayInRect: bounds];
        if representation.is_null() {
            return false;
        }
        let _: () = msg_send![
            view,
            cacheDisplayInRect: bounds,
            toBitmapImageRep: representation
        ];
        let properties: *mut AnyObject = msg_send![objc2::class!(NSDictionary), dictionary];
        // NSBitmapImageFileTypePNG = 4.
        let data: *mut AnyObject = msg_send![
            representation,
            representationUsingType: 4_usize,
            properties: properties
        ];
        if data.is_null() {
            return false;
        }
        let path = ns_string(&path.to_string_lossy());
        msg_send![data, writeToFile: path.as_object(), atomically: true]
    }
}

unsafe fn window_geometry_is_valid(window: &AnyObject) -> bool {
    // SAFETY: The caller supplies an NSWindow retained by the application
    // delegate and invokes this helper on AppKit's main thread.
    let frame: Rect = unsafe { msg_send![window, frame] };
    if !rect_is_finite(frame) || frame.size.width <= 0.0 || frame.size.height <= 0.0 {
        eprintln!("Rinka geometry invalid window frame={frame:?}");
        return false;
    }
    let content: *mut AnyObject = unsafe { msg_send![window, contentView] };
    NonNull::new(content).is_some_and(|content| unsafe { view_geometry_is_valid(content.as_ref()) })
}

unsafe fn view_geometry_is_valid(view: &AnyObject) -> bool {
    // SAFETY: The traversal follows retained NSView subviews on AppKit's main
    // thread and performs read-only geometry and Auto Layout queries.
    let frame: Rect = unsafe { msg_send![view, frame] };
    let ambiguous: bool = unsafe { msg_send![view, hasAmbiguousLayout] };
    let translates: bool = unsafe { msg_send![view, translatesAutoresizingMaskIntoConstraints] };
    if !rect_is_finite(frame)
        || frame.size.width < 0.0
        || frame.size.height < 0.0
        || (ambiguous && !translates)
    {
        let class_name: *mut AnyObject = unsafe { msg_send![view, className] };
        // SAFETY: The optional text query guards its selector, and both
        // string objects are read on the main thread before conversion.
        let text = unsafe {
            let responds: bool = msg_send![view, respondsToSelector: sel!(stringValue)];
            if responds {
                let value: *mut AnyObject = msg_send![view, stringValue];
                rust_string(value)
            } else {
                String::new()
            }
        };
        eprintln!(
            "Rinka geometry invalid view_class={} text={text:?} frame={frame:?} ambiguous={ambiguous} translates={translates}",
            rust_string(class_name)
        );
        return false;
    }
    // A native text view manages private internal subviews (for example
    // NSTextInsertionIndicator, whose caret placeholder legitimately reports
    // ambiguous layout at a zero frame until the caret is placed). The
    // structural invariant validated here covers rinka-created views; the
    // platform control's internals are the platform's own business.
    let is_text_view: bool = unsafe { msg_send![view, isKindOfClass: objc2::class!(NSTextView)] };
    if is_text_view {
        return true;
    }
    let subviews: *mut AnyObject = unsafe { msg_send![view, subviews] };
    let Some(subviews) = NonNull::new(subviews) else {
        return true;
    };
    let count: usize = unsafe { msg_send![subviews.as_ref(), count] };
    (0..count).all(|index| {
        let child: *mut AnyObject = unsafe { msg_send![subviews.as_ref(), objectAtIndex: index] };
        NonNull::new(child).is_some_and(|child| unsafe { view_geometry_is_valid(child.as_ref()) })
    })
}

fn rect_is_finite(rect: Rect) -> bool {
    rect.origin.x.is_finite()
        && rect.origin.y.is_finite()
        && rect.size.width.is_finite()
        && rect.size.height.is_finite()
}

fn refresh_mounted_stacks(node: &MountedNode<AppKitHandle>) {
    for child in node.children() {
        refresh_mounted_stacks(child);
    }
    if node.handle().element_kind() == Some(ElementKind::Stack) {
        refresh_stack_container_constraints(node.handle());
        refresh_stack_constraints(node.handle());
    }
}
