#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostKind {
    Root,
    Element(ElementKind),
}

struct HandleInner {
    view: Id,
    child_host: Option<Id>,
    host_kind: HostKind,
    target: Option<Retained<ActionTarget>>,
    presentations: RefCell<Vec<Presentation>>,
    layout_constraints: RefCell<Vec<Id>>,
    stack_layout: RefCell<Option<StackLayout>>,
    pattern: RefCell<Option<UiPattern>>,
    content_fit_source_width_capped: Cell<bool>,
    table_delegate: RefCell<Option<Retained<TableDelegate>>>,
    text_delegate: RefCell<Option<Retained<TextAreaDelegate>>>,
    list_row: RefCell<Option<Rc<RefCell<TableRowRecord>>>>,
    canvas_view: RefCell<Option<Retained<CanvasView>>>,
    image_stamp: Cell<Option<ImageStamp>>,
    parent: RefCell<Option<Weak<HandleInner>>>,
    justification_views: RefCell<Vec<Id>>,
    justification_constraints: RefCell<Vec<Id>>,
    /// The most recently realized context-menu model, kept for structure
    /// comparison when reconciliation patches the retained native menu.
    context_menu: RefCell<Option<ContextMenu>>,
    /// The element's stable event binding, kept so a patch that introduces a
    /// context menu can connect native items to the current handlers.
    events: RefCell<Option<EventBindings>>,
    auxiliaries: Vec<Id>,
}

#[derive(Clone, Copy, Debug)]
struct StackLayout {
    axis: Axis,
    spacing: Spacing,
    padding: Option<Spacing>,
    align: Align,
    justify: Justify,
}

#[derive(Clone, Debug)]
struct Presentation {
    source: Id,
    source_kind: Option<ElementKind>,
    view: Id,
    measurement: Id,
    /// For controller-backed containers this retains the native item that
    /// owns the child view controller.
    owner: Option<Id>,
    /// The metric supplied by the semantic NSSplitViewItem factory before
    /// declarative content contributes an intrinsic minimum.
    system_minimum_thickness: Option<f64>,
    constraints: Vec<Id>,
}

/// Main-thread retained AppKit object handle.
#[derive(Clone)]
pub struct AppKitHandle(Rc<HandleInner>);

type ListRegistry = Rc<RefCell<Vec<Weak<HandleInner>>>>;

impl AppKitHandle {
    fn new(
        view: Id,
        host_kind: HostKind,
        target: Option<Retained<ActionTarget>>,
        auxiliaries: Vec<Id>,
    ) -> Self {
        Self(Rc::new(HandleInner {
            view,
            child_host: None,
            host_kind,
            target,
            presentations: RefCell::new(Vec::new()),
            layout_constraints: RefCell::new(Vec::new()),
            stack_layout: RefCell::new(None),
            pattern: RefCell::new(None),
            content_fit_source_width_capped: Cell::new(false),
            table_delegate: RefCell::new(None),
            text_delegate: RefCell::new(None),
            list_row: RefCell::new(None),
            canvas_view: RefCell::new(None),
            image_stamp: Cell::new(None),
            parent: RefCell::new(None),
            justification_views: RefCell::new(Vec::new()),
            justification_constraints: RefCell::new(Vec::new()),
            context_menu: RefCell::new(None),
            events: RefCell::new(None),
            auxiliaries,
        }))
    }

    fn new_container(
        view: Id,
        child_host: Id,
        host_kind: HostKind,
        target: Option<Retained<ActionTarget>>,
        auxiliaries: Vec<Id>,
    ) -> Self {
        Self(Rc::new(HandleInner {
            view,
            child_host: Some(child_host),
            host_kind,
            target,
            presentations: RefCell::new(Vec::new()),
            layout_constraints: RefCell::new(Vec::new()),
            stack_layout: RefCell::new(None),
            pattern: RefCell::new(None),
            content_fit_source_width_capped: Cell::new(false),
            table_delegate: RefCell::new(None),
            text_delegate: RefCell::new(None),
            list_row: RefCell::new(None),
            canvas_view: RefCell::new(None),
            image_stamp: Cell::new(None),
            parent: RefCell::new(None),
            justification_views: RefCell::new(Vec::new()),
            justification_constraints: RefCell::new(Vec::new()),
            context_menu: RefCell::new(None),
            events: RefCell::new(None),
            auxiliaries,
        }))
    }

