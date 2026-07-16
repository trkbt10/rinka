#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

const SET_ACCESSIBILITY_LABEL: &str = "setAccessibilityLabel:";
const SET_PLACEHOLDER_STRING: &str = "setPlaceholderString:";
const SET_STRING_VALUE: &str = "setStringValue:";
const SET_TITLE: &str = "setTitle:";

unsafe extern "C" {
    #[link_name = "NSToolbarFlexibleSpaceItemIdentifier"]
    static TOOLBAR_FLEXIBLE_SPACE_IDENTIFIER: *mut AnyObject;
    #[link_name = "NSToolbarToggleSidebarItemIdentifier"]
    static TOOLBAR_TOGGLE_SIDEBAR_IDENTIFIER: *mut AnyObject;
    #[link_name = "NSToolbarToggleInspectorItemIdentifier"]
    static TOOLBAR_TOGGLE_INSPECTOR_IDENTIFIER: *mut AnyObject;
    #[link_name = "NSToolbarSidebarTrackingSeparatorItemIdentifier"]
    static TOOLBAR_SIDEBAR_TRACKING_SEPARATOR_IDENTIFIER: *mut AnyObject;
    #[link_name = "NSToolbarInspectorTrackingSeparatorItemIdentifier"]
    static TOOLBAR_INSPECTOR_TRACKING_SEPARATOR_IDENTIFIER: *mut AnyObject;
    #[link_name = "NSFontTextStyleTitle1"]
    static FONT_TEXT_STYLE_TITLE1: *mut AnyObject;
    #[link_name = "NSFontTextStyleHeadline"]
    static FONT_TEXT_STYLE_HEADLINE: *mut AnyObject;
    #[link_name = "NSFontTextStyleBody"]
    static FONT_TEXT_STYLE_BODY: *mut AnyObject;
    #[link_name = "NSFontTextStyleFootnote"]
    static FONT_TEXT_STYLE_FOOTNOTE: *mut AnyObject;
    #[link_name = "NSParagraphStyleAttributeName"]
    static PARAGRAPH_STYLE_ATTRIBUTE_NAME: *mut AnyObject;
    #[link_name = "NSSplitViewWillResizeSubviewsNotification"]
    static SPLIT_VIEW_WILL_RESIZE_NOTIFICATION: *mut AnyObject;
    #[link_name = "NSSplitViewDidResizeSubviewsNotification"]
    static SPLIT_VIEW_DID_RESIZE_NOTIFICATION: *mut AnyObject;
    #[link_name = "NSViewBoundsDidChangeNotification"]
    static VIEW_BOUNDS_DID_CHANGE_NOTIFICATION: *mut AnyObject;
}

/// `NSSplitViewItemCollapseBehaviorPreferResizingSiblingsWithFixedSplitView`.
/// AppKit then keeps the split view—and therefore its NSWindow—fixed while
/// redistributing the collapsed pane's space among sibling panes.
const COLLAPSE_RESIZES_SIBLINGS_WITH_FIXED_SPLIT_VIEW: isize = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct Size {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct Rect {
    origin: Point,
    size: Size,
}

// SAFETY: These layouts and encodings are the public CoreGraphics/AppKit ABI.
unsafe impl objc2::Encode for Point {
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
        "CGPoint",
        &[objc2::Encoding::Double, objc2::Encoding::Double],
    );
}
// SAFETY: These layouts and encodings are the public CoreGraphics/AppKit ABI.
unsafe impl objc2::Encode for Size {
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
        "CGSize",
        &[objc2::Encoding::Double, objc2::Encoding::Double],
    );
}
// SAFETY: These layouts and encodings are the public CoreGraphics/AppKit ABI.
unsafe impl objc2::Encode for Rect {
    const ENCODING: objc2::Encoding =
        objc2::Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
}
/// Retained Objective-C object confined to the main thread.
struct Id {
    pointer: NonNull<AnyObject>,
    _main_thread: PhantomData<Rc<()>>,
}

impl Id {
    /// Takes ownership of an object returned by alloc/new/init/copy.
    unsafe fn from_owned(pointer: *mut AnyObject) -> Self {
        Self {
            pointer: NonNull::new(pointer).expect("AppKit returned nil from an owning constructor"),
            _main_thread: PhantomData,
        }
    }

    /// Retains an object returned with non-owning return conventions.
    unsafe fn from_borrowed(pointer: *mut AnyObject) -> Self {
        let pointer = NonNull::new(pointer).expect("AppKit returned nil");
        // SAFETY: The pointer is a live Objective-C object and this wrapper
        // balances the retain in Drop on the same main thread.
        let _: *mut AnyObject = unsafe { msg_send![pointer.as_ref(), retain] };
        Self {
            pointer,
            _main_thread: PhantomData,
        }
    }

