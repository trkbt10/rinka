// NSApplication main-menu realization of the declarative menu bar.
//
// Routing decision, recorded for `reports/app-menu-bar`: the installed
// NSMenu tree is realized from the *effective* declaration resolved by
// `rinka_core::MenuBarRouter` — the key window's content-root declaration,
// falling back to the first declaring window (the main window), then to the
// `ApplicationSpec` bar. Every app-defined NSMenuItem targets a
// `MenuBarTarget` carrying only the item identity; activation resolves the
// key window *at dispatch time* and routes through the router into that
// window's live `MenuBarBindings`, whose handlers dispatch into the focused
// window's queued message delivery. A stale native menu therefore always
// dispatches through the freshest declaration, and the model's enabled state
// re-gates every dispatch.
//
// Enabled state is served through native menu validation: the menus keep
// AppKit's `autoenablesItems`, `MenuBarTarget` answers `validateMenuItem:`
// from the declarative model, and standard nil-target items validate down
// the responder chain — which is what makes Edit>Copy work against a native
// field with zero consumer code. This diverges deliberately from
// `create_ns_menu`'s popup discipline (autoenables off): the menu bar hosts
// responder-chain roles, so validation-driven enabling is the native
// contract here.
//
// About and Quit realize in the synthesized application menu (the bold
// app-name menu) regardless of their declared position, which macOS reserves
// for them; a declared `StandardItem::About`/`Quit` slot is skipped on this
// host and kept for hosts whose conventions place them in File or Help.

/// Application-menu-bar state shared by the delegate, the key monitor, and
/// every native menu item target.
#[derive(Clone)]
struct MenuBarHost(Rc<MenuBarHostInner>);

struct MenuBarHostInner {
    mtm: MainThreadMarker,
    application_name: String,
    router: RefCell<MenuBarRouter>,
    window_identities: WindowIdentityRegistry,
    installed: RefCell<Option<InstalledMenuBar>>,
    log_activations: bool,
}

/// Retained native realization paired with the model it was built from.
struct InstalledMenuBar {
    owner: Option<WindowId>,
    model: MenuBar,
    root: Id,
}

impl fmt::Debug for MenuBarHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MenuBarHost")
            .field("application_name", &self.0.application_name)
            .field("installed", &self.0.installed.borrow().is_some())
            .finish_non_exhaustive()
    }
}

impl MenuBarHost {
    fn new(
        mtm: MainThreadMarker,
        application_name: String,
        application_bar: MenuBar,
        window_identities: WindowIdentityRegistry,
    ) -> Self {
        if let Err(reason) = application_bar.validate() {
            panic!("invalid application menu bar: {reason}");
        }
        let log_activations = std::env::var_os("RINKA_APPKIT_MENU_BAR_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_ACCELERATOR_PROBE").is_some();
        Self(Rc::new(MenuBarHostInner {
            mtm,
            application_name,
            router: RefCell::new(MenuBarRouter::new(application_bar)),
            window_identities,
            installed: RefCell::new(None),
            log_activations,
        }))
    }

    /// Registers one window's stable menu bar slot with the router.
    fn register_window(&self, id: WindowId, bindings: MenuBarBindings) {
        self.0.router.borrow_mut().register_window(id, bindings);
    }

    /// Resolves the key window's declarative identity through the delegate's
    /// pointer registry.
    fn key_window_id(&self) -> Option<WindowId> {
        // SAFETY: keyWindow is a main-thread NSApplication read; the registry
        // pairs retained window pointers with their declared identities.
        unsafe {
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let key: *mut AnyObject = msg_send![application, keyWindow];
            self.0
                .window_identities
                .borrow()
                .iter()
                .find_map(|(pointer, id)| (*pointer == key as usize).then(|| id.clone()))
        }
    }