    fn view(&self) -> &AnyObject {
        self.0.view.as_object()
    }

    fn host_view(&self) -> &AnyObject {
        self.0
            .child_host
            .as_ref()
            .map_or_else(|| self.view(), Id::as_object)
    }

    fn element_kind(&self) -> Option<ElementKind> {
        match self.0.host_kind {
            HostKind::Root => None,
            HostKind::Element(kind) => Some(kind),
        }
    }

    fn split_controller(&self) -> Result<&AnyObject, AppKitError> {
        self.0
            .auxiliaries
            .first()
            .map(Id::as_object)
            .ok_or_else(|| AppKitError("split host has no NSSplitViewController".to_owned()))
    }
}

impl fmt::Debug for AppKitHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppKitHandle")
            .field("view", &self.0.view)
            .field("kind", &self.0.host_kind)
            .field("has_target", &self.0.target.is_some())
            .field("presentation_count", &self.0.presentations.borrow().len())
            .finish()
    }
}

/// AppKit adapter diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppKitError(String);

impl fmt::Display for AppKitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for AppKitError {}

/// Reconciler adapter for AppKit views.
#[derive(Debug)]
pub struct AppKitBackend {
    root: AppKitHandle,
    mtm: MainThreadMarker,
    list_registry: ListRegistry,
    split_restore_pending: Rc<Cell<bool>>,
}

impl AppKitBackend {
    fn new(
        root: Id,
        mtm: MainThreadMarker,
        list_registry: ListRegistry,
        split_restore_pending: Rc<Cell<bool>>,
    ) -> Self {
        Self {
            root: AppKitHandle::new(root, HostKind::Root, None, Vec::new()),
            mtm,
            list_registry,
            split_restore_pending,
        }
    }
}

impl NativeBackend for AppKitBackend {
    type Handle = AppKitHandle;
    type Error = AppKitError;

    fn root(&self) -> Self::Handle {
        self.root.clone()
    }

    fn validate(&self, _element: &Element) -> Result<(), Self::Error> {
        Ok(())
    }

    fn monospace_metrics(&self, font_size: f64) -> Option<MonospaceMetrics> {
        measure_monospace_metrics(font_size)
    }

    fn create(
        &mut self,
        element: &Element,
        events: EventBindings,
    ) -> Result<Self::Handle, Self::Error> {
        let handle = create_element(self.mtm, element, events.clone())?;
        *handle.0.events.borrow_mut() = Some(events.clone());
        if let Some(menu) = element.context_menu_model() {
            install_element_context_menu(self.mtm, &handle, menu, &events);
        }
        if matches!(
            handle.element_kind(),
            Some(ElementKind::Stack | ElementKind::Canvas)
        ) {
            set_view_drag_registration(handle.view(), element.drop_target_model().is_some());
        }
        if handle.element_kind() == Some(ElementKind::List) {
            if self.split_restore_pending.get()
                && let Some(delegate) = handle.0.table_delegate.borrow().as_ref()
                && matches!(
                    *delegate.ivars().pattern.borrow(),
                    CollectionPattern::NavigationSidebar
                        | CollectionPattern::Outline
                        | CollectionPattern::DataTable
                )
            {
                *delegate.ivars().suppress_split_expansion.borrow_mut() = true;
            }
            let mut registry = self.list_registry.borrow_mut();
            registry.retain(|registered| registered.strong_count() > 0);
            registry.push(Rc::downgrade(&handle.0));
        }
        Ok(handle)
    }

    fn apply(&mut self, handle: &Self::Handle, patch: &PropertyPatch) -> Result<(), Self::Error> {
        apply_patch(self.mtm, handle, patch)?;
        let list_handles = list_registry_handles(&self.list_registry);
        refresh_semantic_sidebar_for_handle(handle, &list_handles);
        Ok(())
    }

