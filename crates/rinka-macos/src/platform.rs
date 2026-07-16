//! Main-thread AppKit implementation.

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{MainThreadMarker, NSObject};
use rinka_core::{
    Align, ApplicationSpec, Axis, ButtonMaterial, ButtonRole, ControlSize, Element, ElementKind,
    EventBindings, InputKind, Justify, ListRowRole, ListStyle, MountedNode, NativeBackend,
    PanelBehavior, PropertyPatch, Props, Renderer, SortDirection, Spacing, SplitRole, StatusTone,
    Symbol, TableColumn, TableSort, TextRole, ToolbarAction, ToolbarDisplay, ToolbarGroupDisplay,
    ToolbarItem, ToolbarItemKind, ToolbarMenuEntry, ToolbarPlacement, WindowKind, WindowRuntime,
    WindowSpec,
};
use std::cell::{Cell, RefCell};
use std::error::Error;
use std::ffi::{CStr, c_char};
use std::fmt;
use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

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
    entries: &[ToolbarMenuEntry],
) -> (Id, Vec<Retained<ActionTarget>>) {
    let item = allocate_toolbar_item(
        objc2::class!(NSMenuToolbarItem),
        &toolbar_identifier(&spec.id),
    );
    configure_toolbar_item(item.as_object(), &spec.label, &spec.help, spec.enabled);
    let title = ns_string(&spec.label);
    let menu = unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSMenu), alloc];
        let pointer: *mut AnyObject = msg_send![allocated, initWithTitle: title.as_object()];
        Id::from_owned(pointer)
    };
    let mut targets = Vec::new();
    // SAFETY: NSMenu retains each inserted item; explicit targets are retained
    // by the toolbar delegate for the same lifetime.
    unsafe {
        let _: () = msg_send![menu.as_object(), setAutoenablesItems: false];
        for entry in entries {
            match entry {
                ToolbarMenuEntry::Separator => {
                    let separator: *mut AnyObject =
                        msg_send![objc2::class!(NSMenuItem), separatorItem];
                    let _: () = msg_send![menu.as_object(), addItem: separator];
                }
                ToolbarMenuEntry::Action(action) => {
                    let target = ActionTarget::new(
                        mtm,
                        EventBindings::activate(action.on_activate.clone()),
                        TargetKind::Activate,
                    );
                    let title = ns_string(&action.label);
                    let key = ns_string("");
                    let allocated: *mut AnyObject = msg_send![objc2::class!(NSMenuItem), alloc];
                    let menu_item: *mut AnyObject = msg_send![allocated,
                        initWithTitle: title.as_object(),
                        action: sel!(performAction:),
                        keyEquivalent: key.as_object()
                    ];
                    let menu_item = Id::from_owned(menu_item);
                    let _: () = msg_send![menu_item.as_object(), setTarget: &*target];
                    let _: () = msg_send![menu_item.as_object(), setEnabled: spec.enabled && action.enabled];
                    set_string(menu_item.as_object(), "setToolTip:", &action.help);
                    if let Some(image) = system_image(action.symbol) {
                        let _: () = msg_send![menu_item.as_object(), setImage: image.as_object()];
                    }
                    let _: () = msg_send![menu.as_object(), addItem: menu_item.as_object()];
                    targets.push(target);
                }
            }
        }
        let _: () = msg_send![item.as_object(), setMenu: menu.as_object()];
        let _: () = msg_send![item.as_object(), setShowsIndicator: true];
        if let Some(image) = system_image(symbol) {
            let _: () = msg_send![item.as_object(), setImage: image.as_object()];
        }
    }
    (item, targets)
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
        set_string(&*field, "setStringValue:", value);
        set_string(&*field, "setPlaceholderString:", placeholder);
        set_string(&*field, "setAccessibilityLabel:", accessibility_label);
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
    events: EventBindings,
    children: RefCell<Vec<Rc<RefCell<TableRowRecord>>>>,
    outline_identity: Id,
    table: RefCell<Option<Id>>,
}