    /// Returns whether the effective bar owns this chord.
    ///
    /// Precedence decision, recorded for `reports/app-menu-bar`: a chord
    /// bound by any effective menu-bar item is menu-owned. The application
    /// key monitor returns the event untouched so AppKit's native menu
    /// key-equivalent dispatch fires it — exactly once, with the native menu
    /// flash, and (natively) even over focused text input. A same-chord
    /// window accelerator entry is fully shadowed while the menu item
    /// exists, including its `global`/withhold flags; the accelerator
    /// table's defer-to-typing policy governs only chords without a menu
    /// home.
    fn claims_chord(&self, key_window: Option<&WindowId>, chord: KeyChord) -> bool {
        self.0.router.borrow().claims_chord(key_window, chord)
    }

    /// Answers native menu validation for one app-defined item.
    fn item_enabled(&self, item_id: &str) -> bool {
        let key_window = self.key_window_id();
        self.0.router.borrow().item_enabled(key_window.as_ref(), item_id)
    }

    /// Dispatches one app-defined item activation to the focused window.
    fn activate(&self, item_id: &str) {
        let key_window = self.key_window_id();
        let outcome = self.0.router.borrow().activate(key_window.as_ref(), item_id);
        if self.0.log_activations {
            let outcome_text = match &outcome {
                MenuBarActivation::Dispatched { owner: Some(owner) } => {
                    format!("dispatched window={}", owner.as_str())
                }
                MenuBarActivation::Dispatched { owner: None } => {
                    "dispatched window=application".to_owned()
                }
                MenuBarActivation::Refused => "refused".to_owned(),
                MenuBarActivation::Unknown => "unknown".to_owned(),
            };
            eprintln!(
                "Rinka menu-bar activation item={item_id} key_window={} outcome={outcome_text}",
                key_window.as_ref().map_or("none", |id| id.as_str())
            );
        }
    }

    /// Installs or updates NSApplication's main menu from the effective
    /// declaration, preserving the retained native tree when the structure
    /// is unchanged.
    fn refresh(&self) {
        let key_window = self.key_window_id();
        let effective = self.0.router.borrow().effective_model(key_window.as_ref());
        let mut installed = self.0.installed.borrow_mut();
        match (installed.take(), effective) {
            (None, None) => {}
            (Some(_), None) => {
                // SAFETY: Clearing platform menu designations on the shared
                // application is a main-thread call.
                unsafe {
                    let application: *mut AnyObject =
                        msg_send![objc2::class!(NSApplication), sharedApplication];
                    let _: () =
                        msg_send![application, setMainMenu: std::ptr::null::<AnyObject>()];
                    let _: () =
                        msg_send![application, setWindowsMenu: std::ptr::null::<AnyObject>()];
                    let _: () =
                        msg_send![application, setHelpMenu: std::ptr::null::<AnyObject>()];
                }
            }
            (None, Some((owner, model))) => {
                *installed = Some(self.install_native(owner, model));
            }
            (Some(previous), Some((owner, model))) => {
                if previous.owner == owner && previous.model == model {
                    *installed = Some(previous);
                    return;
                }
                match MenuBar::plan_update(&previous.model, &model) {
                    MenuBarUpdate::Unchanged => {
                        // Handlers and ownership resolve live at dispatch
                        // time; nothing native changes.
                        *installed = Some(InstalledMenuBar {
                            owner,
                            model,
                            root: previous.root,
                        });
                    }
                    MenuBarUpdate::RefreshInPlace => {
                        // SAFETY: The retained root was realized from a model
                        // whose structure matches the next model.
                        unsafe {
                            refresh_menu_bar_menus(&previous.root, &model);
                        }
                        *installed = Some(InstalledMenuBar {
                            owner,
                            model,
                            root: previous.root,
                        });
                    }
                    MenuBarUpdate::Rebuild => {
                        *installed = Some(self.install_native(owner, model));
                    }
                }
            }
        }
    }