    fn insert_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        insert_child(parent, child, index)?;
        let list_handles = list_registry_handles(&self.list_registry);
        refresh_semantic_sidebar_for_handle(child, &list_handles);
        Ok(())
    }

    fn remove_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        let semantic_sidebar = semantic_navigation_split_parent(child)
            .or_else(|| semantic_navigation_split_parent(parent));
        remove_child(parent, child, index)?;
        if let Some(sidebar) = semantic_sidebar {
            let list_handles = list_registry_handles(&self.list_registry);
            refresh_semantic_sidebar_content_fit(&sidebar, &list_handles);
        }
        Ok(())
    }

    fn move_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        from: usize,
        to: usize,
    ) -> Result<(), Self::Error> {
        move_child(parent, child, from, to)?;
        let list_handles = list_registry_handles(&self.list_registry);
        refresh_semantic_sidebar_for_handle(child, &list_handles);
        Ok(())
    }

    fn destroy(&mut self, handle: &Self::Handle) -> Result<(), Self::Error> {
        if handle.element_kind() == Some(ElementKind::List) {
            self.list_registry.borrow_mut().retain(|registered| {
                registered
                    .upgrade()
                    .is_some_and(|inner| !Rc::ptr_eq(&inner, &handle.0))
            });
        }
        if handle.element_kind() == Some(ElementKind::TextArea)
            && let Some(delegate) = handle.0.text_delegate.borrow_mut().take()
        {
            // SAFETY: Balances the bounds observation and any queued delayed
            // drain registered while the text area was mounted; both must not
            // outlive the delegate.
            unsafe {
                let center: *mut AnyObject =
                    msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
                let _: () = msg_send![center, removeObserver: &*delegate];
                let _: () = msg_send![objc2::class!(NSObject),
                    cancelPreviousPerformRequestsWithTarget: &*delegate
                ];
            }
        }
        Ok(())
    }
}