struct TableDelegateIvars {
    rows: RefCell<Vec<Rc<RefCell<TableRowRecord>>>>,
    style: RefCell<ListStyle>,
    columns: RefCell<Vec<TableColumn>>,
    events: EventBindings,
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
            .field("style", &self.style.borrow())
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
            let style = *self.ivars().style.borrow();
            let column_index = table_column_index(column, &self.ivars().columns.borrow());
            create_table_cell(&record.borrow(), style, column_index)
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
            let style = *self.ivars().style.borrow();
            let column_index = table_column_index(column, &self.ivars().columns.borrow());
            create_table_cell(&record.borrow(), style, column_index)
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
                if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
                    let visible: Rect = unsafe { msg_send![outline, visibleRect] };
                    let hidden: bool = unsafe { msg_send![outline, isHiddenOrHasHiddenAncestor] };
                    eprintln!(
                        "Rinka outline expansion title={:?} expanded=true visible={visible:?} hidden={hidden}",
                        record.borrow().title
                    );
                }
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
                if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
                    let visible: Rect = unsafe { msg_send![outline, visibleRect] };
                    let hidden: bool = unsafe { msg_send![outline, isHiddenOrHasHiddenAncestor] };
                    eprintln!(
                        "Rinka outline expansion title={:?} expanded=false visible={visible:?} hidden={hidden}",
                        record.borrow().title
                    );
                }
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
                    *self.ivars().style.borrow(),
                    ListStyle::Source | ListStyle::Table
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
        style: ListStyle,
        columns: Vec<TableColumn>,
        events: EventBindings,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(TableDelegateIvars {
            rows: RefCell::new(Vec::new()),
            style: RefCell::new(style),
            columns: RefCell::new(columns),
            events,
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
    record: &TableRowRecord,
    style: ListStyle,
    column_index: usize,
) -> *mut AnyObject {
    if style == ListStyle::Table && column_index > 0 {
        return create_table_value_cell(
            record
                .cells
                .get(column_index - 1)
                .map_or("", String::as_str),
        );
    }
    let cell = new_view(objc2::class!(NSTableCellView));
    let title = label_view(&record.title, TextRole::Body);
    let subtitle = record
        .subtitle
        .as_deref()
        .map(|value| label_view(value, TextRole::Secondary));
    let text_stack = if matches!(style, ListStyle::Content | ListStyle::Plain)
        && let Some(subtitle) = &subtitle
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
    if style == ListStyle::Source {
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
    let disclosure = (style != ListStyle::Source && record.disclosure)
        .then(|| system_image(Symbol::Disclosure))
        .flatten()
        .map(|symbol| unsafe {
            let pointer: *mut AnyObject = msg_send![objc2::class!(NSImageView),
                imageViewWithImage: symbol.as_object()
            ];
            Id::from_borrowed(pointer)
        });

    if matches!(style, ListStyle::Source | ListStyle::Table) {
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
                "setAccessibilityLabel:",
                &record.accessibility_label,
            );
        }
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
            "setAccessibilityLabel:",
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

    autorelease_id(cell)
}

fn create_table_value_cell(value: &str) -> *mut AnyObject {
    let cell = new_view(objc2::class!(NSTableCellView));
    let text = label_view(value, TextRole::Body);
    // SAFETY: NSTableCellView lays out its standard text outlet according to
    // the table's effective row-size style.
    unsafe {
        let _: () = msg_send![cell.as_object(), setClipsToBounds: true];
        let _: () = msg_send![cell.as_object(), addSubview: text.as_object()];
        let _: () = msg_send![text.as_object(), setLineBreakMode: 4_isize];
        let _: () = msg_send![text.as_object(), setUsesSingleLineMode: true];
        let _: () = msg_send![cell.as_object(), setTextField: text.as_object()];
        set_string(cell.as_object(), "setAccessibilityLabel:", value);
    }
    autorelease_id(cell)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostKind {
    Root,
    Element(ElementKind),
}

#[derive(Clone, Copy, Debug)]
enum SplitConfiguration {
    Pair {
        role: SplitRole,
        collapsible: bool,
    },
    Workspace {
        sidebar_collapsible: bool,
        inspector_collapsible: bool,
    },
}

struct HandleInner {
    view: Id,
    child_host: Option<Id>,
    host_kind: HostKind,
    split_role: Option<SplitRole>,
    target: Option<Retained<ActionTarget>>,
    presentations: RefCell<Vec<Presentation>>,
    layout_constraints: RefCell<Vec<Id>>,
    stack_layout: RefCell<Option<StackLayout>>,
    split_configuration: RefCell<Option<SplitConfiguration>>,
    content_fit_source_width_capped: Cell<bool>,
    table_delegate: RefCell<Option<Retained<TableDelegate>>>,
    list_row: RefCell<Option<Rc<RefCell<TableRowRecord>>>>,
    parent: RefCell<Option<Weak<HandleInner>>>,
    justification_views: RefCell<Vec<Id>>,
    justification_constraints: RefCell<Vec<Id>>,
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
        split_role: Option<SplitRole>,
        target: Option<Retained<ActionTarget>>,
        auxiliaries: Vec<Id>,
    ) -> Self {
        Self(Rc::new(HandleInner {
            view,
            child_host: None,
            host_kind,
            split_role,
            target,
            presentations: RefCell::new(Vec::new()),
            layout_constraints: RefCell::new(Vec::new()),
            stack_layout: RefCell::new(None),
            split_configuration: RefCell::new(None),
            content_fit_source_width_capped: Cell::new(false),
            table_delegate: RefCell::new(None),
            list_row: RefCell::new(None),
            parent: RefCell::new(None),
            justification_views: RefCell::new(Vec::new()),
            justification_constraints: RefCell::new(Vec::new()),
            auxiliaries,
        }))
    }

    fn new_container(
        view: Id,
        child_host: Id,
        host_kind: HostKind,
        split_role: Option<SplitRole>,
        target: Option<Retained<ActionTarget>>,
        auxiliaries: Vec<Id>,
    ) -> Self {
        Self(Rc::new(HandleInner {
            view,
            child_host: Some(child_host),
            host_kind,
            split_role,
            target,
            presentations: RefCell::new(Vec::new()),
            layout_constraints: RefCell::new(Vec::new()),
            stack_layout: RefCell::new(None),
            split_configuration: RefCell::new(None),
            content_fit_source_width_capped: Cell::new(false),
            table_delegate: RefCell::new(None),
            list_row: RefCell::new(None),
            parent: RefCell::new(None),
            justification_views: RefCell::new(Vec::new()),
            justification_constraints: RefCell::new(Vec::new()),
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
            root: AppKitHandle::new(root, HostKind::Root, None, None, Vec::new()),
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

    fn create(
        &mut self,
        element: &Element,
        events: EventBindings,
    ) -> Result<Self::Handle, Self::Error> {
        let handle = create_element(self.mtm, element, events)?;
        if handle.element_kind() == Some(ElementKind::List) {
            if self.split_restore_pending.get()
                && let Some(delegate) = handle.0.table_delegate.borrow().as_ref()
                && matches!(
                    *delegate.ivars().style.borrow(),
                    ListStyle::Source | ListStyle::Table
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
        apply_patch(handle, patch)?;
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
            // SAFETY: AppKit is called on the main thread and returns a live label.
            let view = unsafe {
                let pointer: *mut AnyObject =
                    msg_send![objc2::class!(NSTextField), labelWithString: value.as_object()];
                let view = Id::from_borrowed(pointer);
                configure_label(view.as_object(), *role, *selectable);
                view
            };
            Ok(AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Label),
                None,
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
                None,
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
            set_string(view.as_object(), "setStringValue:", value);
            set_string(view.as_object(), "setPlaceholderString:", placeholder);
            set_string(
                view.as_object(),
                "setAccessibilityLabel:",
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
                None,
                Some(target),
                Vec::new(),
            ))
        }
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
                "setAccessibilityLabel:",
                accessibility_label,
            );
            Ok(AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Toggle),
                None,
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
                "setAccessibilityLabel:",
                accessibility_label,
            );
            let handle = AppKitHandle::new(
                view,
                HostKind::Element(ElementKind::Progress),
                None,
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
            HostKind::Element(ElementKind::Stack),
            StackLayout {
                axis: *axis,
                spacing: *spacing,
                padding: *padding,
                align: *align,
                justify: *justify,
            },
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
                None,
                Vec::new(),
            ))
        }
        Props::Split { role, collapsible } => Ok(create_split_handle(
            ElementKind::Split,
            Some(*role),
            SplitConfiguration::Pair {
                role: *role,
                collapsible: *collapsible,
            },
        )),
        Props::Workspace {
            sidebar_collapsible,
            inspector_collapsible,
        } => Ok(create_split_handle(
            ElementKind::Workspace,
            None,
            SplitConfiguration::Workspace {
                sidebar_collapsible: *sidebar_collapsible,
                inspector_collapsible: *inspector_collapsible,
            },
        )),
        Props::List {
            accessibility_label,
            style,
            columns,
        } => Ok(create_native_list(
            mtm,
            accessibility_label,
            *style,
            columns,
            events,
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
            },
        ),
        Props::Status {
            title,
            message,
            tone,
        } => create_status(title, message, *tone),
    }
}

fn create_split_handle(
    kind: ElementKind,
    role: Option<SplitRole>,
    configuration: SplitConfiguration,
) -> AppKitHandle {
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
    let handle = AppKitHandle::new(view, HostKind::Element(kind), role, None, vec![controller]);
    *handle.0.split_configuration.borrow_mut() = Some(configuration);
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
    style: ListStyle,
    columns: &[TableColumn],
    events: EventBindings,
) -> AppKitHandle {
    let scroll = new_view(objc2::class!(NSScrollView));
    let table = if matches!(style, ListStyle::Source | ListStyle::Table) {
        new_view(objc2::class!(NSOutlineView))
    } else {
        new_view(objc2::class!(NSTableView))
    };
    let columns = effective_table_columns(style, columns);

    let delegate = TableDelegate::new(mtm, style, columns.clone(), events);
    // SAFETY: The delegate implements both required informal protocols and is
    // retained by AppKitHandle because NSTableView's delegate is non-owning.
    unsafe {
        install_table_columns(table.as_object(), style, &columns);
        configure_table_sort(table.as_object(), &columns);
        if matches!(style, ListStyle::Source | ListStyle::Table) {
            configure_outline_column(table.as_object());
        }
        let _: () = msg_send![table.as_object(), setDataSource: &*delegate];
        let _: () = msg_send![table.as_object(), setDelegate: &*delegate];
        let _: () = msg_send![table.as_object(), setAllowsMultipleSelection: false];
        let _: () = msg_send![table.as_object(), setAllowsEmptySelection: true];
        let automatic_row_heights = matches!(style, ListStyle::Content | ListStyle::Plain);
        let _: () = msg_send![table.as_object(), setUsesAutomaticRowHeights: automatic_row_heights];
        let _: () = msg_send![table.as_object(), setAutoresizingMask: 2_usize];
        let _: () = msg_send![scroll.as_object(), setDocumentView: table.as_object()];
        let _: () = msg_send![scroll.as_object(), setHasVerticalScroller: true];
        let _: () = msg_send![scroll.as_object(),
            setHasHorizontalScroller: style == ListStyle::Table
        ];
        let _: () = msg_send![scroll.as_object(), setAutohidesScrollers: true];
    }
    configure_growth(scroll.as_object(), true, true);
    set_string(
        scroll.as_object(),
        "setAccessibilityLabel:",
        accessibility_label,
    );
    set_string(
        table.as_object(),
        "setAccessibilityLabel:",
        accessibility_label,
    );
    configure_list_style(scroll.as_object(), table.as_object(), style);

    let handle = AppKitHandle::new_container(
        scroll,
        table,
        HostKind::Element(ElementKind::List),
        None,
        None,
        Vec::new(),
    );
    *handle.0.table_delegate.borrow_mut() = Some(delegate);
    handle
}

fn effective_table_columns(style: ListStyle, columns: &[TableColumn]) -> Vec<TableColumn> {
    if style == ListStyle::Table && !columns.is_empty() {
        columns.to_vec()
    } else {
        vec![TableColumn::new("primary", "Name")]
    }
}

unsafe fn install_table_columns(table: &AnyObject, style: ListStyle, columns: &[TableColumn]) {
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
        set_string(native.as_object(), "setTitle:", &column.title);
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
    let autoresizing_style = if style == ListStyle::Table {
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
    // column. Source-list style owns indentation, row height, intercell
    // spacing, selection, and background metrics for the current user setting.
    let native_columns: *mut AnyObject = unsafe { msg_send![outline, tableColumns] };
    let primary: *mut AnyObject = unsafe { msg_send![native_columns, objectAtIndex: 0_usize] };
    let _: () = unsafe { msg_send![outline, setOutlineTableColumn: primary] };
}

fn configure_list_style(scroll: &AnyObject, table: &AnyObject, style: ListStyle) {
    let native_style = match style {
        ListStyle::Content => 2_isize,
        ListStyle::Source => 3_isize,
        ListStyle::Table => 1_isize,
        ListStyle::Plain => 4_isize,
    };
    // SAFETY: Values map directly to public NSTableViewStyle and
    // NSTableViewRowSizeStyle constants. The visual metrics remain AppKit-owned.
    unsafe {
        let _: () = msg_send![table, setStyle: native_style];
        let automatic_row_heights = matches!(style, ListStyle::Content | ListStyle::Plain);
        let _: () = msg_send![table, setUsesAutomaticRowHeights: automatic_row_heights];
        match style {
            ListStyle::Source => {
                let _: () = msg_send![table, setRowSizeStyle: -1_isize];
            }
            ListStyle::Table => {
                // A dense multi-column list uses AppKit's tested small table
                // metric. Source lists continue to follow the user's system
                // sidebar-size preference through the default style.
                let _: () = msg_send![table, setRowSizeStyle: 1_isize];
            }
            ListStyle::Content | ListStyle::Plain => {}
        }
        let _: () = msg_send![table,
            setUsesAlternatingRowBackgroundColors: style == ListStyle::Table
        ];
        let _: () = msg_send![scroll, setDrawsBackground: style != ListStyle::Source];
        if style == ListStyle::Table {
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
            *delegate.ivars().style.borrow(),
            ListStyle::Source | ListStyle::Table
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
        layout_scroll_documents(handle.view(), false);
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
                *delegate.ivars().style.borrow(),
                ListStyle::Source | ListStyle::Table
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
        if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
            eprintln!(
                "Rinka outline controlled title={:?} expanded={}",
                record.borrow().title,
                record.borrow().expanded
            );
        }
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
    if *delegate.ivars().style.borrow() != ListStyle::Table {
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
    if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
        let cell_frame: Rect = unsafe { msg_send![cell.as_ref(), frame] };
        let text_frame: Rect = unsafe { msg_send![text_field.as_ref(), frame] };
        let image_view: *mut AnyObject = unsafe { msg_send![cell.as_ref(), imageView] };
        let image_frame: Rect = NonNull::new(image_view)
            .map(|image| unsafe { msg_send![image.as_ref(), frame] })
            .unwrap_or_default();
        eprintln!(
            "Rinka AppKit primary alignment cell={cell_frame:?} image={image_frame:?} text={text_frame:?} glyph={glyph_rect:?} row_text={row_text_origin:?} header={header_rect:?} header_title={title_rect:?} indent={measured_indent}"
        );
    }
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
            let trailing = (cell_bounds.size.width
                - text_frame.origin.x
                - text_frame.size.width)
                .max(0.0);
            let required = text_frame.origin.x + intrinsic_width + trailing;
            let visible = text_frame
                .size
                .width
                .min((cell_bounds.size.width - text_frame.origin.x).max(0.0));
            if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
                let value: *mut AnyObject = unsafe { msg_send![text_field.as_ref(), stringValue] };
                eprintln!(
                    "Rinka AppKit source row={} cell_frame={frame:?} cell_bounds={cell_bounds:?} text_frame={text_frame:?} intrinsic={intrinsic:?} cell_size={cell_size:?} required={required} visible={visible}",
                    rust_string(value)
                );
            }
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
        if matches!(
            parent.element_kind(),
            Some(ElementKind::Split | ElementKind::Workspace)
        ) {
            let semantic_navigation = matches!(
                *parent.0.split_configuration.borrow(),
                Some(SplitConfiguration::Workspace { .. })
                    | Some(SplitConfiguration::Pair {
                        role: SplitRole::Navigation,
                        ..
                    })
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
        let trace = std::env::var_os("RINKA_APPKIT_TRACE").is_some();
        let frame_before: Rect = msg_send![window, frame];
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
                .is_some_and(|delegate| *delegate.ivars().style.borrow() == ListStyle::Source);
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
            if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
                eprintln!(
                    "Rinka AppKit source fit row_width={row_width} surrounding_width={surrounding_width} pane_width={} source_width={}",
                    pane_bounds.size.width, source_content_size.width
                );
            }
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
        if trace {
            let frame_after: Rect = msg_send![window, frame];
            eprintln!(
                "Rinka AppKit semantic sidebar fit system_minimum={system_minimum} requested_minimum={requested_content_minimum} content_required_width={content_required_width} available_extent={available_extent} applied_minimum={minimum} source_width_capped={source_width_capped} frame_before={frame_before:?} frame_after={frame_after:?}"
            );
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
            .is_some_and(|delegate| *delegate.ivars().style.borrow() == ListStyle::Source);
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

fn configure_label(view: &AnyObject, role: TextRole, selectable: bool) {
    // SAFETY: The receiver is an NSTextField label created above.
    unsafe {
        let _: () = msg_send![view, setSelectable: selectable];
        let _: () = msg_send![view, setLineBreakMode: 0_isize];
        let _: () = msg_send![view, setUsesSingleLineMode: false];
        let font: *mut AnyObject = match role {
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
        };
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
    set_string(view, "setAccessibilityLabel:", accessibility_label);
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
    let handle = AppKitHandle::new_container(view, child_host, host_kind, None, None, auxiliaries);
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
            (Axis::Vertical, Some(Spacing::Content), false, Justify::Center) => {
                constraints.extend([
                    equal_anchor(
                        msg_send![stack.host_view(), centerYAnchor],
                        msg_send![stack.view(), centerYAnchor],
                    ),
                    greater_equal_anchor(inner_top, outer_top),
                    greater_equal_anchor(outer_bottom, inner_bottom),
                ])
            }
            (Axis::Vertical, Some(Spacing::Content), false, Justify::End) => constraints.extend([
                greater_equal_anchor(inner_top, outer_top),
                equal_anchor(outer_bottom, inner_bottom),
            ]),
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
            (Axis::Horizontal, Some(Spacing::Content), false, Justify::Center) => constraints
                .extend([
                    equal_anchor(
                        msg_send![stack.host_view(), centerXAnchor],
                        msg_send![stack.view(), centerXAnchor],
                    ),
                    greater_equal_anchor(inner_leading, outer_leading),
                    greater_equal_anchor(outer_trailing, inner_trailing),
                ]),
            (Axis::Horizontal, Some(Spacing::Content), false, Justify::End) => {
                constraints.extend([
                    greater_equal_anchor(inner_leading, outer_leading),
                    equal_anchor(outer_trailing, inner_trailing),
                ])
            }
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
            (Axis::Vertical, Some(padding), false, Justify::Center) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerYAnchor],
                    msg_send![stack.view(), centerYAnchor],
                ),
                vertical_system_spacing_at_least_with_priority(
                    inner_top, outer_top, padding, 750.0,
                ),
                vertical_system_spacing_at_least_with_priority(
                    outer_bottom,
                    inner_bottom,
                    padding,
                    750.0,
                ),
            ]),
            (Axis::Vertical, Some(padding), false, Justify::End) => constraints.extend([
                vertical_system_spacing_at_least_with_priority(
                    inner_top, outer_top, padding, 750.0,
                ),
                vertical_system_spacing_with_priority(outer_bottom, inner_bottom, padding, 751.0),
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
            (Axis::Horizontal, Some(padding), false, Justify::Center) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerXAnchor],
                    msg_send![stack.view(), centerXAnchor],
                ),
                horizontal_system_spacing_at_least_with_priority(
                    inner_leading,
                    outer_leading,
                    padding,
                    750.0,
                ),
                horizontal_system_spacing_at_least_with_priority(
                    outer_trailing,
                    inner_trailing,
                    padding,
                    750.0,
                ),
            ]),
            (Axis::Horizontal, Some(padding), false, Justify::End) => constraints.extend([
                horizontal_system_spacing_at_least_with_priority(
                    inner_leading,
                    outer_leading,
                    padding,
                    750.0,
                ),
                horizontal_system_spacing_with_priority(
                    outer_trailing,
                    inner_trailing,
                    padding,
                    751.0,
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
            (Axis::Vertical, None, false, Justify::Center) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerYAnchor],
                    msg_send![stack.view(), centerYAnchor],
                ),
                greater_equal_anchor(inner_top, outer_top),
                greater_equal_anchor(outer_bottom, inner_bottom),
            ]),
            (Axis::Vertical, None, false, Justify::End) => constraints.extend([
                greater_equal_anchor(inner_top, outer_top),
                equal_anchor(outer_bottom, inner_bottom),
            ]),
            (Axis::Horizontal, None, true, _) => constraints.extend([
                equal_anchor(inner_leading, outer_leading),
                equal_anchor(outer_trailing, inner_trailing),
            ]),
            (Axis::Horizontal, None, false, Justify::Start) => constraints.extend([
                equal_anchor(inner_leading, outer_leading),
                greater_equal_anchor(outer_trailing, inner_trailing),
            ]),
            (Axis::Horizontal, None, false, Justify::Center) => constraints.extend([
                equal_anchor(
                    msg_send![stack.host_view(), centerXAnchor],
                    msg_send![stack.view(), centerXAnchor],
                ),
                greater_equal_anchor(inner_leading, outer_leading),
                greater_equal_anchor(outer_trailing, inner_trailing),
            ]),
            (Axis::Horizontal, None, false, Justify::End) => constraints.extend([
                greater_equal_anchor(inner_leading, outer_leading),
                equal_anchor(outer_trailing, inner_trailing),
            ]),
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
        "setAccessibilityLabel:",
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
        events,
        children: RefCell::new(Vec::new()),
        outline_identity: new_object(objc2::class!(NSObject)),
        table: RefCell::new(None),
    }));
    let handle = AppKitHandle::new(
        view,
        HostKind::Element(ElementKind::ListRow),
        None,
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
        None,
        auxiliaries,
    );
    *handle.0.layout_constraints.borrow_mut() = size_constraints;
    Ok(handle)
}