    fn install_native(&self, owner: Option<WindowId>, model: MenuBar) -> InstalledMenuBar {
        let root = self.build_main_menu(&model);
        // SAFETY: Installing the freshly built menu on the shared application
        // is a main-thread call; NSApplication retains its main menu.
        unsafe {
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let _: () = msg_send![application, setMainMenu: root.as_object()];
        }
        InstalledMenuBar { owner, model, root }
    }

    /// Builds the complete main menu: the synthesized application menu
    /// followed by one native menu per declared top-level menu.
    fn build_main_menu(&self, model: &MenuBar) -> Id {
        // SAFETY: All construction happens on the main thread over freshly
        // allocated AppKit objects; NSMenu retains every added item. The
        // main menu and every submenu keep autoenablesItems so nil-target
        // standard roles validate down the responder chain and app-defined
        // items validate through MenuBarTarget.
        unsafe {
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let _: () = msg_send![application, setWindowsMenu: std::ptr::null::<AnyObject>()];
            let _: () = msg_send![application, setHelpMenu: std::ptr::null::<AnyObject>()];

            let root = new_ns_menu("");
            let app_item = self.build_application_menu_item();
            let _: () = msg_send![root.as_object(), addItem: app_item.as_object()];
            for menu in &model.menus {
                let holder = submenu_holder_item(&menu.label, true);
                let submenu = new_ns_menu(&menu.label);
                self.append_menu_bar_entries(&submenu, &menu.entries);
                let _: () = msg_send![holder.as_object(), setSubmenu: submenu.as_object()];
                let _: () = msg_send![root.as_object(), addItem: holder.as_object()];
                match menu.role {
                    MenuBarMenuRole::Custom => {}
                    MenuBarMenuRole::Window => {
                        let _: () =
                            msg_send![application, setWindowsMenu: submenu.as_object()];
                    }
                    MenuBarMenuRole::Help => {
                        let _: () = msg_send![application, setHelpMenu: submenu.as_object()];
                    }
                }
            }
            root
        }
    }

    /// Builds the application menu (the bold app-name menu): About, Hide,
    /// Hide Others, Show All, and Quit through their native selectors.
    fn build_application_menu_item(&self) -> Id {
        let name = &self.0.application_name;
        // SAFETY: All construction happens on the main thread; the standard
        // selectors are handled by NSApplication through the nil-target
        // action path.
        unsafe {
            let menu = new_ns_menu(name);
            let about = native_selector_item(
                &format!("About {name}"),
                sel!(orderFrontStandardAboutPanel:),
                None,
            );
            let _: () = msg_send![menu.as_object(), addItem: about.as_object()];
            add_ns_menu_separator(&menu);
            let hide = native_selector_item(
                &format!("Hide {name}"),
                sel!(hide:),
                Some(standard_chord("Primary+H")),
            );
            let _: () = msg_send![menu.as_object(), addItem: hide.as_object()];
            let hide_others = native_selector_item(
                "Hide Others",
                sel!(hideOtherApplications:),
                Some(standard_chord("Primary+Alt+H")),
            );
            let _: () = msg_send![menu.as_object(), addItem: hide_others.as_object()];
            let show_all =
                native_selector_item("Show All", sel!(unhideAllApplications:), None);
            let _: () = msg_send![menu.as_object(), addItem: show_all.as_object()];
            add_ns_menu_separator(&menu);
            let quit = native_selector_item(
                &format!("Quit {name}"),
                sel!(terminate:),
                Some(standard_chord("Primary+Q")),
            );
            let _: () = msg_send![menu.as_object(), addItem: quit.as_object()];

            let holder = submenu_holder_item(name, true);
            // SAFETY: NSMenuItem retains its submenu.
            let _: () = msg_send![holder.as_object(), setSubmenu: menu.as_object()];
            holder
        }
    }