    fn as_object(&self) -> &AnyObject {
        // SAFETY: Id owns a retain for the lifetime of self.
        unsafe { self.pointer.as_ref() }
    }

    fn as_ptr(&self) -> *mut AnyObject {
        self.pointer.as_ptr()
    }
}

impl Clone for Id {
    fn clone(&self) -> Self {
        // SAFETY: Cloning creates a balanced retain on the same main thread.
        unsafe { Self::from_borrowed(self.as_ptr()) }
    }
}

impl Drop for Id {
    fn drop(&mut self) {
        // SAFETY: Id owns one retain and cannot cross to another thread.
        unsafe {
            let _: () = msg_send![self.pointer.as_ref(), release];
        }
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Id")
            .field(&self.pointer.as_ptr())
            .finish()
    }
}

fn ns_string(value: &str) -> Id {
    // SAFETY: NSString copies the provided UTF-8 bytes before returning.
    unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSString), alloc];
        let string: *mut AnyObject = msg_send![allocated,
            initWithBytes: value.as_ptr().cast::<std::ffi::c_void>(),
            length: value.len(),
            encoding: 4_usize
        ];
        Id::from_owned(string)
    }
}

fn rust_string(value: *mut AnyObject) -> String {
    let Some(value) = NonNull::new(value) else {
        return String::new();
    };
    // SAFETY: UTF8String remains valid while the NSString receiver is alive;
    // the bytes are copied into an owned Rust String before returning.
    unsafe {
        let bytes: *const c_char = msg_send![value.as_ref(), UTF8String];
        if bytes.is_null() {
            String::new()
        } else {
            CStr::from_ptr(bytes).to_string_lossy().into_owned()
        }
    }
}

#[derive(Clone, Debug)]
enum TargetKind {
    Activate,
    Input,
    Toggle,
    ToolbarSelection(Vec<String>),
    /// Context-menu item activation dispatching one item identity through the
    /// element's stable event binding.
    ContextMenu(String),
}

#[derive(Debug)]
struct ActionTargetIvars {
    events: EventBindings,
    kind: TargetKind,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ActionTargetIvars]
    struct ActionTarget;

    // SAFETY: NSObjectProtocol adds no invariants beyond the NSObject superclass.
    unsafe impl NSObjectProtocol for ActionTarget {}

    impl ActionTarget {
        #[unsafe(method(performAction:))]
        fn perform_action(&self, sender: &AnyObject) {
            match &self.ivars().kind {
                TargetKind::Activate => self.ivars().events.emit_activate(),
                TargetKind::Input => {
                    // SAFETY: NSTextField and NSSearchField both expose stringValue.
                    let value: *mut AnyObject = unsafe { msg_send![sender, stringValue] };
                    self.ivars().events.emit_input(rust_string(value));
                }
                TargetKind::Toggle => {
                    // SAFETY: The sender is the NSButton connected by create_control.
                    let state: isize = unsafe { msg_send![sender, state] };
                    self.ivars().events.emit_toggle(state != 0);
                }
                TargetKind::ToolbarSelection(identifiers) => {
                    // SAFETY: The sender is the NSToolbarItemGroup created for
                    // this target and reports its selected segment index.
                    let selected: isize = unsafe { msg_send![sender, selectedIndex] };
                    let Ok(index) = usize::try_from(selected) else {
                        return;
                    };
                    if let Some(identifier) = identifiers.get(index) {
                        self.ivars().events.emit_input(identifier.clone());
                    }
                }
                TargetKind::ContextMenu(item_id) => {
                    // The semantic model re-validates enabled state, so a
                    // stale native item cannot dispatch a refused command.
                    let _ = self.ivars().events.emit_context_menu_activation(item_id);
                }
            }
        }
    }
);

impl ActionTarget {
    fn new(mtm: MainThreadMarker, events: EventBindings, kind: TargetKind) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(ActionTargetIvars { events, kind });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }
}

struct ToolbarDelegateIvars {
    items: Vec<ToolbarItem>,
    sidebar_controller: Option<Id>,
    inspector_controller: Option<Id>,
    targets: RefCell<Vec<Retained<ActionTarget>>>,
    native_items: RefCell<Vec<Id>>,
}