fn label_view(text: &str, role: TextRole) -> Id {
    let value = ns_string(text);
    // SAFETY: AppKit returns a live autoreleased label.
    unsafe {
        let pointer: *mut AnyObject =
            msg_send![objc2::class!(NSTextField), labelWithString: value.as_object()];
        let view = Id::from_borrowed(pointer);
        configure_label(view.as_object(), role, false);
        view
    }
}

fn apply_patch(handle: &AppKitHandle, patch: &PropertyPatch) -> Result<(), AppKitError> {
    match patch {
        PropertyPatch::Label {
            text,
            role,
            selectable,
        } => {
            set_string(handle.view(), "setStringValue:", text);
            configure_label(handle.view(), *role, *selectable);
        }
        PropertyPatch::Button {
            label,
            role,
            size,
            material,
            enabled,
            tooltip,
            accessibility_label,
        } => {
            set_string(handle.view(), "setTitle:", label);
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
        PropertyPatch::Input {
            value,
            placeholder,
            enabled,
            accessibility_label,
            ..
        } => {
            set_string(handle.view(), "setStringValue:", value);
            set_string(handle.view(), "setPlaceholderString:", placeholder);
            set_string(handle.view(), "setAccessibilityLabel:", accessibility_label);
            // SAFETY: The receiver is an NSTextField or NSSearchField.
            unsafe {
                let _: () = msg_send![handle.view(), setEnabled: *enabled];
            }
        }
        PropertyPatch::Toggle {
            label,
            value,
            size,
            enabled,
            accessibility_label,
        } => {
            set_string(handle.view(), "setTitle:", label);
            set_string(handle.view(), "setAccessibilityLabel:", accessibility_label);
            // SAFETY: The receiver is an NSButton checkbox.
            unsafe {
                let _: () = msg_send![handle.view(), setState: isize::from(*value)];
                let _: () = msg_send![handle.view(), setControlSize: control_size(*size)];
                let _: () = msg_send![handle.view(), setEnabled: *enabled];
            }
        }
        PropertyPatch::Progress {
            fraction,
            accessibility_label,
        } => {
            // SAFETY: The receiver is a determinate NSProgressIndicator.
            unsafe {
                let _: () = msg_send![handle.view(), setDoubleValue: *fraction];
            }
            set_string(handle.view(), "setAccessibilityLabel:", accessibility_label);
        }
        PropertyPatch::Separator { axis } => {
            // SAFETY: NSView autoresizing flags are a stable bitmask.
            unsafe {
                let _: () = msg_send![handle.view(), setAutoresizingMask: separator_mask(*axis)];
            }
        }
        PropertyPatch::Stack {
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
        PropertyPatch::Spacer {
            horizontal,
            vertical,
        } => configure_growth(handle.view(), *horizontal, *vertical),
        PropertyPatch::Scroll { axis } => {
            // SAFETY: The receiver is an NSScrollView.
            unsafe {
                let _: () =
                    msg_send![handle.view(), setHasVerticalScroller: *axis == Axis::Vertical];
                let _: () =
                    msg_send![handle.view(), setHasHorizontalScroller: *axis == Axis::Horizontal];
            }
        }
        PropertyPatch::Split { role, collapsible } => {
            *handle.0.split_configuration.borrow_mut() = Some(SplitConfiguration::Pair {
                role: *role,
                collapsible: *collapsible,
            });
            refresh_split_item_configuration(handle);
        }
        PropertyPatch::Workspace {
            sidebar_collapsible,
            inspector_collapsible,
        } => {
            *handle.0.split_configuration.borrow_mut() = Some(SplitConfiguration::Workspace {
                sidebar_collapsible: *sidebar_collapsible,
                inspector_collapsible: *inspector_collapsible,
            });
            refresh_split_item_configuration(handle);
        }
        PropertyPatch::List {
            accessibility_label,
            style,
            columns,
        } => {
            set_string(handle.view(), "setAccessibilityLabel:", accessibility_label);
            set_string(
                handle.host_view(),
                "setAccessibilityLabel:",
                accessibility_label,
            );
            if let Some(delegate) = handle.0.table_delegate.borrow().as_ref() {
                *delegate.ivars().style.borrow_mut() = *style;
                *delegate.ivars().columns.borrow_mut() = effective_table_columns(*style, columns);
            }
            let columns = effective_table_columns(*style, columns);
            // SAFETY: A List handle's child host is its NSTableView.
            unsafe {
                install_table_columns(handle.host_view(), *style, &columns);
                if matches!(*style, ListStyle::Source | ListStyle::Table) {
                    configure_outline_column(handle.host_view());
                }
            }
            configure_list_style(handle.view(), handle.host_view(), *style);
            reload_native_list(handle)?;
        }
        PropertyPatch::ListRow {
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
            set_string(handle.view(), "setAccessibilityLabel:", accessibility_label);
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
            }
            if let Some(list) = list_ancestor(handle) {
                reload_native_list(&list)?;
            }
        }
        PropertyPatch::Status { title, message, .. } => {
            if let Some(title_view) = handle.0.auxiliaries.first() {
                set_string(title_view.as_object(), "setStringValue:", title);
            }
            if let Some(message_view) = handle.0.auxiliaries.get(1) {
                set_string(message_view.as_object(), "setStringValue:", message);
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
            HostKind::Element(ElementKind::Split | ElementKind::Workspace) => {
                let view_controller = if matches!(
                    child.element_kind(),
                    Some(ElementKind::Split | ElementKind::Workspace)
                ) {
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
        .split_configuration
        .borrow()
        .ok_or_else(|| AppKitError("split host has no semantic configuration".to_owned()))?;
    // SAFETY: Each factory takes a live view controller and returns an
    // autoreleased NSSplitViewItem with the corresponding system behavior.
    let pointer: *mut AnyObject = unsafe {
        match (configuration, index) {
            (
                SplitConfiguration::Pair {
                    role: SplitRole::Navigation,
                    ..
                },
                0,
            )
            | (SplitConfiguration::Workspace { .. }, 0) => {
                msg_send![objc2::class!(NSSplitViewItem),
                    sidebarWithViewController: view_controller
                ]
            }
            (
                SplitConfiguration::Pair {
                    role: SplitRole::Utility,
                    ..
                },
                1,
            )
            | (SplitConfiguration::Workspace { .. }, 2) => {
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
    match *parent.0.split_configuration.borrow() {
        Some(SplitConfiguration::Pair {
            role: SplitRole::Navigation,
            ..
        }) => index == 1,
        Some(SplitConfiguration::Pair {
            role: SplitRole::Utility,
            ..
        }) => index == 0,
        Some(SplitConfiguration::Workspace { .. }) => index == 1,
        None => false,
    }
}

fn configure_split_item(parent: &AppKitHandle, item: &AnyObject, index: usize) {
    let Some(configuration) = *parent.0.split_configuration.borrow() else {
        return;
    };
    // SAFETY: System sidebar and inspector factories own physical metrics.
    // Rinka supplies only semantic collapse policy and marks the one
    // content item whose safe area follows overlay panes.
    unsafe {
        match (configuration, index) {
            (
                SplitConfiguration::Pair {
                    role: SplitRole::Navigation,
                    collapsible,
                },
                0,
            ) => {
                let _: () = msg_send![item, setCanCollapse: collapsible];
                let _: () = msg_send![item, setCanCollapseFromWindowResize: false];
                let _: () = msg_send![item,
                    setCollapseBehavior: COLLAPSE_RESIZES_SIBLINGS_WITH_FIXED_SPLIT_VIEW
                ];
            }
            (
                SplitConfiguration::Pair {
                    role: SplitRole::Utility,
                    collapsible,
                },
                1,
            ) => {
                let _: () = msg_send![item, setCanCollapse: collapsible];
                let _: () = msg_send![item,
                    setCollapseBehavior: COLLAPSE_RESIZES_SIBLINGS_WITH_FIXED_SPLIT_VIEW
                ];
            }
            (
                SplitConfiguration::Workspace {
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
                SplitConfiguration::Workspace {
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
            (SplitConfiguration::Pair { .. }, _) | (SplitConfiguration::Workspace { .. }, 1) => {
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
                | ElementKind::Split
                | ElementKind::Workspace,
            ) => {
                if matches!(
                    parent.element_kind(),
                    Some(ElementKind::Split | ElementKind::Workspace)
                ) {
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
        Some(ElementKind::Split | ElementKind::Workspace) => {
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

fn set_string(receiver: &AnyObject, selector_name: &str, value: &str) {
    let value = ns_string(value);
    // SAFETY: Every match arm names a public one-NSString-argument AppKit setter.
    unsafe {
        match selector_name {
            "setStringValue:" => {
                let _: () = msg_send![receiver, setStringValue: value.as_object()];
            }
            "setPlaceholderString:" => {
                let _: () = msg_send![receiver, setPlaceholderString: value.as_object()];
            }
            "setAccessibilityLabel:" => {
                let _: () = msg_send![receiver, setAccessibilityLabel: value.as_object()];
            }
            "setTitle:" => {
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

struct ApplicationDelegateIvars {
    application: RefCell<Option<ApplicationSpec>>,
    transition_sizes: Option<(Size, Size)>,
    windows: RefCell<Vec<Id>>,
    window_initial_sizes: RefCell<Vec<Size>>,
    window_initial_extent_constraints: RefCell<Vec<Vec<Id>>>,
    split_window_frames: RefCell<Vec<Option<Rect>>>,
    renderers: RefCell<Vec<WindowRuntime<AppKitBackend>>>,
    list_registries: RefCell<Vec<ListRegistry>>,
    split_resize_epoch: Cell<u64>,
    split_restore_pending: Rc<Cell<bool>>,
    toolbar_delegates: RefCell<Vec<Retained<ToolbarDelegate>>>,
    transition_probe: RefCell<Option<TransitionProbe>>,
    scene_probe: RefCell<Option<SceneProbe>>,
}

#[derive(Clone, Copy, Debug)]
struct TransitionProbe {
    step: usize,
    phase: TransitionProbePhase,
    baseline: Rect,
    wide_size: Size,
    minimum_size: Size,
    attempts: usize,
    observed_split_epoch: u64,
    quiet_turns: usize,
    passed: bool,
}

#[derive(Debug)]
struct SceneProbe {
    expected_scene: String,
    phase: SceneProbePhase,
    attempts: usize,
    requires_live_panel: bool,
    passed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SceneProbePhase {
    AwaitingMainWindow,
    AwaitingPanelWindow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransitionProbePhase {
    WidePending,
    Wide,
    MinimumPending,
    Minimum,
}

impl TransitionProbePhase {
    const fn label(self) -> &'static str {
        match self {
            Self::WidePending => "wide-pending",
            Self::Wide => "wide",
            Self::MinimumPending => "minimum-pending",
            Self::Minimum => "minimum",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransitionProbeAction {
    ToggleSidebar,
    ToggleInspector,
    RestorePanes,
    ResizeToMinimum,
    CompletePhase,
    Finish,
    Wait,
    Fail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TransitionProbeStep {
    label: &'static str,
    sidebar_collapsed: bool,
    inspector_collapsed: bool,
    action: TransitionProbeAction,
}

const TRANSITION_PROBE_STEPS: [TransitionProbeStep; 24] = [
    TransitionProbeStep {
        label: "sidebar-cycle-1-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "sidebar-cycle-1-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "sidebar-cycle-2-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "sidebar-cycle-2-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "sidebar-cycle-3-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "sidebar-cycle-3-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-1-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-1-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-2-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-2-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-3-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-3-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-1-sidebar-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-1-both-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-1-inspector-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-1-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-2-sidebar-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-2-both-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-2-inspector-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-2-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-3-sidebar-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-3-both-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-3-inspector-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-3-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::CompletePhase,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiagnosticAppearance {
    Light,
    Dark,
}

impl DiagnosticAppearance {
    const fn native_name(self) -> &'static str {
        match self {
            Self::Light => "NSAppearanceNameAqua",
            Self::Dark => "NSAppearanceNameDarkAqua",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

fn parse_diagnostic_appearance(
    value: Option<&str>,
) -> Result<Option<DiagnosticAppearance>, String> {
    match value {
        None => Ok(None),
        Some("light") => Ok(Some(DiagnosticAppearance::Light)),
        Some("dark") => Ok(Some(DiagnosticAppearance::Dark)),
        Some(value) => Err(format!(
            "RINKA_APPKIT_APPEARANCE expects 'light' or 'dark', received '{value}'"
        )),
    }
}

unsafe fn configure_diagnostic_appearance(application: &AnyObject) -> Option<DiagnosticAppearance> {
    let requested = match std::env::var("RINKA_APPKIT_APPEARANCE") {
        Ok(value) => parse_diagnostic_appearance(Some(&value)),
        Err(std::env::VarError::NotPresent) => parse_diagnostic_appearance(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("RINKA_APPKIT_APPEARANCE must be valid UTF-8".to_owned())
        }
    }
    .unwrap_or_else(|error| panic!("{error}"));
    let requested = requested?;
    let name = ns_string(requested.native_name());
    let appearance: *mut AnyObject =
        unsafe { msg_send![objc2::class!(NSAppearance), appearanceNamed: name.as_object()] };
    let Some(appearance) = NonNull::new(appearance) else {
        panic!(
            "AppKit did not provide the requested {} appearance",
            requested.label()
        );
    };
    unsafe {
        let _: () = msg_send![application, setAppearance: appearance.as_ref()];
    }
    Some(requested)
}

unsafe fn assert_diagnostic_appearance(window: &AnyObject, requested: DiagnosticAppearance) {
    let effective: *mut AnyObject = unsafe { msg_send![window, effectiveAppearance] };
    let light = ns_string(DiagnosticAppearance::Light.native_name());
    let dark = ns_string(DiagnosticAppearance::Dark.native_name());
    let choices = ns_array(&[light, dark]);
    let matched: *mut AnyObject =
        unsafe { msg_send![effective, bestMatchFromAppearancesWithNames: choices.as_object()] };
    let actual = rust_string(matched);
    if actual != requested.native_name() {
        panic!(
            "AppKit effective appearance mismatch: requested {}, received {actual}",
            requested.label()
        );
    }
    eprintln!(
        "Rinka AppKit appearance requested={} effective={} pass=true",
        requested.label(),
        actual
    );
}

impl fmt::Debug for ApplicationDelegateIvars {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApplicationDelegateIvars")
            .field("has_application", &self.application.borrow().is_some())
            .field("window_count", &self.windows.borrow().len())
            .field(
                "window_initial_size_count",
                &self.window_initial_sizes.borrow().len(),
            )
            .field(
                "window_initial_extent_constraint_count",
                &self.window_initial_extent_constraints.borrow().len(),
            )
            .field(
                "split_window_frame_count",
                &self.split_window_frames.borrow().len(),
            )
            .field("renderer_count", &self.renderers.borrow().len())
            .finish_non_exhaustive()
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ApplicationDelegateIvars]
    struct ApplicationDelegate;

    // SAFETY: NSObjectProtocol adds no invariants beyond the NSObject superclass.
    unsafe impl NSObjectProtocol for ApplicationDelegate {}

    impl ApplicationDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &AnyObject) {
            self.show_initial_windows();
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window(&self, _application: &AnyObject) -> bool {
            true
        }

        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(
            &self,
            _application: &AnyObject,
            _has_visible_windows: bool,
        ) -> bool {
            // The windows are retained by this delegate and AppKit brings them
            // forward as part of its standard reopen processing.
            true
        }

        #[unsafe(method(refreshInitialLayout:))]
        fn refresh_initial_layout(&self, _sender: *mut AnyObject) {
            let trace = std::env::var_os("RINKA_APPKIT_TRACE").is_some();
            for runtime in self.ivars().renderers.borrow().iter() {
                runtime.with_renderer(|renderer| {
                    if let Some(root) = renderer.mounted() {
                        refresh_mounted_stacks(root);
                    }
                });
            }
            for window in self.ivars().windows.borrow().iter() {
                // SAFETY: The retained objects are NSWindow instances created
                // by this delegate and the delayed selector runs on main.
                unsafe {
                    let content: *mut AnyObject = msg_send![window.as_object(), contentView];
                    if let Some(content) = NonNull::new(content) {
                        let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
                        layout_scroll_documents(content.as_ref(), trace);
                        let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
                        if trace {
                            trace_window_contract(window.as_object());
                            trace_view_tree(content.as_ref(), 0);
                        }
                    }
                }
            }
            // Apply declarative window sizes after the initial native layout
            // has propagated through every content-view controller.
            unsafe {
                let _: () = msg_send![self,
                    performSelector: sel!(restoreInitialWindowSizes:),
                    withObject: std::ptr::null::<AnyObject>(),
                    afterDelay: 0.0_f64
                ];
            }
        }

        #[unsafe(method(restoreInitialWindowSizes:))]
        fn restore_initial_window_sizes(&self, _sender: *mut AnyObject) {
            let trace = std::env::var_os("RINKA_APPKIT_TRACE").is_some();
            for (index, window) in self.ivars().windows.borrow().iter().enumerate() {
                let Some(initial_size) = self
                    .ivars()
                    .window_initial_sizes
                    .borrow()
                    .get(index)
                    .copied()
                else {
                    continue;
                };
                // SAFETY: Retained NSWindow instances are resized on AppKit's
                // main thread after their content controllers have settled.
                unsafe {
                    self.set_window_content_extent(window.as_object(), initial_size);
                    let _: () = msg_send![window.as_object(), setContentSize: initial_size];
                    if trace {
                        let frame: Rect = msg_send![window.as_object(), frame];
                        eprintln!(
                            "Rinka AppKit post-initial-layout size={initial_size:?} frame={frame:?}"
                        );
                    }
                }
            }
            self.begin_transition_probe();
            self.begin_scene_probe();
        }

        #[unsafe(method(runTransitionProbe:))]
        fn run_transition_probe(&self, _sender: *mut AnyObject) {
            self.advance_transition_probe();
        }

        #[unsafe(method(runSceneProbe:))]
        fn run_scene_probe(&self, _sender: *mut AnyObject) {
            self.advance_scene_probe();
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, notification: &AnyObject) {
            // SAFETY: NSWindowDidResizeNotification carries the resized
            // NSWindow as its object and is delivered on the main thread.
            unsafe {
                let window: *mut AnyObject = msg_send![notification, object];
                let Some(window) = NonNull::new(window) else {
                    return;
                };
                let trace = std::env::var_os("RINKA_APPKIT_TRACE").is_some();
                if trace {
                    let frame: Rect = msg_send![window.as_ref(), frame];
                    eprintln!("Rinka AppKit windowDidResize begin frame={frame:?}");
                }
                let content: *mut AnyObject = msg_send![window.as_ref(), contentView];
                let Some(content) = NonNull::new(content) else {
                    return;
                };
                let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
                layout_scroll_documents(content.as_ref(), false);
                let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
            }
            let list_handles = registered_list_handles(&self.ivars().list_registries);
            refresh_all_semantic_sidebar_content_fit(&list_handles);
            if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
                unsafe {
                    let window: *mut AnyObject = msg_send![notification, object];
                    let frame: Rect = msg_send![window, frame];
                    eprintln!("Rinka AppKit windowDidResize end frame={frame:?}");
                    if let Some(index) = self.window_index(&*window)
                        && let Some(constraints) = self
                            .ivars()
                            .window_initial_extent_constraints
                            .borrow()
                            .get(index)
                    {
                        for (axis, constraint) in constraints.iter().enumerate() {
                            let active: bool = msg_send![constraint.as_object(), isActive];
                            let constant: f64 = msg_send![constraint.as_object(), constant];
                            eprintln!(
                                "Rinka AppKit window extent constraint axis={axis} active={active} constant={constant}"
                            );
                        }
                    }
                }
            }
        }

        #[unsafe(method(windowWillResize:toSize:))]
        fn window_will_resize(&self, sender: &AnyObject, frame_size: Size) -> Size {
            // NSWindow proposes an outer-frame size here. Keep the retained
            // content-extent constraints synchronized with the corresponding
            // content rect during an interactive edge drag. AppKit also calls
            // this delegate method for fitting-size changes initiated by a
            // content-view controller; those must not replace the declarative
            // window extent.
            unsafe {
                let in_live_resize: bool = msg_send![sender, inLiveResize];
                if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
                    let current: Rect = msg_send![sender, frame];
                    eprintln!(
                        "Rinka AppKit windowWillResize live={in_live_resize} current={current:?} proposed={frame_size:?}"
                    );
                }
                if !in_live_resize {
                    return frame_size;
                }
                let frame: Rect = msg_send![sender, frame];
                let proposed_frame = Rect {
                    origin: frame.origin,
                    size: frame_size,
                };
                let proposed_content: Rect = msg_send![sender, contentRectForFrameRect: proposed_frame];
                self.set_window_content_extent(sender, proposed_content.size);
            }
            frame_size
        }

        #[unsafe(method(splitViewWillResizeSubviews:))]
        fn split_view_will_resize_subviews(&self, notification: &AnyObject) {
            // SAFETY: A new transaction supersedes any restore queued for the
            // previous native resize sequence.
            unsafe {
                let _: () = msg_send![objc2::class!(NSObject),
                    cancelPreviousPerformRequestsWithTarget: self,
                    selector: sel!(restoreSplitOutlineState:),
                    object: std::ptr::null::<AnyObject>()
                ];
            }
            // Native sidebar and inspector transitions are allowed to resize
            // their sibling items, but never the owning window. Capture the
            // exact outer frame once for the whole coalesced transition.
            unsafe {
                let split_view: *mut AnyObject = msg_send![notification, object];
                if let Some(split_view) = NonNull::new(split_view) {
                    let window: *mut AnyObject = msg_send![split_view.as_ref(), window];
                    if let Some(window) = NonNull::new(window)
                        && let Some(index) = self.window_index(window.as_ref())
                    {
                        let mut frames = self.ivars().split_window_frames.borrow_mut();
                        if let Some(saved) = frames.get_mut(index)
                            && saved.is_none()
                        {
                            *saved = Some(msg_send![window.as_ref(), frame]);
                        }
                    }
                }
            }
            self.ivars().split_restore_pending.set(true);
            self.ivars()
                .split_resize_epoch
                .set(self.ivars().split_resize_epoch.get().wrapping_add(1));
            for handle in registered_list_handles(&self.ivars().list_registries) {
                if let Some(delegate) = handle.0.table_delegate.borrow().as_ref() {
                    if !matches!(
                        *delegate.ivars().style.borrow(),
                        ListStyle::Source | ListStyle::Table
                    ) {
                        continue;
                    }
                    // SAFETY: A newer split transaction owns suppression from
                    // this point, so a deferred clear from an older transaction
                    // must not release it.
                    unsafe {
                        let _: () = msg_send![objc2::class!(NSObject),
                            cancelPreviousPerformRequestsWithTarget: &**delegate,
                            selector: sel!(clearSplitExpansionSuppression),
                            object: std::ptr::null::<AnyObject>()
                        ];
                    }
                    *delegate.ivars().suppress_split_expansion.borrow_mut() = true;
                }
            }
        }

        #[unsafe(method(splitViewDidResizeSubviews:))]
        fn split_view_did_resize_subviews(&self, _notification: &AnyObject) {
            self.ivars().split_restore_pending.set(true);
            self.ivars()
                .split_resize_epoch
                .set(self.ivars().split_resize_epoch.get().wrapping_add(1));
            // SAFETY: Coalescing repeated resize notifications lets the last
            // animation frame restore every controlled outline exactly once.
            unsafe {
                let _: () = msg_send![objc2::class!(NSObject),
                    cancelPreviousPerformRequestsWithTarget: self,
                    selector: sel!(restoreSplitOutlineState:),
                    object: std::ptr::null::<AnyObject>()
                ];
                let _: () = msg_send![self,
                    performSelector: sel!(restoreSplitOutlineState:),
                    withObject: std::ptr::null::<AnyObject>(),
                    afterDelay: 0.08_f64
                ];
            }
        }

        #[unsafe(method(restoreSplitOutlineState:))]
        fn restore_split_outline_state(&self, _sender: *mut AnyObject) {
            let list_handles = registered_list_handles(&self.ivars().list_registries);
            for handle in &list_handles {
                let delegate = handle.0.table_delegate.borrow();
                let Some(delegate) = delegate.as_ref() else {
                    continue;
                };
                if !matches!(
                    *delegate.ivars().style.borrow(),
                    ListStyle::Source | ListStyle::Table
                ) {
                    continue;
                }
                // The mounted tree may have replaced this outline after Will.
                // Claim suppression again immediately before applying state so
                // every live delegate belongs to the current restore sequence.
                unsafe {
                    let _: () = msg_send![objc2::class!(NSObject),
                        cancelPreviousPerformRequestsWithTarget: &**delegate,
                        selector: sel!(clearSplitExpansionSuppression),
                        object: std::ptr::null::<AnyObject>()
                    ];
                }
                *delegate.ivars().suppress_split_expansion.borrow_mut() = true;
                // SAFETY: The retained handle is a mounted NSOutlineView.
                // Reapplying only expansion preserves selection, scrolling,
                // sorting, and cell identity in every unaffected list.
                unsafe {
                    apply_outline_expansion(handle.host_view(), &delegate.ivars().rows.borrow());
                    let _: () = msg_send![&**delegate,
                        performSelector: sel!(clearSplitExpansionSuppression),
                        withObject: std::ptr::null::<AnyObject>(),
                        afterDelay: 0.0_f64
                    ];
                }
            }
            refresh_all_semantic_sidebar_content_fit(&list_handles);
            let frames = {
                let mut saved = self.ivars().split_window_frames.borrow_mut();
                saved
                    .iter_mut()
                    .map(Option::take)
                    .collect::<Vec<Option<Rect>>>()
            };
            for (index, frame) in frames.into_iter().enumerate() {
                let Some(frame) = frame else {
                    continue;
                };
                let Some(window) = self.ivars().windows.borrow().get(index).cloned() else {
                    continue;
                };
                // Restore both the origin and size. AppKit may otherwise keep
                // the content width while shifting the outer frame as a split
                // item expands near a screen edge.
                unsafe {
                    let content: Rect = msg_send![window.as_object(), contentRectForFrameRect: frame];
                    self.set_window_content_extent(window.as_object(), content.size);
                    let _: () = msg_send![window.as_object(), setFrame: frame, display: true];
                }
            }
            self.ivars().split_restore_pending.set(false);
        }
    }
);

impl ApplicationDelegate {
    fn new(mtm: MainThreadMarker, application: ApplicationSpec) -> Retained<Self> {
        let transition_sizes = application.windows.first().map(|spec| {
            (
                Size {
                    width: spec.initial_size.width,
                    height: spec.initial_size.height,
                },
                Size {
                    width: spec.minimum_size.width,
                    height: spec.minimum_size.height,
                },
            )
        });
        let object = Self::alloc(mtm).set_ivars(ApplicationDelegateIvars {
            application: RefCell::new(Some(application)),
            transition_sizes,
            windows: RefCell::new(Vec::new()),
            window_initial_sizes: RefCell::new(Vec::new()),
            window_initial_extent_constraints: RefCell::new(Vec::new()),
            split_window_frames: RefCell::new(Vec::new()),
            renderers: RefCell::new(Vec::new()),
            list_registries: RefCell::new(Vec::new()),
            split_resize_epoch: Cell::new(0),
            split_restore_pending: Rc::new(Cell::new(false)),
            toolbar_delegates: RefCell::new(Vec::new()),
            transition_probe: RefCell::new(None),
            scene_probe: RefCell::new(None),
        });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }

    fn window_index(&self, window: &AnyObject) -> Option<usize> {
        self.ivars()
            .windows
            .borrow()
            .iter()
            .position(|candidate| std::ptr::eq(candidate.as_object(), window))
    }

    unsafe fn set_window_content_extent(&self, window: &AnyObject, content_size: Size) {
        let Some(index) = self.window_index(window) else {
            return;
        };
        if let Some(constraints) = self
            .ivars()
            .window_initial_extent_constraints
            .borrow()
            .get(index)
        {
            if let Some(width) = constraints.first() {
                let _: () =
                    unsafe { msg_send![width.as_object(), setConstant: content_size.width] };
            }
            if let Some(height) = constraints.get(1) {
                let _: () =
                    unsafe { msg_send![height.as_object(), setConstant: content_size.height] };
            }
        }
        let controller: *mut AnyObject = unsafe { msg_send![window, contentViewController] };
        if let Some(controller) = NonNull::new(controller) {
            let _: () =
                unsafe { msg_send![controller.as_ref(), setPreferredContentSize: content_size] };
        }
    }

    fn show_initial_windows(&self) {
        let Some(application) = self.ivars().application.borrow_mut().take() else {
            return;
        };
        // SAFETY: The host enters here only from the process main thread.
        let app: *mut AnyObject =
            unsafe { msg_send![objc2::class!(NSApplication), sharedApplication] };
        // SAFETY: NSApplication setters are main-thread calls. Cargo-launched
        // diagnostics reach this point without a LaunchServices registration;
        // establishing the policy after AppKit launch supports both that path
        // and normal application-bundle launches.
        unsafe {
            let _: bool = msg_send![app, setActivationPolicy: 0_isize];
        }
        // SAFETY: NSApplication implements NSAppearanceCustomization. This
        // diagnostic override is applied before constructing any native view.
        let diagnostic_appearance = unsafe { configure_diagnostic_appearance(&*app) };

        let mut key_window = None;
        for window in application.windows {
            let is_primary = matches!(window.kind, WindowKind::Main | WindowKind::Preferences);
            let initial_size = Size {
                width: window.initial_size.width,
                height: window.initial_size.height,
            };
            match build_window(
                self.mtm(),
                &window,
                self.ivars().split_restore_pending.clone(),
            ) {
                Ok((
                    native_window,
                    renderer,
                    toolbar_delegate,
                    list_registry,
                    initial_extent_constraints,
                )) => {
                    if let Some(requested) = diagnostic_appearance {
                        // SAFETY: The retained object is the NSWindow or NSPanel
                        // just built on AppKit's main thread.
                        unsafe {
                            assert_diagnostic_appearance(native_window.as_object(), requested);
                        }
                    }
                    // SAFETY: NSWindow's delegate is weak. The application
                    // retains this delegate for the complete event loop.
                    unsafe {
                        let _: () = msg_send![native_window.as_object(), setDelegate: &**self];
                    }
                    if is_primary && key_window.is_none() {
                        key_window = Some(native_window.clone());
                    }
                    self.ivars()
                        .list_registries
                        .borrow_mut()
                        .push(list_registry);
                    self.ivars()
                        .toolbar_delegates
                        .borrow_mut()
                        .push(toolbar_delegate);
                    self.ivars().renderers.borrow_mut().push(renderer);
                    self.ivars()
                        .window_initial_sizes
                        .borrow_mut()
                        .push(initial_size);
                    self.ivars()
                        .window_initial_extent_constraints
                        .borrow_mut()
                        .push(initial_extent_constraints);
                    self.ivars().split_window_frames.borrow_mut().push(None);
                    self.ivars().windows.borrow_mut().push(native_window);
                }
                Err(error) => eprintln!("AppKit host error: {error}"),
            }
        }
        // SAFETY: The application delegate lives for the AppKit run loop.
        // Observing native split resize boundaries keeps controlled outline
        // state isolated from pane animation and window-size layout traffic.
        unsafe {
            let center: *mut AnyObject =
                msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![center,
                addObserver: self,
                selector: sel!(splitViewWillResizeSubviews:),
                name: SPLIT_VIEW_WILL_RESIZE_NOTIFICATION,
                object: std::ptr::null::<AnyObject>()
            ];
            let _: () = msg_send![center,
                addObserver: self,
                selector: sel!(splitViewDidResizeSubviews:),
                name: SPLIT_VIEW_DID_RESIZE_NOTIFICATION,
                object: std::ptr::null::<AnyObject>()
            ];
        }
        // SAFETY: Required for a Cargo-launched, unbundled AppKit process.
        unsafe {
            let _: () = msg_send![app, activate];
            if let Some(window) = key_window {
                let _: () = msg_send![window.as_object(), makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
            }
            let _: () = msg_send![self,
                performSelector: sel!(refreshInitialLayout:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.0_f64
            ];
        }
    }

    fn begin_scene_probe(&self) {
        let Some(expected_scene) = std::env::var_os("RINKA_APPKIT_SCENE_PROBE") else {
            return;
        };
        if std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_some() {
            panic!("scene and transition probes must run in separate processes");
        }
        if self.ivars().scene_probe.borrow().is_some() {
            return;
        }
        let expected_scene = expected_scene
            .into_string()
            .unwrap_or_else(|_| panic!("RINKA_APPKIT_SCENE_PROBE must be valid UTF-8"));
        if !matches!(
            expected_scene.as_str(),
            "ready" | "empty" | "busy" | "error"
        ) {
            panic!("RINKA_APPKIT_SCENE_PROBE expects ready, empty, busy, or error");
        }

        let observed_scene = {
            let renderers = self.ivars().renderers.borrow();
            renderers.first().and_then(|runtime| {
                runtime.with_renderer(|renderer| renderer.mounted().and_then(mounted_scene))
            })
        };
        let windows = self.ivars().windows.borrow();
        let expected_window_count = if expected_scene == "busy" { 2 } else { 1 };
        let scene_passed = observed_scene == Some(expected_scene.as_str());
        let window_count_passed = windows.len() == expected_window_count;
        // SAFETY: The probe only reads retained AppKit windows and views on
        // the main thread after initial layout has completed.
        let geometry_passed = unsafe {
            windows
                .iter()
                .all(|window| window_geometry_is_valid(window.as_object()))
        };
        let visibility_passed = unsafe {
            windows.iter().all(|window| {
                let visible: bool = msg_send![window.as_object(), isVisible];
                let screen: *mut AnyObject = msg_send![window.as_object(), screen];
                visible && !screen.is_null()
            })
        };
        let visibility_required = std::env::var_os("RINKA_APPKIT_WINDOW_LIVE_PROBE").is_some();
        let passed = scene_passed
            && window_count_passed
            && geometry_passed
            && (!visibility_required || visibility_passed);
        eprintln!(
            "Rinka scene probe expected={} observed={} scene_pass={} windows={} expected_windows={} window_count_pass={} geometry_pass={} visibility_required={} visibility_pass={} pass={}",
            expected_scene,
            observed_scene.unwrap_or("unknown"),
            scene_passed,
            windows.len(),
            expected_window_count,
            window_count_passed,
            geometry_passed,
            visibility_required,
            visibility_passed,
            passed
        );

        *self.ivars().scene_probe.borrow_mut() = Some(SceneProbe {
            expected_scene: expected_scene.clone(),
            phase: SceneProbePhase::AwaitingMainWindow,
            attempts: 0,
            requires_live_panel: std::env::var_os("RINKA_APPKIT_PANEL_LIVE_PROBE").is_some(),
            passed,
        });
        if expected_scene == "busy" {
            let requires_live_panel = self
                .ivars()
                .scene_probe
                .borrow()
                .as_ref()
                .is_some_and(|probe| probe.requires_live_panel);
            if !requires_live_panel {
                let panel_contract = windows
                    .get(1)
                    .is_some_and(|panel| unsafe { panel_contract_is_valid(panel.as_object()) });
                drop(windows);
                let stop_reached = self.perform_panel_stop_action();
                eprintln!(
                    "Rinka scene probe panel_static contract={panel_contract} stop_reached={stop_reached} pass={}",
                    panel_contract && stop_reached
                );
                if let Some(probe) = self.ivars().scene_probe.borrow_mut().as_mut() {
                    probe.passed &= panel_contract && stop_reached;
                }
                self.finish_scene_probe();
                return;
            }
            drop(windows);
            self.schedule_scene_probe();
        } else {
            drop(windows);
            self.finish_scene_probe();
        }
    }

    fn advance_scene_probe(&self) {
        let windows = self.ivars().windows.borrow();
        let Some(main) = windows.first() else {
            return;
        };
        let Some(panel) = windows.get(1) else {
            return;
        };
        let phase = self
            .ivars()
            .scene_probe
            .borrow()
            .as_ref()
            .map(|probe| probe.phase);
        if phase == Some(SceneProbePhase::AwaitingMainWindow) {
            // SAFETY: Busy declares one main NSWindow followed by one
            // keyboard-capable floating NSPanel. State is read after at least
            // one main-loop turn so activation can settle without time guesses.
            let (application_active, key_is_main, main_is_main, panel_contract) = unsafe {
                let application: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let application_active: bool = msg_send![application, isActive];
                let key: *mut AnyObject = msg_send![application, keyWindow];
                let main_window: *mut AnyObject = msg_send![application, mainWindow];
                (
                    application_active,
                    key == main.as_ptr(),
                    main_window == main.as_ptr(),
                    panel_contract_is_valid(panel.as_object()),
                )
            };
            if !application_active || !key_is_main || !main_is_main {
                // SAFETY: Activation is idempotent and the retained primary
                // window is the intended key/main window for this phase.
                unsafe {
                    let application: *mut AnyObject =
                        msg_send![objc2::class!(NSApplication), sharedApplication];
                    let _: () = msg_send![application, activate];
                    let _: () = msg_send![main.as_object(),
                        makeKeyAndOrderFront: std::ptr::null::<AnyObject>()
                    ];
                }
                drop(windows);
                if self.retry_scene_probe() {
                    self.schedule_scene_probe();
                } else {
                    self.finish_scene_probe();
                }
                return;
            }
            let initial_passed = panel_contract;
            eprintln!(
                "Rinka scene probe panel_initial application_active={application_active} key_is_main={key_is_main} main_is_main={main_is_main} panel_contract={panel_contract} pass={initial_passed}"
            );
            if let Some(probe) = self.ivars().scene_probe.borrow_mut().as_mut() {
                probe.passed &= initial_passed;
                probe.phase = SceneProbePhase::AwaitingPanelWindow;
                probe.attempts = 0;
            }
            // SAFETY: The retained panel accepts keyboard focus by contract.
            unsafe {
                let _: () = msg_send![panel.as_object(),
                    makeKeyAndOrderFront: std::ptr::null::<AnyObject>()
                ];
            }
            drop(windows);
            self.schedule_scene_probe();
            return;
        }
        // SAFETY: The panel was made key on the preceding main-loop turn.
        let (key_is_panel, main_remains_main, panel_reports_key, main_reports_main) = unsafe {
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let key: *mut AnyObject = msg_send![application, keyWindow];
            let main_window: *mut AnyObject = msg_send![application, mainWindow];
            let panel_is_key: bool = msg_send![panel.as_object(), isKeyWindow];
            let main_is_main: bool = msg_send![main.as_object(), isMainWindow];
            (
                key == panel.as_ptr(),
                main_window == main.as_ptr(),
                panel_is_key,
                main_is_main,
            )
        };
        let panel_passed =
            key_is_panel && main_remains_main && panel_reports_key && main_reports_main;
        if !panel_passed {
            drop(windows);
            if self.retry_scene_probe() {
                self.schedule_scene_probe();
            } else {
                self.finish_scene_probe();
            }
            return;
        }
        eprintln!(
            "Rinka scene probe panel_key key_is_panel={key_is_panel} main_remains_main={main_remains_main} panel_reports_key={panel_reports_key} main_reports_main={main_reports_main} pass={panel_passed}"
        );
        drop(windows);

        let stop_reached = self.perform_panel_stop_action();
        eprintln!("Rinka scene probe panel_stop_reached={stop_reached}");
        if let Some(probe) = self.ivars().scene_probe.borrow_mut().as_mut() {
            probe.passed &= panel_passed && stop_reached;
        }
        self.finish_scene_probe();
    }

    fn perform_panel_stop_action(&self) -> bool {
        {
            let renderers = self.ivars().renderers.borrow();
            renderers.get(1).is_some_and(|runtime| {
                runtime.with_renderer(|renderer| {
                    let Some(root) = renderer.mounted() else {
                        return false;
                    };
                    let Some(handle) = mounted_handle_for_key(root, "cancel-transfer") else {
                        return false;
                    };
                    // SAFETY: The key identifies the native Stop NSButton.
                    unsafe {
                        let _: () = msg_send![handle.view(),
                            performClick: std::ptr::null::<AnyObject>()
                        ];
                    }
                    true
                })
            })
        }
    }

    fn schedule_scene_probe(&self) {
        // SAFETY: The next main-loop turn observes key/main status after AppKit
        // completes panel activation.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runSceneProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.02_f64
            ];
        }
    }

    fn retry_scene_probe(&self) -> bool {
        const MAX_MAIN_LOOP_TURNS: usize = 100;
        let mut probe = self.ivars().scene_probe.borrow_mut();
        let Some(probe) = probe.as_mut() else {
            return false;
        };
        probe.attempts += 1;
        if probe.attempts < MAX_MAIN_LOOP_TURNS {
            return true;
        }
        probe.passed = false;
        eprintln!(
            "Rinka scene probe activation_timeout phase={:?} turns={}",
            probe.phase, probe.attempts
        );
        false
    }

    fn finish_scene_probe(&self) {
        let probe = self.ivars().scene_probe.borrow();
        let Some(probe) = probe.as_ref() else {
            return;
        };
        eprintln!(
            "Rinka scene probe scene={} result={}",
            probe.expected_scene,
            if probe.passed { "PASS" } else { "FAIL" }
        );
        if std::env::var_os("RINKA_APPKIT_SCENE_PROBE_HOLD").is_none() {
            // SAFETY: Diagnostic completion terminates only the current test app.
            unsafe {
                let application: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let _: () = msg_send![application,
                    terminate: std::ptr::null::<AnyObject>()
                ];
            }
        }
    }

    fn begin_transition_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_none()
            || self.ivars().transition_probe.borrow().is_some()
        {
            return;
        }
        let Some(window) = self.ivars().windows.borrow().first().cloned() else {
            return;
        };
        let Some((wide_size, minimum_size)) = self.ivars().transition_sizes else {
            return;
        };
        // SAFETY: The first application window has completed its initial
        // layout and owns the promoted NSSplitViewController.
        unsafe {
            let _: () = msg_send![window.as_object(), setContentSize: wide_size];
            let controller: *mut AnyObject = msg_send![window.as_object(), contentViewController];
            set_split_item_collapsed(controller, 0, false);
            set_split_item_collapsed(controller, 2, false);
            let frame: Rect = msg_send![window.as_object(), frame];
            *self.ivars().transition_probe.borrow_mut() = Some(TransitionProbe {
                step: 0,
                phase: TransitionProbePhase::WidePending,
                baseline: frame,
                wide_size,
                minimum_size,
                attempts: 0,
                observed_split_epoch: self.ivars().split_resize_epoch.get(),
                quiet_turns: 0,
                passed: true,
            });
            self.schedule_transition_probe();
        }
    }

    fn advance_transition_probe(&self) {
        const MAX_SETTLING_TURNS: usize = 200;
        const REQUIRED_QUIET_TURNS: usize = 2;
        let Some(window) = self.ivars().windows.borrow().first().cloned() else {
            return;
        };
        // SAFETY: The probe runs on AppKit's main thread and advances only
        // after the requested split state has actually settled.
        unsafe {
            let controller: *mut AnyObject = msg_send![window.as_object(), contentViewController];
            let frame: Rect = msg_send![window.as_object(), frame];
            let split_resize_epoch = self.ivars().split_resize_epoch.get();
            let split_restore_pending = self.ivars().split_restore_pending.get();
            let outline_state_settled =
                registered_outline_state_is_settled(&self.ivars().list_registries);
            let source_widths = registered_visible_source_widths(&self.ivars().list_registries);
            let action = {
                let mut probe_ref = self.ivars().transition_probe.borrow_mut();
                let Some(probe) = probe_ref.as_mut() else {
                    return;
                };
                if matches!(
                    probe.phase,
                    TransitionProbePhase::WidePending | TransitionProbePhase::MinimumPending
                ) {
                    let target_size = if probe.phase == TransitionProbePhase::WidePending {
                        probe.wide_size
                    } else {
                        probe.minimum_size
                    };
                    let size_matches = rect_size_matches(frame, target_size);
                    let sidebar_collapsed = split_item_collapsed(controller, 0);
                    let inspector_collapsed = split_item_collapsed(controller, 2);
                    let split_is_quiet = probe.observed_split_epoch == split_resize_epoch;
                    if !size_matches
                        || sidebar_collapsed
                        || inspector_collapsed
                        || split_restore_pending
                        || !outline_state_settled
                        || !source_widths.all_widths_resolved
                        || !split_is_quiet
                        || probe.quiet_turns < REQUIRED_QUIET_TURNS
                    {
                        probe.attempts += 1;
                        if split_is_quiet
                            && !split_restore_pending
                            && outline_state_settled
                            && source_widths.all_widths_resolved
                            && size_matches
                            && !sidebar_collapsed
                            && !inspector_collapsed
                        {
                            probe.quiet_turns += 1;
                        } else {
                            probe.observed_split_epoch = split_resize_epoch;
                            probe.quiet_turns = 0;
                        }
                        if probe.attempts >= MAX_SETTLING_TURNS {
                            probe.passed = false;
                            eprintln!(
                                "Rinka transition probe phase={} settlement_timeout frame={frame:?} target_size={target_size:?} sidebar_collapsed={sidebar_collapsed} inspector_collapsed={inspector_collapsed} source_rows_fit={} source_width_resolved={} source_width_capped={}",
                                probe.phase.label(),
                                source_widths.all_rows_fit,
                                source_widths.all_widths_resolved,
                                source_widths.any_width_capped,
                            );
                            TransitionProbeAction::Fail
                        } else if sidebar_collapsed || inspector_collapsed {
                            TransitionProbeAction::RestorePanes
                        } else {
                            TransitionProbeAction::Wait
                        }
                    } else {
                        probe.baseline = frame;
                        probe.step = 0;
                        probe.attempts = 0;
                        probe.quiet_turns = 0;
                        probe.phase = if probe.phase == TransitionProbePhase::WidePending {
                            TransitionProbePhase::Wide
                        } else {
                            TransitionProbePhase::Minimum
                        };
                        eprintln!(
                            "Rinka transition probe phase={} baseline={frame:?} sidebar_collapsed={sidebar_collapsed} inspector_collapsed={inspector_collapsed} pass=true",
                            probe.phase.label()
                        );
                        TransitionProbeAction::ToggleSidebar
                    }
                } else {
                    let Some(expectation) = TRANSITION_PROBE_STEPS.get(probe.step).copied() else {
                        return;
                    };
                    let frame_matches = rect_matches(frame, probe.baseline);
                    let sidebar_collapsed = split_item_collapsed(controller, 0);
                    let inspector_collapsed = split_item_collapsed(controller, 2);
                    let expected_inspector_collapsed = expectation.inspector_collapsed;
                    let state_matches = sidebar_collapsed == expectation.sidebar_collapsed
                        && inspector_collapsed == expected_inspector_collapsed;
                    if !frame_matches {
                        probe.passed = false;
                    }
                    let split_is_quiet = probe.observed_split_epoch == split_resize_epoch;
                    if !state_matches
                        || split_restore_pending
                        || !outline_state_settled
                        || !source_widths.all_widths_resolved
                        || !split_is_quiet
                        || probe.quiet_turns < REQUIRED_QUIET_TURNS
                    {
                        probe.attempts += 1;
                        if split_is_quiet
                            && !split_restore_pending
                            && outline_state_settled
                            && source_widths.all_widths_resolved
                            && state_matches
                        {
                            probe.quiet_turns += 1;
                        } else {
                            probe.observed_split_epoch = split_resize_epoch;
                            probe.quiet_turns = 0;
                        }
                        if probe.attempts >= MAX_SETTLING_TURNS {
                            probe.passed = false;
                            eprintln!(
                                "Rinka transition probe phase={} step={} state={} settlement_timeout expected_sidebar_collapsed={} sidebar_collapsed={} expected_inspector_collapsed={} inspector_collapsed={} frame={frame:?} frame_matches={frame_matches} source_rows_fit={} source_width_resolved={} source_width_capped={}",
                                probe.phase.label(),
                                probe.step,
                                expectation.label,
                                expectation.sidebar_collapsed,
                                sidebar_collapsed,
                                expected_inspector_collapsed,
                                inspector_collapsed,
                                source_widths.all_rows_fit,
                                source_widths.all_widths_resolved,
                                source_widths.any_width_capped,
                            );
                            TransitionProbeAction::Fail
                        } else {
                            TransitionProbeAction::Wait
                        }
                    } else {
                        let step_passed = frame_matches;
                        probe.passed &= step_passed;
                        probe.attempts = 0;
                        probe.quiet_turns = 0;
                        eprintln!(
                            "Rinka transition probe phase={} step={} state={} expected_sidebar_collapsed={} sidebar_collapsed={} expected_inspector_collapsed={} inspector_collapsed={} frame={frame:?} frame_matches={frame_matches} pass={step_passed}",
                            probe.phase.label(),
                            probe.step,
                            expectation.label,
                            expectation.sidebar_collapsed,
                            sidebar_collapsed,
                            expected_inspector_collapsed,
                            inspector_collapsed,
                        );
                        match (probe.phase, expectation.action) {
                            (TransitionProbePhase::Wide, TransitionProbeAction::CompletePhase) => {
                                probe.phase = TransitionProbePhase::MinimumPending;
                                probe.step = 0;
                                TransitionProbeAction::ResizeToMinimum
                            }
                            (
                                TransitionProbePhase::Minimum,
                                TransitionProbeAction::CompletePhase,
                            ) => TransitionProbeAction::Finish,
                            (_, action) => {
                                probe.step += 1;
                                action
                            }
                        }
                    }
                }
            };
            match action {
                TransitionProbeAction::ToggleSidebar => {
                    let _: () = msg_send![controller, toggleSidebar: std::ptr::null::<AnyObject>()];
                    self.schedule_transition_probe();
                }
                TransitionProbeAction::ToggleInspector => {
                    let _: () =
                        msg_send![controller, toggleInspector: std::ptr::null::<AnyObject>()];
                    self.schedule_transition_probe();
                }
                TransitionProbeAction::RestorePanes => {
                    set_split_item_collapsed(controller, 0, false);
                    set_split_item_collapsed(controller, 2, false);
                    self.schedule_transition_probe();
                }
                TransitionProbeAction::ResizeToMinimum => {
                    let minimum = self
                        .ivars()
                        .transition_probe
                        .borrow()
                        .as_ref()
                        .map_or(Size::default(), |probe| probe.minimum_size);
                    self.set_window_content_extent(window.as_object(), minimum);
                    let _: () = msg_send![window.as_object(), setContentSize: minimum];
                    self.schedule_transition_probe();
                }
                TransitionProbeAction::CompletePhase => unreachable!(
                    "phase completion is resolved while the transition probe is borrowed"
                ),
                TransitionProbeAction::Finish | TransitionProbeAction::Fail => {
                    let passed = self
                        .ivars()
                        .transition_probe
                        .borrow()
                        .is_some_and(|probe| probe.passed);
                    eprintln!(
                        "Rinka transition probe result={}",
                        if passed { "PASS" } else { "FAIL" }
                    );
                    if std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE_HOLD").is_none() {
                        let application: *mut AnyObject =
                            msg_send![objc2::class!(NSApplication), sharedApplication];
                        let _: () =
                            msg_send![application, terminate: std::ptr::null::<AnyObject>()];
                    }
                }
                TransitionProbeAction::Wait => self.schedule_transition_probe(),
            }
        }
    }

    fn schedule_transition_probe(&self) {
        // SAFETY: Short main-loop polling observes AppKit's actual native
        // split state instead of assuming an animation duration.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runTransitionProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.02_f64
            ];
        }
    }
}

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
    set_string(window.as_object(), "setTitle:", &spec.title);
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
    let runtime = WindowRuntime::mount(renderer, spec.content.clone())
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
        if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
            let frame: Rect = msg_send![window.as_object(), frame];
            let content: *mut AnyObject = msg_send![window.as_object(), contentView];
            let content_frame: Rect = msg_send![content, frame];
            eprintln!(
                "Rinka AppKit toolbar installed frame={frame:?} content_frame={content_frame:?}"
            );
        }
        let _: () = msg_send![window.as_object(), setContentSize: Size {
            width: spec.initial_size.width,
            height: spec.initial_size.height,
        }];
        if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
            let frame: Rect = msg_send![window.as_object(), frame];
            let content: *mut AnyObject = msg_send![window.as_object(), contentView];
            let content_frame: Rect = msg_send![content, frame];
            eprintln!(
                "Rinka AppKit toolbar size restored frame={frame:?} content_frame={content_frame:?}"
            );
        }
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
        layout_scroll_documents(
            content.as_object(),
            std::env::var_os("RINKA_APPKIT_TRACE").is_some(),
        );
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
    let controller = if matches!(
        handle.element_kind(),
        Some(ElementKind::Split | ElementKind::Workspace)
    ) {
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
        let trace = std::env::var_os("RINKA_APPKIT_TRACE").is_some();
        if trace {
            let root_frame: Rect = msg_send![handle.view(), frame];
            let window_frame: Rect = msg_send![window, frame];
            eprintln!(
                "Rinka AppKit root attach begin root_frame={root_frame:?} window_frame={window_frame:?}"
            );
        }
        let _: () =
            msg_send![controller.as_object(), setPreferredContentSize: initial_content_size];
        let _: () = msg_send![handle.view(), setFrameSize: initial_content_size];
        if trace {
            let root_frame: Rect = msg_send![handle.view(), frame];
            eprintln!("Rinka AppKit root attach sized root_frame={root_frame:?}");
        }
        let _: () = msg_send![window, setContentViewController: controller.as_object()];
        let _: () = msg_send![window, setContentSize: initial_content_size];
        let content: *mut AnyObject = msg_send![window, contentView];
        let _: () = msg_send![content, setFrameSize: initial_content_size];
        // AppKit replaces the root ownership graph while assigning the
        // content-view controller and deactivates constraints attached to the
        // previous graph. Create the retained extent constraints only after
        // that transfer is complete.
        let sizing_view: *mut AnyObject = if matches!(
            handle.element_kind(),
            Some(ElementKind::Split | ElementKind::Workspace)
        ) {
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
        if trace {
            let root_frame: Rect = msg_send![handle.view(), frame];
            let translates: bool =
                msg_send![handle.view(), translatesAutoresizingMaskIntoConstraints];
            eprintln!(
                "Rinka AppKit root attach before-layout root_frame={root_frame:?} translates={translates}"
            );
        }
        if trace {
            let root_frame: Rect = msg_send![handle.view(), frame];
            let window_frame: Rect = msg_send![window, frame];
            eprintln!(
                "Rinka AppKit root attach end root_frame={root_frame:?} window_frame={window_frame:?}"
            );
        }
        finalize_split_mount(handle);
        Ok(initial_extent_constraints)
    }
}

fn finalize_split_mount(handle: &AppKitHandle) {
    let Some(configuration) = *handle.0.split_configuration.borrow() else {
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
            match (configuration, index) {
                (
                    SplitConfiguration::Pair {
                        role: SplitRole::Navigation,
                        collapsible,
                    },
                    0,
                ) => {
                    let _: () = msg_send![item.as_object(), setCollapsed: false];
                    let _: () =
                        msg_send![item.as_object(), setCanCollapseFromWindowResize: collapsible];
                }
                (
                    SplitConfiguration::Workspace {
                        sidebar_collapsible,
                        ..
                    },
                    0,
                ) => {
                    let _: () = msg_send![item.as_object(), setCollapsed: false];
                    let _: () = msg_send![item.as_object(), setCanCollapseFromWindowResize: sidebar_collapsible];
                }
                (
                    SplitConfiguration::Pair {
                        role: SplitRole::Utility,
                        ..
                    },
                    1,
                )
                | (SplitConfiguration::Workspace { .. }, 2) => {
                    let _: () = msg_send![item.as_object(), setCollapsed: false];
                }
                _ => {}
            }
        }
        let _: () = msg_send![handle.view(), layoutSubtreeIfNeeded];
    }
}

unsafe fn layout_scroll_documents(view: &AnyObject, trace: bool) {
    // SAFETY: The traversal stays on the AppKit main thread and only inspects
    // NSView descendants. Scroll document geometry is updated after the
    // enclosing window has its final initial size.
    let is_scroll: bool = unsafe { msg_send![view, isKindOfClass: objc2::class!(NSScrollView)] };
    if is_scroll {
        let document: *mut AnyObject = unsafe { msg_send![view, documentView] };
        if let Some(document) = NonNull::new(document) {
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
            if trace {
                let class_name: *mut AnyObject = unsafe { msg_send![document.as_ref(), className] };
                let before: Rect = unsafe { msg_send![document.as_ref(), frame] };
                eprintln!(
                    "Rinka AppKit scroll document={} content={content_size:?} fitting={fitting_size:?} before={before:?}",
                    rust_string(class_name)
                );
            }
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
            unsafe { layout_scroll_documents(child.as_ref(), trace) };
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

unsafe fn trace_view_tree(view: &AnyObject, depth: usize) {
    if depth > 9 {
        return;
    }
    // SAFETY: Diagnostics only query NSView layout state on the main thread.
    let class_name: *mut AnyObject = unsafe { msg_send![view, className] };
    let frame: Rect = unsafe { msg_send![view, frame] };
    let fitting: Size = unsafe { msg_send![view, fittingSize] };
    let ambiguous: bool = unsafe { msg_send![view, hasAmbiguousLayout] };
    let horizontal_hugging: f32 =
        unsafe { msg_send![view, contentHuggingPriorityForOrientation: 0_isize] };
    let vertical_hugging: f32 =
        unsafe { msg_send![view, contentHuggingPriorityForOrientation: 1_isize] };
    eprintln!(
        "Rinka AppKit view depth={depth} class={} frame={frame:?} fitting={fitting:?} ambiguous={ambiguous} hugging=({horizontal_hugging},{vertical_hugging})",
        rust_string(class_name)
    );
    let is_table: bool = unsafe { msg_send![view, isKindOfClass: objc2::class!(NSTableView)] };
    if is_table {
        let columns: *mut AnyObject = unsafe { msg_send![view, tableColumns] };
        let column_count: usize = unsafe { msg_send![columns, count] };
        let row_count: isize = unsafe { msg_send![view, numberOfRows] };
        let row_height: f64 = unsafe { msg_send![view, rowHeight] };
        let row_size_style: isize = unsafe { msg_send![view, rowSizeStyle] };
        let effective_row_size_style: isize = unsafe { msg_send![view, effectiveRowSizeStyle] };
        let style: isize = unsafe { msg_send![view, style] };
        let effective_style: isize = unsafe { msg_send![view, effectiveStyle] };
        let intercell: Size = unsafe { msg_send![view, intercellSpacing] };
        eprintln!(
            "Rinka AppKit table columns={column_count} rows={row_count} row_height={row_height} row_size_style={row_size_style} effective_row_size_style={effective_row_size_style} style={style} effective_style={effective_style} intercell={intercell:?}"
        );
        for row in 0..row_count.min(8) {
            let row_rect: Rect = unsafe { msg_send![view, rectOfRow: row] };
            eprintln!("Rinka AppKit table row={row} rect={row_rect:?}");
        }
        for index in 0..column_count {
            let column: *mut AnyObject = unsafe { msg_send![columns, objectAtIndex: index] };
            let title: *mut AnyObject = unsafe { msg_send![column, title] };
            let width: f64 = unsafe { msg_send![column, width] };
            let minimum: f64 = unsafe { msg_send![column, minWidth] };
            eprintln!(
                "Rinka AppKit table column={index} title={} width={width} minimum={minimum}",
                rust_string(title)
            );
        }
    }
    let subviews: *mut AnyObject = unsafe { msg_send![view, subviews] };
    let count: usize = unsafe { msg_send![subviews, count] };
    for index in 0..count {
        let child: *mut AnyObject = unsafe { msg_send![subviews, objectAtIndex: index] };
        if let Some(child) = NonNull::new(child) {
            unsafe { trace_view_tree(child.as_ref(), depth + 1) };
        }
    }
}

unsafe fn trace_window_contract(window: &AnyObject) {
    // SAFETY: Diagnostics query public NSWindow, view-controller, split-item,
    // and toolbar properties on the AppKit main thread.
    let frame: Rect = unsafe { msg_send![window, frame] };
    let min_size: Size = unsafe { msg_send![window, minSize] };
    let max_size: Size = unsafe { msg_send![window, maxSize] };
    let content_min_size: Size = unsafe { msg_send![window, contentMinSize] };
    let content_max_size: Size = unsafe { msg_send![window, contentMaxSize] };
    eprintln!(
        "Rinka AppKit contract frame={frame:?} min_size={min_size:?} max_size={max_size:?} content_min_size={content_min_size:?} content_max_size={content_max_size:?}"
    );
    let controller: *mut AnyObject = unsafe { msg_send![window, contentViewController] };
    let Some(controller) = NonNull::new(controller) else {
        eprintln!("Rinka AppKit contract content_controller=nil");
        return;
    };
    let controller_class: *mut AnyObject = unsafe { msg_send![controller.as_ref(), className] };
    let is_split: bool = unsafe {
        msg_send![controller.as_ref(), isKindOfClass: objc2::class!(NSSplitViewController)]
    };
    eprintln!(
        "Rinka AppKit contract content_controller={} split={is_split}",
        rust_string(controller_class)
    );
    if is_split {
        let items: *mut AnyObject = unsafe { msg_send![controller.as_ref(), splitViewItems] };
        let count: usize = unsafe { msg_send![items, count] };
        eprintln!("Rinka AppKit contract split_items={count}");
        for index in 0..count {
            let item: *mut AnyObject = unsafe { msg_send![items, objectAtIndex: index] };
            let behavior: isize = unsafe { msg_send![item, behavior] };
            let collapsed: bool = unsafe { msg_send![item, isCollapsed] };
            let can_collapse: bool = unsafe { msg_send![item, canCollapse] };
            let resize_collapse: bool = unsafe { msg_send![item, canCollapseFromWindowResize] };
            let automatic_safe_area: bool =
                unsafe { msg_send![item, automaticallyAdjustsSafeAreaInsets] };
            let item_controller: *mut AnyObject = unsafe { msg_send![item, viewController] };
            let parent: *mut AnyObject =
                unsafe { msg_send![item_controller, parentViewController] };
            let view: *mut AnyObject = unsafe { msg_send![item_controller, view] };
            let frame: Rect = unsafe { msg_send![view, frame] };
            eprintln!(
                "Rinka AppKit contract split_item={index} behavior={behavior} collapsed={collapsed} can_collapse={can_collapse} resize_collapse={resize_collapse} automatic_safe_area={automatic_safe_area} parent_is_root={} frame={frame:?}",
                parent == controller.as_ptr()
            );
        }
    }
    let toolbar: *mut AnyObject = unsafe { msg_send![window, toolbar] };
    let Some(toolbar) = NonNull::new(toolbar) else {
        eprintln!("Rinka AppKit contract toolbar=nil");
        return;
    };
    let items: *mut AnyObject = unsafe { msg_send![toolbar.as_ref(), items] };
    let count: usize = unsafe { msg_send![items, count] };
    eprintln!("Rinka AppKit contract toolbar_items={count}");
    for index in 0..count {
        let item: *mut AnyObject = unsafe { msg_send![items, objectAtIndex: index] };
        let identifier: *mut AnyObject = unsafe { msg_send![item, itemIdentifier] };
        let target: *mut AnyObject = unsafe { msg_send![item, target] };
        let item_class: *mut AnyObject = unsafe { msg_send![item, className] };
        eprintln!(
            "Rinka AppKit contract toolbar_item={index} class={} identifier={} target_nil={}",
            rust_string(item_class),
            rust_string(identifier),
            target.is_null()
        );
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
        .and_then(|root| split_controller_for(root, SplitRole::Navigation));
    let inspector_controller = renderer
        .mounted()
        .and_then(|root| split_controller_for(root, SplitRole::Utility));
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

const fn native_toolbar_group_display(display: ToolbarGroupDisplay) -> isize {
    match display {
        ToolbarGroupDisplay::Automatic => 0,
        ToolbarGroupDisplay::Expanded => 1,
        ToolbarGroupDisplay::Collapsed => 2,
    }
}

fn split_controller_for(node: &MountedNode<AppKitHandle>, role: SplitRole) -> Option<Id> {
    if node.handle().element_kind() == Some(ElementKind::Workspace)
        || node.handle().0.split_role == Some(role)
    {
        return node.handle().0.auxiliaries.first().cloned();
    }
    node.children()
        .iter()
        .find_map(|child| split_controller_for(child, role))
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

fn mounted_scene(node: &MountedNode<AppKitHandle>) -> Option<&'static str> {
    [
        ("file-list", "ready"),
        ("directory-empty", "empty"),
        ("directory-busy", "busy"),
        ("directory-error", "error"),
    ]
    .into_iter()
    .find_map(|(key, scene)| mounted_handle_for_key(node, key).map(|_| scene))
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
        eprintln!(
            "Rinka geometry invalid view_class={} frame={frame:?} ambiguous={ambiguous} translates={translates}",
            rust_string(class_name)
        );
        return false;
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

fn new_object(class: &objc2::runtime::AnyClass) -> Id {
    // SAFETY: Every caller passes an NSObject subclass with init.
    unsafe {
        let allocated: *mut AnyObject = msg_send![class, alloc];
        let pointer: *mut AnyObject = msg_send![allocated, init];
        Id::from_owned(pointer)
    }
}

/// Runs the application on AppKit's main loop.
pub fn run(application: ApplicationSpec) {
    autoreleasepool(|_| {
        let mtm = MainThreadMarker::new().expect("AppKit must start on the process main thread");
        // SAFETY: sharedApplication is the AppKit singleton on the main thread.
        let app: *mut AnyObject =
            unsafe { msg_send![objc2::class!(NSApplication), sharedApplication] };
        let delegate = ApplicationDelegate::new(mtm, application);
        // SAFETY: NSApplication keeps a non-owning delegate pointer; the retained
        // local lives until run returns. AppKit must complete launch and state
        // restoration before application-owned windows are constructed.
        unsafe {
            let _: () = msg_send![app, setDelegate: &*delegate];
            let _: () = msg_send![app, finishLaunching];
        }
        if let Err(exception) = objc2::exception::catch(AssertUnwindSafe(|| {
            delegate.show_initial_windows();
        })) {
            panic!("AppKit rejected the native view tree: {exception:?}");
        }
        unsafe {
            let _: () = msg_send![app, run];
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        DiagnosticAppearance, TRANSITION_PROBE_STEPS, TransitionProbeAction,
        parse_diagnostic_appearance, should_install_toolbar,
    };
    use rinka_core::{PanelBehavior, WindowKind};

    #[test]
    fn workspace_split_controls_install_a_toolbar_without_custom_items() {
        assert!(should_install_toolbar(WindowKind::Main, 0, true));
        assert!(!should_install_toolbar(WindowKind::Main, 0, false));
    }

    #[test]
    fn panels_do_not_install_a_toolbar() {
        assert!(!should_install_toolbar(
            WindowKind::Panel(PanelBehavior {
                floating: true,
                hides_when_inactive: false,
                accepts_keyboard: true,
            }),
            1,
            true,
        ));
    }

    #[test]
    fn transition_probe_covers_three_combined_cycles_per_size_phase() {
        assert_eq!(TRANSITION_PROBE_STEPS.len(), 24);
        assert_eq!(
            TRANSITION_PROBE_STEPS
                .iter()
                .filter(|step| step.sidebar_collapsed && step.inspector_collapsed)
                .count(),
            3
        );
        assert_eq!(
            TRANSITION_PROBE_STEPS.last().unwrap().action,
            TransitionProbeAction::CompletePhase
        );
        assert!(
            TRANSITION_PROBE_STEPS
                .last()
                .is_some_and(|step| { !step.sidebar_collapsed && !step.inspector_collapsed })
        );
    }

    #[test]
    fn diagnostic_appearance_accepts_only_explicit_matrix_values() {
        assert_eq!(parse_diagnostic_appearance(None).unwrap(), None);
        assert_eq!(
            parse_diagnostic_appearance(Some("light")).unwrap(),
            Some(DiagnosticAppearance::Light)
        );
        assert_eq!(
            parse_diagnostic_appearance(Some("dark")).unwrap(),
            Some(DiagnosticAppearance::Dark)
        );
        assert!(parse_diagnostic_appearance(Some("system")).is_err());
    }
}