    /// Appends declared menu bar entries onto a native menu.
    fn append_menu_bar_entries(&self, menu: &Id, entries: &[MenuBarEntry]) {
        for entry in entries {
            match entry {
                MenuBarEntry::Separator => add_ns_menu_separator(menu),
                MenuBarEntry::Item(item) => self.append_app_defined_item(menu, item),
                MenuBarEntry::Standard(standard) => {
                    if let Some(native) = standard_ns_menu_item(*standard) {
                        // SAFETY: NSMenu retains the inserted item.
                        unsafe {
                            let _: () = msg_send![menu.as_object(), addItem: native.as_object()];
                        }
                    }
                }
                MenuBarEntry::Submenu(submenu) => {
                    let holder = submenu_holder_item(&submenu.label, submenu.enabled);
                    let nested = new_ns_menu(&submenu.label);
                    self.append_nested_menu_entries(&nested, &submenu.entries);
                    // SAFETY: NSMenuItem retains its submenu and NSMenu
                    // retains the inserted item.
                    unsafe {
                        let _: () = msg_send![holder.as_object(), setSubmenu: nested.as_object()];
                        let _: () = msg_send![menu.as_object(), addItem: holder.as_object()];
                    }
                }
            }
        }
    }

    /// Appends shared-vocabulary entries of a nested submenu, each item
    /// targeting the menu bar dispatch.
    fn append_nested_menu_entries(&self, menu: &Id, entries: &[MenuEntry]) {
        for entry in entries {
            match entry {
                MenuEntry::Separator => add_ns_menu_separator(menu),
                MenuEntry::Item(item) => self.append_app_defined_item(menu, item),
                MenuEntry::Submenu(submenu) => {
                    let holder = submenu_holder_item(&submenu.label, submenu.enabled);
                    let nested = new_ns_menu(&submenu.label);
                    self.append_nested_menu_entries(&nested, &submenu.entries);
                    // SAFETY: NSMenuItem retains its submenu and NSMenu
                    // retains the inserted item.
                    unsafe {
                        let _: () = msg_send![holder.as_object(), setSubmenu: nested.as_object()];
                        let _: () = msg_send![menu.as_object(), addItem: holder.as_object()];
                    }
                }
            }
        }
    }

    fn append_app_defined_item(&self, menu: &Id, item: &MenuItem) {
        let target = MenuBarTarget::new(self.0.mtm, self.clone(), item.id.clone());
        // The item retains the target through representedObject; enabled
        // state is served by validateMenuItem: from the live model.
        let native = create_ns_menu_item(item, true, &*target, sel!(performMenuBarAction:));
        // SAFETY: NSMenu retains the inserted item.
        unsafe {
            let _: () = msg_send![menu.as_object(), addItem: native.as_object()];
        }
    }
}

/// Creates a menu with AppKit's automatic item validation left enabled.
///
/// The popup builder `create_ns_menu` disables autoenabling because popup
/// items are all target-bound; the menu bar keeps it so nil-target standard
/// roles are validated down the responder chain natively.
fn new_ns_menu(title: &str) -> Id {
    let title = ns_string(title);
    // SAFETY: initWithTitle: is NSMenu's designated initializer.
    unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSMenu), alloc];
        let pointer: *mut AnyObject = msg_send![allocated, initWithTitle: title.as_object()];
        Id::from_owned(pointer)
    }
}

fn add_ns_menu_separator(menu: &Id) {
    // SAFETY: separatorItem returns a shared autoreleased item and NSMenu
    // retains every item it contains.
    unsafe {
        let separator: *mut AnyObject = msg_send![objc2::class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu.as_object(), addItem: separator];
    }
}

/// Creates an action-less item that holds a submenu.
fn submenu_holder_item(title: &str, enabled: bool) -> Id {
    let title = ns_string(title);
    let key = ns_string("");
    // SAFETY: The item is created through the designated initializer with a
    // nil action.
    unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSMenuItem), alloc];
        let pointer: *mut AnyObject = msg_send![allocated,
            initWithTitle: title.as_object(),
            action: None::<objc2::runtime::Sel>,
            keyEquivalent: key.as_object()
        ];
        let native = Id::from_owned(pointer);
        let _: () = msg_send![native.as_object(), setEnabled: enabled];
        native
    }
}