impl fmt::Debug for ToolbarDelegateIvars {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolbarDelegateIvars")
            .field("item_count", &self.items.len())
            .field("target_count", &self.targets.borrow().len())
            .finish_non_exhaustive()
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ToolbarDelegateIvars]
    struct ToolbarDelegate;

    // SAFETY: NSObjectProtocol adds no invariants beyond NSObject.
    unsafe impl NSObjectProtocol for ToolbarDelegate {}

    impl ToolbarDelegate {
        #[unsafe(method(toolbarDefaultItemIdentifiers:))]
        fn default_item_identifiers(&self, _toolbar: &AnyObject) -> *mut AnyObject {
            self.identifier_array().as_ptr()
        }

        #[unsafe(method(toolbarAllowedItemIdentifiers:))]
        fn allowed_item_identifiers(&self, _toolbar: &AnyObject) -> *mut AnyObject {
            self.identifier_array().as_ptr()
        }

        #[unsafe(method(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item_for_identifier(
            &self,
            _toolbar: &AnyObject,
            identifier: &AnyObject,
            _will_insert: bool,
        ) -> *mut AnyObject {
            let requested = rust_string(identifier as *const AnyObject as *mut AnyObject);
            let Some(spec) = self
                .ivars()
                .items
                .iter()
                .find(|item| toolbar_identifier(&item.id) == requested)
            else {
                return std::ptr::null_mut();
            };
            let (native, targets) = create_toolbar_item(self.mtm(), spec);
            let pointer = native.as_ptr();
            self.ivars().targets.borrow_mut().extend(targets);
            self.ivars().native_items.borrow_mut().push(native);
            pointer
        }
    }
);

impl ToolbarDelegate {
    fn new(
        mtm: MainThreadMarker,
        items: Vec<ToolbarItem>,
        sidebar_controller: Option<Id>,
        inspector_controller: Option<Id>,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(ToolbarDelegateIvars {
            items,
            sidebar_controller,
            inspector_controller,
            targets: RefCell::new(Vec::new()),
            native_items: RefCell::new(Vec::new()),
        });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }

    fn identifier_array(&self) -> Id {
        let mut identifiers = Vec::with_capacity(self.ivars().items.len() + 3);
        // Standard identifiers let AppKit create the toggle actions and bind
        // toolbar sections to the native split-view dividers.
        if self.ivars().sidebar_controller.is_some() {
            identifiers.push(unsafe { Id::from_borrowed(TOOLBAR_TOGGLE_SIDEBAR_IDENTIFIER) });
            identifiers
                .push(unsafe { Id::from_borrowed(TOOLBAR_SIDEBAR_TRACKING_SEPARATOR_IDENTIFIER) });
        }
        for item in self
            .ivars()
            .items
            .iter()
            .filter(|item| item.placement == ToolbarPlacement::Leading)
        {
            identifiers.push(ns_string(&toolbar_identifier(&item.id)));
        }
        identifiers.push(unsafe { Id::from_borrowed(TOOLBAR_FLEXIBLE_SPACE_IDENTIFIER) });
        for placement in [ToolbarPlacement::Center, ToolbarPlacement::Trailing] {
            for item in self
                .ivars()
                .items
                .iter()
                .filter(|item| item.placement == placement)
            {
                identifiers.push(ns_string(&toolbar_identifier(&item.id)));
            }
        }
        if self.ivars().inspector_controller.is_some() {
            identifiers.push(unsafe {
                Id::from_borrowed(TOOLBAR_INSPECTOR_TRACKING_SEPARATOR_IDENTIFIER)
            });
            identifiers.push(unsafe { Id::from_borrowed(TOOLBAR_TOGGLE_INSPECTOR_IDENTIFIER) });
        }
        ns_array(&identifiers)
    }
}

fn toolbar_identifier(id: &str) -> String {
    format!("jp.bunko.rinka.toolbar.{id}")
}

fn create_toolbar_item(
    mtm: MainThreadMarker,
    spec: &ToolbarItem,
) -> (Id, Vec<Retained<ActionTarget>>) {
    match &spec.kind {
        ToolbarItemKind::Action {
            symbol,
            on_activate,
        } => {
            let target = ActionTarget::new(
                mtm,
                EventBindings::activate(on_activate.clone()),
                TargetKind::Activate,
            );
            let item = create_action_toolbar_item(
                &toolbar_identifier(&spec.id),
                &spec.label,
                *symbol,
                &spec.help,
                spec.enabled,
                &target,
            );
            (item, vec![target])
        }
        ToolbarItemKind::ActionGroup { actions } => create_toolbar_action_group(mtm, spec, actions),
        ToolbarItemKind::SelectionGroup {
            choices,
            selected_id,
            on_select,
        } => create_toolbar_selection_group(mtm, spec, choices, selected_id, on_select),
        ToolbarItemKind::Menu { symbol, entries } => {
            create_toolbar_menu(mtm, spec, *symbol, entries)
        }
        ToolbarItemKind::Search {
            value,
            placeholder,
            accessibility_label,
            on_input,
        } => create_toolbar_search(mtm, spec, value, placeholder, accessibility_label, on_input),
    }
}