fn create_element(
    mtm: MainThreadMarker,
    element: &Element,
    events: EventBindings,
) -> Result<AppKitHandle, AppKitError> {
    match element.props() {
        Props::Label {
            text,
            role,
            selectable,
        } => {
            let value = ns_string(text);
            // SAFETY: AppKit is called on the main thread and returns a live
            // label; the menu-aware label class serves contextual clicks.
            let view = unsafe {
                let pointer: *mut AnyObject =
                    msg_send![context_menu_label_class(), labelWithString: value.as_object()];
                let view = Id::from_borrowed(pointer);
                configure_label(view.as_object(), *role, *selectable);
                view
            };
            Ok(AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Label),
                None,
                Vec::new(),
            ))
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
            let target = ActionTarget::new(mtm, events, TargetKind::Activate);
            let title = ns_string(label);
            // SAFETY: The selector target has the matching one-argument signature.
            let pointer: *mut AnyObject = unsafe {
                msg_send![objc2::class!(NSButton),
                    buttonWithTitle: title.as_object(),
                    target: &*target,
                    action: sel!(performAction:)
                ]
            };
            // SAFETY: Class convenience constructor returns a live autoreleased button.
            let view = unsafe { Id::from_borrowed(pointer) };
            configure_button(
                view.as_object(),
                *role,
                *size,
                *material,
                *enabled,
                tooltip.as_deref(),
                accessibility_label,
            );
            Ok(AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Button),
                Some(target),
                Vec::new(),
            ))
        }
        Props::Input {
            value,
            placeholder,
            kind,
            enabled,
            accessibility_label,
        } => {
            let target = ActionTarget::new(mtm, events, TargetKind::Input);
            let class = match kind {
                InputKind::Search => objc2::class!(NSSearchField),
                InputKind::Text | InputKind::Secure => objc2::class!(NSTextField),
            };
            // SAFETY: initWithFrame is the designated view initializer.
            let view = unsafe {
                let allocated: *mut AnyObject = msg_send![class, alloc];
                let pointer: *mut AnyObject = msg_send![allocated, initWithFrame: Rect::default()];
                Id::from_owned(pointer)
            };
            set_string(view.as_object(), SET_STRING_VALUE, value);
            set_string(view.as_object(), SET_PLACEHOLDER_STRING, placeholder);
            set_string(
                view.as_object(),
                SET_ACCESSIBILITY_LABEL,
                accessibility_label,
            );
            // SAFETY: NSTextField target/action and enabled setters accept these values.
            unsafe {
                let _: () = msg_send![view.as_object(), setTarget: &*target];
                let _: () = msg_send![view.as_object(), setAction: sel!(performAction:)];
                let _: () = msg_send![view.as_object(), setEnabled: *enabled];
            }
            Ok(AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Input),
                Some(target),
                Vec::new(),
            ))
        }
        Props::TextArea {
            content,
            spans,
            selection,
            read_only,
            role,
            accessibility_label,
        } => Ok(create_text_area(
            mtm,
            TextAreaConfig {
                content,
                spans,
                selection: *selection,
                read_only: *read_only,
                role: *role,
                accessibility_label,
            },
            events,
        )),
        Props::Toggle {
            label,
            value,
            size,
            enabled,
            accessibility_label,
        } => {
            let target = ActionTarget::new(mtm, events, TargetKind::Toggle);
            let title = ns_string(label);
            // SAFETY: The selector target has the matching one-argument signature.
            let pointer: *mut AnyObject = unsafe {
                msg_send![objc2::class!(NSButton),
                    checkboxWithTitle: title.as_object(),
                    target: &*target,
                    action: sel!(performAction:)
                ]
            };
            // SAFETY: Class convenience constructor returns a live button.
            let view = unsafe { Id::from_borrowed(pointer) };
            // SAFETY: NSButton accepts state and enabled values.
            unsafe {
                let _: () = msg_send![view.as_object(), setState: isize::from(*value)];
                let _: () = msg_send![view.as_object(), setControlSize: control_size(*size)];
                let _: () = msg_send![view.as_object(), setEnabled: *enabled];
            }
            set_string(
                view.as_object(),
                SET_ACCESSIBILITY_LABEL,
                accessibility_label,
            );
            Ok(AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Toggle),
                Some(target),
                Vec::new(),
            ))
        }
        Props::Progress {
            fraction,
            accessibility_label,
        } => {
            let view = new_view(objc2::class!(NSProgressIndicator));
            // SAFETY: NSProgressIndicator's determinate range accepts these values.
            unsafe {
                let _: () = msg_send![view.as_object(), setIndeterminate: false];
                let _: () = msg_send![view.as_object(), setMinValue: 0.0_f64];
                let _: () = msg_send![view.as_object(), setMaxValue: 1.0_f64];
                let _: () = msg_send![view.as_object(), setDoubleValue: *fraction];
                let _: () = msg_send![view.as_object(), setFrameSize: Size {
                    width: 240.0,
                    height: 20.0,
                }];
            }
            set_string(
                view.as_object(),
                SET_ACCESSIBILITY_LABEL,
                accessibility_label,
            );
            let handle = AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Progress),
                None,
                Vec::new(),
            );
            // NSProgressIndicator intentionally has no intrinsic horizontal
            // size. Supply a soft native preferred width so centered layouts
            // are determinate while a required parent width can still stretch
            // the control for applications that request a full-width meter.
            unsafe {
                handle
                    .0
                    .layout_constraints
                    .borrow_mut()
                    .push(dimension_constant_constraint(
                        msg_send![handle.view(), widthAnchor],
                        240.0,
                        750.0,
                    ));
            }
            Ok(handle)
        }
        Props::Image {
            content,
            scaling,
            accessibility_label,
        } => create_image(content, *scaling, accessibility_label),
        Props::Separator { axis } => {
            let view = new_view(objc2::class!(NSBox));
            // SAFETY: NSBoxSeparator is the public box-type value 2.
            unsafe {
                let _: () = msg_send![view.as_object(), setBoxType: 2_isize];
                let _: () = msg_send![view.as_object(), setContentViewMargins: Size::default()];
                let _: () = msg_send![view.as_object(), setAutoresizingMask: separator_mask(*axis)];
            }
            Ok(AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Separator),
                None,
                Vec::new(),
            ))
        }
        Props::Spacer {
            horizontal,
            vertical,
        } => {
            let view = new_view(objc2::class!(NSView));
            configure_growth(view.as_object(), *horizontal, *vertical);
            Ok(AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Spacer),
                None,
                Vec::new(),
            ))
        }
        Props::Stack {
            axis,
            spacing,
            padding,
            align,
            justify,
        } => Ok(create_stack_handle(
            mtm,
            HostKind::Element(ElementKind::Stack),
            StackLayout {
                axis: *axis,
                spacing: *spacing,
                padding: *padding,
                align: *align,
                justify: *justify,
            },
            events,
            Vec::new(),
        )),
        Props::Scroll { axis } => {
            let view = new_view(objc2::class!(NSScrollView));
            // SAFETY: NSScrollView owns its scroller configuration.
            unsafe {
                let _: () =
                    msg_send![view.as_object(), setHasVerticalScroller: *axis == Axis::Vertical];
                let _: () = msg_send![view.as_object(), setHasHorizontalScroller: *axis == Axis::Horizontal];
                let _: () = msg_send![view.as_object(), setAutohidesScrollers: true];
                let _: () = msg_send![view.as_object(), setDrawsBackground: false];
            }
            // A scroll surface is the primary recipient of surplus room on
            // its scrolling axis; its document retains its content size.
            configure_growth(view.as_object(), true, true);
            Ok(AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Scroll),
                None,
                Vec::new(),
            ))
        }
        Props::Pattern { pattern } => Ok(create_pattern_handle(*pattern)),
        Props::List {
            accessibility_label,
            pattern,
            columns,
        } => Ok(create_native_list(
            mtm,
            accessibility_label,
            *pattern,
            columns,
            events,
            element.drop_target_model().cloned(),
        )),
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
        } => create_list_row(
            mtm,
            events,
            ListRowConfig {
                title,
                subtitle: subtitle.as_deref(),
                cells,
                role: *role,
                expanded: *expanded,
                symbol: *symbol,
                selected: *selected,
                disclosure: *disclosure,
                accessibility_label,
                drop_target: element.drop_target_model().cloned(),
            },
        ),
        Props::Status {
            title,
            message,
            tone,
        } => create_status(title, message, *tone),
        Props::Canvas {
            size,
            scene,
            accepts_input,
            ime_caret,
            accessibility_label,
        } => Ok(create_canvas(
            mtm,
            *size,
            scene,
            *accepts_input,
            *ime_caret,
            accessibility_label,
            events,
        )),
    }
}