/// Creates a nil-target item dispatching a native selector down the
/// responder chain.
fn native_selector_item(title: &str, action: objc2::runtime::Sel, chord: Option<KeyChord>) -> Id {
    let title = ns_string(title);
    let key = ns_string("");
    // SAFETY: The item is created through the designated initializer; a nil
    // target sends the action down the responder chain, AppKit's contract
    // for standard menu roles.
    unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSMenuItem), alloc];
        let pointer: *mut AnyObject = msg_send![allocated,
            initWithTitle: title.as_object(),
            action: action,
            keyEquivalent: key.as_object()
        ];
        let native = Id::from_owned(pointer);
        if let Some(chord) = chord {
            apply_menu_item_chord(native.as_object(), chord);
        }
        native
    }
}

fn standard_chord(text: &str) -> KeyChord {
    text.parse().expect("standard menu chords are canonical")
}

/// Realizes one standard role as its native nil-target item.
///
/// About and Quit return `None` here: on macOS they live in the synthesized
/// application menu regardless of their declared position.
fn standard_ns_menu_item(item: StandardItem) -> Option<Id> {
    let (title, action) = match item {
        StandardItem::About | StandardItem::Quit => return None,
        StandardItem::CloseWindow => ("Close Window", sel!(performClose:)),
        StandardItem::Minimize => ("Minimize", sel!(performMiniaturize:)),
        StandardItem::Undo => ("Undo", sel!(undo:)),
        StandardItem::Redo => ("Redo", sel!(redo:)),
        StandardItem::Cut => ("Cut", sel!(cut:)),
        StandardItem::Copy => ("Copy", sel!(copy:)),
        StandardItem::Paste => ("Paste", sel!(paste:)),
        StandardItem::SelectAll => ("Select All", sel!(selectAll:)),
    };
    Some(native_selector_item(title, action, item.canonical_chord()))
}

/// Updates a structurally unchanged retained main menu to the next model:
/// titles, checkmarks, help, symbols, and key equivalents. Enabled state is
/// deliberately left to native validation, except on submenu holders, which
/// AppKit does not validate through a target.
///
/// # Safety
///
/// `root` must be the retained main menu realized from a model whose
/// structure matches `model`, as established by [`MenuBar::plan_update`].
unsafe fn refresh_menu_bar_menus(root: &Id, model: &MenuBar) {
    for (index, menu) in model.menus.iter().enumerate() {
        // Item 0 is the synthesized application menu; declared menus follow.
        let Ok(native_index) = isize::try_from(index + 1) else {
            return;
        };
        // SAFETY: The caller guarantees the index is within the item count.
        let holder: *mut AnyObject =
            unsafe { msg_send![root.as_object(), itemAtIndex: native_index] };
        let Some(holder) = NonNull::new(holder) else {
            continue;
        };
        let title = ns_string(&menu.label);
        // SAFETY: The holder and its submenu are live retained objects.
        unsafe {
            let _: () = msg_send![holder.as_ref(), setTitle: title.as_object()];
            let submenu: *mut AnyObject = msg_send![holder.as_ref(), submenu];
            if let Some(submenu) = NonNull::new(submenu) {
                let _: () = msg_send![submenu.as_ref(), setTitle: title.as_object()];
                refresh_menu_bar_entries(submenu.as_ref(), &menu.entries);
            }
        }
    }
}