fn allocate_toolbar_item(class: &objc2::runtime::AnyClass, identifier: &str) -> Id {
    let identifier = ns_string(identifier);
    // SAFETY: NSToolbarItem subclasses share the designated initializer.
    unsafe {
        let allocated: *mut AnyObject = msg_send![class, alloc];
        let pointer: *mut AnyObject =
            msg_send![allocated, initWithItemIdentifier: identifier.as_object()];
        Id::from_owned(pointer)
    }
}

fn configure_toolbar_item(item: &AnyObject, label: &str, help: &str, enabled: bool) {
    set_string(item, "setLabel:", label);
    set_string(item, "setPaletteLabel:", label);
    set_string(item, "setToolTip:", help);
    // SAFETY: Common NSToolbarItem state is available on every representation.
    unsafe {
        let _: () = msg_send![item, setEnabled: enabled];
    }
}

fn create_action_toolbar_item(
    identifier: &str,
    label: &str,
    symbol: Symbol,
    help: &str,
    enabled: bool,
    target: &Retained<ActionTarget>,
) -> Id {
    let item = allocate_toolbar_item(objc2::class!(NSToolbarItem), identifier);
    configure_toolbar_item(item.as_object(), label, help, enabled);
    // SAFETY: A plain bordered toolbar item lets AppKit provide the Tahoe
    // glass shape, hover response, spacing, and overflow behavior.
    unsafe {
        let _: () = msg_send![item.as_object(), setTarget: &**target];
        let _: () = msg_send![item.as_object(), setAction: sel!(performAction:)];
        let _: () = msg_send![item.as_object(), setBordered: true];
        let _: () = msg_send![item.as_object(), setStyle: 0_isize];
        if let Some(image) = system_image(symbol) {
            let _: () = msg_send![item.as_object(), setImage: image.as_object()];
        }
    }
    item
}

fn create_toolbar_action_group(
    mtm: MainThreadMarker,
    spec: &ToolbarItem,
    actions: &[ToolbarAction],
) -> (Id, Vec<Retained<ActionTarget>>) {
    let group = allocate_toolbar_item(
        objc2::class!(NSToolbarItemGroup),
        &toolbar_identifier(&spec.id),
    );
    configure_toolbar_item(group.as_object(), &spec.label, &spec.help, spec.enabled);
    let mut targets = Vec::with_capacity(actions.len());
    let mut subitems = Vec::with_capacity(actions.len());
    for action in actions {
        let target = ActionTarget::new(
            mtm,
            EventBindings::activate(action.on_activate.clone()),
            TargetKind::Activate,
        );
        let item = create_action_toolbar_item(
            &format!("{}.{}", toolbar_identifier(&spec.id), action.id),
            &action.label,
            action.symbol,
            &action.help,
            spec.enabled && action.enabled,
            &target,
        );
        targets.push(target);
        subitems.push(item);
    }
    let subitems = ns_array(&subitems);
    // SAFETY: The receiver is NSToolbarItemGroup and copies its subitem array.
    unsafe {
        let _: () = msg_send![group.as_object(), setSubitems: subitems.as_object()];
        let _: () = msg_send![group.as_object(),
            setControlRepresentation: native_toolbar_group_display(spec.group_display)
        ];
    }
    (group, targets)
}