/// Attaches a context-menu model at creation time.
///
/// A list row realizes its menu on the table cells the delegate builds, so
/// the model is stored on the row record; every other element realizes it on
/// its owning native view.
fn install_element_context_menu(
    mtm: MainThreadMarker,
    handle: &AppKitHandle,
    menu: &ContextMenu,
    events: &EventBindings,
) {
    if let Some(record) = handle.0.list_row.borrow().as_ref() {
        record.borrow_mut().context_menu = Some(menu.clone());
        return;
    }
    reconcile_view_context_menu(
        mtm,
        context_menu_owner_view(handle),
        &handle.0.context_menu,
        Some(menu),
        events,
    );
}

/// Returns the native view that owns an element's context menu.
///
/// A list attaches the menu to its table so the contextual interaction covers
/// the whole scrolling content area; every other element uses its outer
/// semantic view, and the responder chain serves clicks on menu-less
/// descendants.
fn context_menu_owner_view(handle: &AppKitHandle) -> &AnyObject {
    if handle.element_kind() == Some(ElementKind::List) {
        handle.host_view()
    } else {
        handle.view()
    }
}

fn create_pattern_handle(pattern: UiPattern) -> AppKitHandle {
    let controller = new_object(objc2::class!(NSSplitViewController));
    // SAFETY: NSSplitViewController owns the split view and root view.
    let split_view: *mut AnyObject = unsafe { msg_send![controller.as_object(), splitView] };
    let view: *mut AnyObject = unsafe { msg_send![controller.as_object(), view] };
    let view = unsafe { Id::from_borrowed(view) };
    // SAFETY: A vertical controller split lays panes leading to trailing.
    unsafe {
        let _: () = msg_send![split_view, setVertical: true];
        let _: () = msg_send![split_view, setDividerStyle: 1_isize];
        let _ = nonnegative_dimension_constraint(msg_send![view.as_object(), widthAnchor]);
        let _ = nonnegative_dimension_constraint(msg_send![view.as_object(), heightAnchor]);
    }
    let handle = AppKitHandle::new(
        view,
        HostKind::Element(ElementKind::Pattern),
        None,
        vec![controller],
    );
    *handle.0.pattern.borrow_mut() = Some(pattern);
    handle
}