/// Refreshes one realized menu's items from its declared entries.
///
/// # Safety
///
/// `menu` must be a live NSMenu whose item sequence matches `entries`.
unsafe fn refresh_menu_bar_entries(menu: &AnyObject, entries: &[MenuBarEntry]) {
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
                MenuBarEntry::Separator | MenuBarEntry::Standard(_) => {}
                MenuBarEntry::Item(item) => {
                    refresh_app_defined_item(native.as_ref(), item);
                }
                MenuBarEntry::Submenu(submenu) => {
                    let title = ns_string(&submenu.label);
                    let _: () = msg_send![native.as_ref(), setTitle: title.as_object()];
                    let _: () = msg_send![native.as_ref(), setEnabled: submenu.enabled];
                    let nested: *mut AnyObject = msg_send![native.as_ref(), submenu];
                    if let Some(nested) = NonNull::new(nested) {
                        let _: () = msg_send![nested.as_ref(), setTitle: title.as_object()];
                        refresh_nested_menu_entries(nested.as_ref(), &submenu.entries);
                    }
                }
            }
        }
    }
}

/// Refreshes shared-vocabulary entries inside a nested submenu.
///
/// # Safety
///
/// `menu` must be a live NSMenu whose item sequence matches `entries`.
unsafe fn refresh_nested_menu_entries(menu: &AnyObject, entries: &[MenuEntry]) {
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
                MenuEntry::Item(item) => refresh_app_defined_item(native.as_ref(), item),
                MenuEntry::Submenu(submenu) => {
                    let title = ns_string(&submenu.label);
                    let _: () = msg_send![native.as_ref(), setTitle: title.as_object()];
                    let _: () = msg_send![native.as_ref(), setEnabled: submenu.enabled];
                    let nested: *mut AnyObject = msg_send![native.as_ref(), submenu];
                    if let Some(nested) = NonNull::new(nested) {
                        let _: () = msg_send![nested.as_ref(), setTitle: title.as_object()];
                        refresh_nested_menu_entries(nested.as_ref(), &submenu.entries);
                    }
                }
            }
        }
    }
}

/// Refreshes one app-defined item's comparable state on its retained native
/// item; enabled state stays with `validateMenuItem:`.
///
/// # Safety
///
/// `native` must be a live NSMenuItem realized from an item with this
/// identity.
unsafe fn refresh_app_defined_item(native: &AnyObject, item: &MenuItem) {
    let title = ns_string(&item.label);
    // SAFETY: All properties are public NSMenuItem API on the main thread.
    unsafe {
        let _: () = msg_send![native, setTitle: title.as_object()];
        let _: () = msg_send![native, setState: isize::from(item.checked)];
        set_string(native, "setToolTip:", &item.help);
        match item.symbol.and_then(system_image) {
            Some(image) => {
                let _: () = msg_send![native, setImage: image.as_object()];
            }
            None => {
                let _: () = msg_send![native, setImage: std::ptr::null::<AnyObject>()];
            }
        }
        match item.chord {
            Some(chord) => apply_menu_item_chord(native, chord),
            None => {
                let empty = ns_string("");
                let _: () = msg_send![native, setKeyEquivalent: empty.as_object()];
                let _: () = msg_send![native, setKeyEquivalentModifierMask: 0_usize];
            }
        }
    }
}

struct MenuBarTargetIvars {
    host: MenuBarHost,
    item_id: String,
}

impl fmt::Debug for MenuBarTargetIvars {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MenuBarTargetIvars")
            .field("item_id", &self.item_id)
            .finish_non_exhaustive()
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuBarTargetIvars]
    struct MenuBarTarget;

    // SAFETY: NSObjectProtocol adds no invariants beyond the NSObject superclass.
    unsafe impl NSObjectProtocol for MenuBarTarget {}

    impl MenuBarTarget {
        #[unsafe(method(performMenuBarAction:))]
        fn perform_menu_bar_action(&self, _sender: &AnyObject) {
            self.ivars().host.activate(&self.ivars().item_id);
        }

        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, _item: &AnyObject) -> bool {
            self.ivars().host.item_enabled(&self.ivars().item_id)
        }
    }
);

impl MenuBarTarget {
    fn new(mtm: MainThreadMarker, host: MenuBarHost, item_id: String) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(MenuBarTargetIvars { host, item_id });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }
}