fn create_toolbar_selection_group(
    mtm: MainThreadMarker,
    spec: &ToolbarItem,
    choices: &[rinka_core::ToolbarChoice],
    selected_id: &str,
    on_select: &rinka_core::InputHandler,
) -> (Id, Vec<Retained<ActionTarget>>) {
    let identifiers = choices.iter().map(|choice| choice.id.clone()).collect();
    let target = ActionTarget::new(
        mtm,
        EventBindings::input(on_select.clone()),
        TargetKind::ToolbarSelection(identifiers),
    );
    let identifier = ns_string(&toolbar_identifier(&spec.id));
    let images: Vec<Id> = choices
        .iter()
        .map(|choice| {
            system_image(choice.symbol)
                .unwrap_or_else(|| panic!("missing AppKit system image for {:?}", choice.symbol))
        })
        .collect();
    let labels: Vec<Id> = choices
        .iter()
        .map(|choice| ns_string(&choice.label))
        .collect();
    let images = ns_array(&images);
    let labels = ns_array(&labels);
    // SAFETY: The class method returns a system-managed segmented group and
    // copies the equally ordered image and label arrays.
    let pointer: *mut AnyObject = unsafe {
        msg_send![objc2::class!(NSToolbarItemGroup),
            groupWithItemIdentifier: identifier.as_object(),
            images: images.as_object(),
            selectionMode: 0_isize,
            labels: labels.as_object(),
            target: &*target,
            action: sel!(performAction:)
        ]
    };
    let group = unsafe { Id::from_borrowed(pointer) };
    configure_toolbar_item(group.as_object(), &spec.label, &spec.help, spec.enabled);
    // SAFETY: The convenience constructor creates one subitem per choice.
    unsafe {
        let _: () = msg_send![group.as_object(),
            setControlRepresentation: native_toolbar_group_display(spec.group_display)
        ];
        let subitems: *mut AnyObject = msg_send![group.as_object(), subitems];
        for (index, choice) in choices.iter().enumerate() {
            let subitem: *mut AnyObject = msg_send![subitems, objectAtIndex: index];
            let _: () = msg_send![subitem, setEnabled: spec.enabled && choice.enabled];
            if choice.id == selected_id {
                let _: () = msg_send![group.as_object(), setSelected: true, atIndex: index];
            }
        }
    }
    (group, vec![target])
}

fn create_toolbar_menu(
    mtm: MainThreadMarker,
    spec: &ToolbarItem,
    symbol: Symbol,
    entries: &[MenuEntry],
) -> (Id, Vec<Retained<ActionTarget>>) {
    let item = allocate_toolbar_item(
        objc2::class!(NSMenuToolbarItem),
        &toolbar_identifier(&spec.id),
    );
    configure_toolbar_item(item.as_object(), &spec.label, &spec.help, spec.enabled);
    let menu = create_ns_menu(&spec.label);
    let mut targets = Vec::new();
    append_ns_menu_entries(
        &menu,
        entries,
        spec.enabled,
        &mut targets,
        &|menu_item: &MenuItem| {
            ActionTarget::new(
                mtm,
                EventBindings::activate(menu_item.on_activate.clone()),
                TargetKind::Activate,
            )
        },
    );
    // SAFETY: The receiver is an NSMenuToolbarItem; it retains the menu, and
    // the explicit targets are retained by the toolbar delegate for the same
    // lifetime because NSMenuItem holds its target weakly.
    unsafe {
        let _: () = msg_send![item.as_object(), setMenu: menu.as_object()];
        let _: () = msg_send![item.as_object(), setShowsIndicator: true];
        if let Some(image) = system_image(symbol) {
            let _: () = msg_send![item.as_object(), setImage: image.as_object()];
        }
    }
    (item, targets)
}

fn create_ns_menu(title: &str) -> Id {
    let title = ns_string(title);
    // SAFETY: initWithTitle: is NSMenu's designated initializer. Automatic
    // enabling is disabled because Rinka's declarative enabled state is
    // authoritative for every menu item.
    unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSMenu), alloc];
        let pointer: *mut AnyObject = msg_send![allocated, initWithTitle: title.as_object()];
        let menu = Id::from_owned(pointer);
        let _: () = msg_send![menu.as_object(), setAutoenablesItems: false];
        menu
    }
}