fn new_view(class: &objc2::runtime::AnyClass) -> Id {
    // SAFETY: Every caller passes an NSView subclass supporting initWithFrame:.
    unsafe {
        let allocated: *mut AnyObject = msg_send![class, alloc];
        let pointer: *mut AnyObject = msg_send![allocated, initWithFrame: Rect::default()];
        Id::from_owned(pointer)
    }
}

fn create_native_list(
    mtm: MainThreadMarker,
    accessibility_label: &str,
    pattern: CollectionPattern,
    columns: &[TableColumn],
    events: EventBindings,
    drop_target: Option<DropTarget>,
) -> AppKitHandle {
    let scroll = new_view(objc2::class!(NSScrollView));
    let table = if matches!(
        pattern,
        CollectionPattern::NavigationSidebar
            | CollectionPattern::Outline
            | CollectionPattern::DataTable
    ) {
        new_view(objc2::class!(NSOutlineView))
    } else {
        new_view(objc2::class!(NSTableView))
    };
    let columns = effective_table_columns(pattern, columns);

    let delegate = TableDelegate::new(mtm, pattern, columns.clone(), events, drop_target);
    // SAFETY: The delegate implements both required informal protocols and is
    // retained by AppKitHandle because NSTableView's delegate is non-owning.
    unsafe {
        install_table_columns(table.as_object(), pattern, &columns);
        configure_table_sort(table.as_object(), &columns);
        if matches!(
            pattern,
            CollectionPattern::NavigationSidebar
                | CollectionPattern::Outline
                | CollectionPattern::DataTable
        ) {
            configure_outline_column(table.as_object());
        }
        let _: () = msg_send![table.as_object(), setDataSource: &*delegate];
        let _: () = msg_send![table.as_object(), setDelegate: &*delegate];
        let _: () = msg_send![table.as_object(), setAllowsMultipleSelection: false];
        let _: () = msg_send![table.as_object(), setAllowsEmptySelection: true];
        let automatic_row_heights = matches!(
            pattern,
            CollectionPattern::ContentList | CollectionPattern::EmbeddedList
        );
        let _: () = msg_send![table.as_object(), setUsesAutomaticRowHeights: automatic_row_heights];
        let _: () = msg_send![table.as_object(), setAutoresizingMask: 2_usize];
        let _: () = msg_send![scroll.as_object(), setDocumentView: table.as_object()];
        let _: () = msg_send![scroll.as_object(), setHasVerticalScroller: true];
        let _: () = msg_send![scroll.as_object(),
            setHasHorizontalScroller: pattern.presents_columns()
        ];
        let _: () = msg_send![scroll.as_object(), setAutohidesScrollers: true];
    }
    configure_growth(scroll.as_object(), true, true);
    set_string(
        scroll.as_object(),
        SET_ACCESSIBILITY_LABEL,
        accessibility_label,
    );
    set_string(
        table.as_object(),
        SET_ACCESSIBILITY_LABEL,
        accessibility_label,
    );
    configure_collection_pattern(scroll.as_object(), table.as_object(), pattern);

    configure_table_drag_source(table.as_object());

    let handle = AppKitHandle::new_container(
        scroll,
        table,
        HostKind::Element(ElementKind::List),
        None,
        Vec::new(),
    );
    *handle.0.table_delegate.borrow_mut() = Some(delegate);
    refresh_table_drag_registration(&handle);
    handle
}