/// Appends the shared menu vocabulary onto a native menu.
///
/// `ancestors_enabled` folds the enabled state of the owning control and every
/// enclosing submenu into each item, matching the semantic contract that a
/// disabled ancestor also disables its entries. The caller owns target
/// retention because NSMenuItem holds its target weakly.
fn append_ns_menu_entries(
    menu: &Id,
    entries: &[MenuEntry],
    ancestors_enabled: bool,
    targets: &mut Vec<Retained<ActionTarget>>,
    make_target: &dyn Fn(&MenuItem) -> Retained<ActionTarget>,
) {
    for entry in entries {
        match entry {
            MenuEntry::Separator => {
                // SAFETY: separatorItem returns a shared autoreleased item and
                // NSMenu retains every item it contains.
                unsafe {
                    let separator: *mut AnyObject =
                        msg_send![objc2::class!(NSMenuItem), separatorItem];
                    let _: () = msg_send![menu.as_object(), addItem: separator];
                }
            }
            MenuEntry::Item(item) => {
                let target = make_target(item);
                let native = create_ns_menu_item(item, ancestors_enabled, &target);
                // SAFETY: NSMenu retains the inserted item.
                unsafe {
                    let _: () = msg_send![menu.as_object(), addItem: native.as_object()];
                }
                targets.push(target);
            }
            MenuEntry::Submenu(submenu) => {
                let enabled = ancestors_enabled && submenu.enabled;
                let title = ns_string(&submenu.label);
                let key = ns_string("");
                let nested = create_ns_menu(&submenu.label);
                append_ns_menu_entries(&nested, &submenu.entries, enabled, targets, make_target);
                // SAFETY: The item is created through the designated
                // initializer with a nil action; NSMenuItem retains its
                // submenu and NSMenu retains the item.
                unsafe {
                    let allocated: *mut AnyObject = msg_send![objc2::class!(NSMenuItem), alloc];
                    let pointer: *mut AnyObject = msg_send![allocated,
                        initWithTitle: title.as_object(),
                        action: None::<objc2::runtime::Sel>,
                        keyEquivalent: key.as_object()
                    ];
                    let native = Id::from_owned(pointer);
                    let _: () = msg_send![native.as_object(), setEnabled: enabled];
                    let _: () = msg_send![native.as_object(), setSubmenu: nested.as_object()];
                    let _: () = msg_send![menu.as_object(), addItem: native.as_object()];
                }
            }
        }
    }
}

fn create_ns_menu_item(
    item: &MenuItem,
    ancestors_enabled: bool,
    target: &Retained<ActionTarget>,
) -> Id {
    let title = ns_string(&item.label);
    let key = ns_string("");
    // SAFETY: The item is created through the designated initializer and the
    // selector target has the matching one-argument signature. State value 1
    // is NSControlStateValueOn, the public checkmark constant. NSMenuItem
    // holds its target weakly, so the item also retains the target through
    // its strong representedObject property; the target then lives exactly
    // as long as the native item that fires it.
    unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSMenuItem), alloc];
        let pointer: *mut AnyObject = msg_send![allocated,
            initWithTitle: title.as_object(),
            action: sel!(performAction:),
            keyEquivalent: key.as_object()
        ];
        let native = Id::from_owned(pointer);
        let _: () = msg_send![native.as_object(), setTarget: &**target];
        let _: () = msg_send![native.as_object(), setRepresentedObject: &**target];
        let _: () =
            msg_send![native.as_object(), setEnabled: ancestors_enabled && item.enabled];
        let _: () = msg_send![native.as_object(), setState: isize::from(item.checked)];
        set_string(native.as_object(), "setToolTip:", &item.help);
        if let Some(image) = item.symbol.and_then(system_image) {
            let _: () = msg_send![native.as_object(), setImage: image.as_object()];
        }
        if let Some(chord) = item.chord {
            // The menu displays the chord; app-wide delivery is owned by the
            // window's accelerator table.
            apply_menu_item_chord(native.as_object(), chord);
        }
        // MenuItemRole::Destructive: AppKit exposes no destructive menu-item
        // treatment (verified against the macOS 26.5 SDK headers), so the
        // item keeps the standard native appearance; the role stays in the
        // model. The resolved fallback contract is documented in
        // reports/context-menus.
        native
    }
}

/// Returns the label class whose contextual interactions honor the nearest
/// retained context menu.
///
/// Stock NSTextField consumes right-clicks without consulting its ancestors,
/// so a click landing on a row title or a labelled element would silently
/// drop the context menu. The subclass resolves `menuForEvent:` by walking
/// from itself through its ancestor views to the first retained menu — the
/// same contract the responder chain provides for other view classes — and
/// pops that menu on right-click. rinka-macos binds AppKit dynamically, so
/// the subclass is registered once through the Objective-C runtime.
fn context_menu_label_class() -> &'static objc2::runtime::AnyClass {
    static CLASS: std::sync::OnceLock<&'static objc2::runtime::AnyClass> =
        std::sync::OnceLock::new();
    CLASS.get_or_init(|| {
        let mut builder = objc2::runtime::ClassBuilder::new(
            c"RinkaContextMenuLabel",
            objc2::class!(NSTextField),
        )
        .expect("the context-menu label class registers once per process");
        // SAFETY: Both implementations match the public NSResponder/NSView
        // selector signatures (one NSEvent argument; menuForEvent: returns
        // NSMenu*, rightMouseDown: returns void).
        unsafe {
            builder.add_method(
                sel!(menuForEvent:),
                label_menu_for_event as extern "C-unwind" fn(_, _, _) -> _,
            );
            builder.add_method(
                sel!(rightMouseDown:),
                label_right_mouse_down as extern "C-unwind" fn(_, _, _),
            );
        }
        builder.register()
    })
}

extern "C-unwind" fn label_menu_for_event(
    label: &AnyObject,
    _command: objc2::runtime::Sel,
    event: &AnyObject,
) -> *mut AnyObject {
    // SAFETY: AppKit delivers the event to a live view on the main thread;
    // the walk reads only retained superviews.
    unsafe {
        let mut view: *mut AnyObject = label as *const AnyObject as *mut AnyObject;
        while let Some(current) = NonNull::new(view) {
            let menu: *mut AnyObject = msg_send![current.as_ref(), menu];
            if !menu.is_null() {
                return menu;
            }
            view = msg_send![current.as_ref(), superview];
        }
        msg_send![super(label, objc2::class!(NSTextField)), menuForEvent: event]
    }
}

extern "C-unwind" fn label_right_mouse_down(
    label: &AnyObject,
    _command: objc2::runtime::Sel,
    event: &AnyObject,
) {
    // SAFETY: AppKit delivers the event to a live view on the main thread.
    // Popping the resolved menu runs the same native contextual-click
    // presentation AppKit uses for view classes that do not consume
    // right-clicks.
    unsafe {
        let menu: *mut AnyObject = msg_send![label, menuForEvent: event];
        if let Some(menu) = NonNull::new(menu) {
            let _: () = msg_send![
                objc2::class!(NSMenu),
                popUpContextMenu: menu.as_ref(),
                withEvent: event,
                forView: label
            ];
        } else {
            let _: () = msg_send![
                super(label, objc2::class!(NSTextField)),
                rightMouseDown: event
            ];
        }
    }
}

/// Builds the native menu realizing one element's context-menu model.
///
/// Every item targets the element's stable event binding with its own item
/// identity, so reconciliation refreshes activation behavior without touching
/// the native menu. Each NSMenuItem retains its target through
/// representedObject, so the menu owns its complete dispatch chain.
fn build_context_ns_menu(
    mtm: MainThreadMarker,
    menu: &ContextMenu,
    events: &EventBindings,
) -> Id {
    let native = create_ns_menu("");
    let mut targets = Vec::new();
    let events = events.clone();
    append_ns_menu_entries(
        &native,
        &menu.entries,
        true,
        &mut targets,
        &move |item: &MenuItem| {
            ActionTarget::new(
                mtm,
                events.clone(),
                TargetKind::ContextMenu(item.id.clone()),
            )
        },
    );
    // The items retain the targets through representedObject; no external
    // owner is required.
    drop(targets);
    native
}

/// Returns whether two menu models share one native structure, meaning the
/// retained NSMenu can be updated in place instead of being replaced.
fn menu_structure_matches(current: &[MenuEntry], next: &[MenuEntry]) -> bool {
    current.len() == next.len()
        && current.iter().zip(next).all(|pair| match pair {
            (MenuEntry::Separator, MenuEntry::Separator) => true,
            (MenuEntry::Item(current), MenuEntry::Item(next)) => current.id == next.id,
            (MenuEntry::Submenu(current), MenuEntry::Submenu(next)) => {
                current.id == next.id && menu_structure_matches(&current.entries, &next.entries)
            }
            _ => false,
        })
}

/// Updates a structurally unchanged retained NSMenu to the next declarative
/// state: titles, enabled state, checkmarks, help, and symbols.
///
/// # Safety
///
/// `menu` must be a live NSMenu whose item sequence matches `entries`, as
/// established by [`menu_structure_matches`] against the previously realized
/// model.
unsafe fn refresh_ns_menu_items(menu: &AnyObject, entries: &[MenuEntry], ancestors_enabled: bool) {
    for (index, entry) in entries.iter().enumerate() {
        let Ok(index) = isize::try_from(index) else {
            return;
        };
        // SAFETY: The caller guarantees the index is within the item count.
        let native: *mut AnyObject = unsafe { msg_send![menu, itemAtIndex: index] };
        let Some(native) = NonNull::new(native) else {
            continue;
        };
        // SAFETY: The item is a live NSMenuItem owned by the retained menu.
        unsafe {
            match entry {
                MenuEntry::Separator => {}
                MenuEntry::Item(item) => {
                    let title = ns_string(&item.label);
                    let _: () = msg_send![native.as_ref(), setTitle: title.as_object()];
                    let _: () = msg_send![
                        native.as_ref(),
                        setEnabled: ancestors_enabled && item.enabled
                    ];
                    let _: () = msg_send![native.as_ref(), setState: isize::from(item.checked)];
                    set_string(native.as_ref(), "setToolTip:", &item.help);
                    match item.symbol.and_then(system_image) {
                        Some(image) => {
                            let _: () = msg_send![native.as_ref(), setImage: image.as_object()];
                        }
                        None => {
                            let _: () = msg_send![
                                native.as_ref(),
                                setImage: std::ptr::null::<AnyObject>()
                            ];
                        }
                    }
                }
                MenuEntry::Submenu(submenu) => {
                    let enabled = ancestors_enabled && submenu.enabled;
                    let title = ns_string(&submenu.label);
                    let _: () = msg_send![native.as_ref(), setTitle: title.as_object()];
                    let _: () = msg_send![native.as_ref(), setEnabled: enabled];
                    let nested: *mut AnyObject = msg_send![native.as_ref(), submenu];
                    if let Some(nested) = NonNull::new(nested) {
                        refresh_ns_menu_items(nested.as_ref(), &submenu.entries, enabled);
                    }
                }
            }
        }
    }
}

/// Realizes, updates, or removes the context menu retained by a native view.
///
/// AppKit then owns the contextual interactions: secondary click and
/// ctrl-click pop the view's menu at the pointer, and the accessibility
/// show-menu action opens it without one. A structure-preserving model change
/// updates the retained NSMenu in place so even an open menu reflects the
/// next declarative state; a structural change replaces the menu object.
fn reconcile_view_context_menu(
    mtm: MainThreadMarker,
    view: &AnyObject,
    stored: &RefCell<Option<ContextMenu>>,
    next: Option<&ContextMenu>,
    events: &EventBindings,
) {
    let mut stored = stored.borrow_mut();
    match (stored.as_ref(), next) {
        (None, None) => {}
        (Some(_), None) => {
            // SAFETY: The receiver is a live NSView; a nil menu removes the
            // contextual interaction.
            unsafe {
                let _: () = msg_send![view, setMenu: std::ptr::null::<AnyObject>()];
            }
            *stored = None;
        }
        (None, Some(menu)) => {
            let native = build_context_ns_menu(mtm, menu, events);
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
                // SAFETY: The view's retained menu was realized from the
                // stored model, whose structure matches the next model.
                unsafe {
                    let native: *mut AnyObject = msg_send![view, menu];
                    if let Some(native) = NonNull::new(native) {
                        refresh_ns_menu_items(native.as_ref(), &menu.entries, true);
                    }
                }
            } else {
                let native = build_context_ns_menu(mtm, menu, events);
                // SAFETY: The receiver is a live NSView and retains its menu.
                unsafe {
                    let _: () = msg_send![view, setMenu: native.as_object()];
                }
            }
            *stored = Some(menu.clone());
        }
    }
}

fn create_toolbar_search(
    mtm: MainThreadMarker,
    spec: &ToolbarItem,
    value: &str,
    placeholder: &str,
    accessibility_label: &str,
    on_input: &rinka_core::InputHandler,
) -> (Id, Vec<Retained<ActionTarget>>) {
    let item = allocate_toolbar_item(
        objc2::class!(NSSearchToolbarItem),
        &toolbar_identifier(&spec.id),
    );
    configure_toolbar_item(item.as_object(), &spec.label, &spec.help, spec.enabled);
    let target = ActionTarget::new(
        mtm,
        EventBindings::input(on_input.clone()),
        TargetKind::Input,
    );
    // SAFETY: NSSearchToolbarItem owns and sizes its default NSSearchField.
    unsafe {
        let field: *mut AnyObject = msg_send![item.as_object(), searchField];
        set_string(&*field, SET_STRING_VALUE, value);
        set_string(&*field, SET_PLACEHOLDER_STRING, placeholder);
        set_string(&*field, SET_ACCESSIBILITY_LABEL, accessibility_label);
        let _: () = msg_send![field, setEnabled: spec.enabled];
        let _: () = msg_send![field, setTarget: &*target];
        let _: () = msg_send![field, setAction: sel!(performAction:)];
        let _: () = msg_send![item.as_object(), setResignsFirstResponderWithCancel: true];
    }
    (item, vec![target])
}

fn ns_array(objects: &[Id]) -> Id {
    let pointers: Vec<*mut AnyObject> = objects.iter().map(Id::as_ptr).collect();
    // SAFETY: The pointer buffer is valid for the duration of this call and
    // NSArray retains all objects it contains.
    unsafe {
        let pointer: *mut AnyObject = msg_send![objc2::class!(NSArray),
            arrayWithObjects: pointers.as_ptr(),
            count: pointers.len()
        ];
        Id::from_borrowed(pointer)
    }
}
