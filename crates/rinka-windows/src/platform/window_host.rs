/// Runs the native Windows application until its last top-level window closes.
pub fn run(application: ApplicationSpec) -> Result<(), WindowsDiagnostic> {
    if !application.menu_bar.is_empty() {
        // The Win32 contract probe has no HMENU application-menu-bar
        // realization (reports/app-menu-bar); the declared bar is rejected
        // instead of silently dropped.
        return Err(WindowsDiagnostic::UnsupportedApplicationCapability {
            capability: "application menu bar",
        });
    }
    let _apartment = initialize_native_process()?;
    let instance = module_instance()?;
    register_window_class(instance)?;
    let mut primary_windows = Vec::new();
    let mut panels = Vec::new();
    for window in application.windows {
        if matches!(window.kind, WindowKind::Panel(_)) {
            panels.push(window);
        } else {
            primary_windows.push(window);
        }
    }
    let mut main_window: HWND = null_mut();
    let mut created_windows = Vec::new();
    for window in primary_windows.into_iter().chain(panels) {
        let owner = match window.kind {
            WindowKind::Panel(PanelBehavior { floating: true, .. }) => main_window,
            _ => null_mut(),
        };
        let is_first_main = main_window.is_null() && matches!(window.kind, WindowKind::Main);
        let hwnd = match create_host_window(instance, owner, window) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                for created in created_windows.into_iter().rev() {
                    // SAFETY: these HWND values were created on this thread and are still live.
                    unsafe {
                        let _ = DestroyWindow(created);
                    }
                }
                return Err(error);
            }
        };
        created_windows.push(hwnd);
        if is_first_main {
            main_window = hwnd;
        }
    }
    if created_windows.is_empty() {
        return Err(WindowsDiagnostic::InvalidNativeState {
            reason: "Windows application requires at least one top-level window".to_owned(),
        });
    }
    MESSAGE_LOOP_ACTIVE.with(|active| active.set(true));
    // SAFETY: the message loop owns all created HWND values on the current thread.
    unsafe {
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, null_mut(), 0, 0);
            if result == -1 {
                MESSAGE_LOOP_ACTIVE.with(|active| active.set(false));
                return Err(last_error("GetMessageW"));
            }
            if result == 0 {
                break;
            }
            let active_window = GetActiveWindow();
            if !active_window.is_null() && IsDialogMessageW(active_window, &message) != 0 {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    MESSAGE_LOOP_ACTIVE.with(|active| active.set(false));
    Ok(())
}

fn initialize_native_process() -> Result<ComApartment, WindowsDiagnostic> {
    // SAFETY: this executes before any HWND is created on the process UI thread.
    unsafe {
        if let Err(error) =
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            && !AreDpiAwarenessContextsEqual(
                GetThreadDpiAwarenessContext(),
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            )
            .as_bool()
        {
            return Err(WindowsDiagnostic::InvalidNativeState {
                reason: format!("PerMonitorV2 DPI initialization failed: {error}"),
            });
        }
        let controls = INITCOMMONCONTROLSEX {
            dwSize: u32::try_from(size_of::<INITCOMMONCONTROLSEX>()).unwrap_or(u32::MAX),
            dwICC: ICC_BAR_CLASSES
                | ICC_LISTVIEW_CLASSES
                | ICC_PROGRESS_CLASS
                | ICC_TREEVIEW_CLASSES,
        };
        if InitCommonControlsEx(&controls) == 0 {
            return Err(last_error("InitCommonControlsEx"));
        }
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|error| WindowsDiagnostic::InvalidNativeState {
                reason: format!("COM apartment initialization failed: {error}"),
            })?;
    }
    Ok(ComApartment)
}

fn register_window_class(instance: HINSTANCE) -> Result<(), WindowsDiagnostic> {
    let class_name = wide(CLASS_NAME);
    // SAFETY: the class structure references local UTF-16 storage only for the synchronous call.
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(u32::MAX),
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: LoadIconW(null_mut(), IDI_APPLICATION),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (6usize) as HBRUSH,
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: LoadIconW(null_mut(), IDI_APPLICATION),
        };
        if RegisterClassExW(&class) == 0 {
            let code = GetLastError();
            if code != 1410 {
                return Err(WindowsDiagnostic::NativeOperation {
                    operation: "RegisterClassExW",
                    code,
                });
            }
        }
    }
    Ok(())
}

fn create_host_window(
    instance: HINSTANCE,
    owner: HWND,
    spec: WindowSpec,
) -> Result<HWND, WindowsDiagnostic> {
    let dark = dark_appearance();
    let class_name = wide(CLASS_NAME);
    let title = wide(&spec.title);
    let panel_behavior = match spec.kind {
        WindowKind::Panel(behavior) => Some(behavior),
        WindowKind::Main | WindowKind::Preferences => None,
    };
    let (extended, style) = match panel_behavior {
        None => (0, WS_OVERLAPPEDWINDOW),
        Some(behavior) => (
            WS_EX_TOOLWINDOW
                | if behavior.accepts_keyboard {
                    0
                } else {
                    WS_EX_NOACTIVATE
                },
            WS_CAPTION | WS_SYSMENU | WS_THICKFRAME,
        ),
    };
    let workspace_panes = match spec.content.snapshot().props() {
        Props::Pattern {
            pattern:
                UiPattern::NavigationWorkspace {
                    sidebar_collapsible,
                    inspector_collapsible,
                },
        } => Some((*sidebar_collapsible, *inspector_collapsible)),
        _ => None,
    };
    let has_toolbar = !spec.toolbar.is_empty()
        || workspace_panes.is_some_and(|(sidebar, inspector)| sidebar || inspector);
    let creation_dpi = unsafe { GetDpiForSystem() }.max(96);
    let window_style = style | WS_CLIPCHILDREN;
    let initial_width = scale(spec.initial_size.width.round() as i32, creation_dpi);
    let initial_height = scale(spec.initial_size.height.round() as i32, creation_dpi)
        + if has_toolbar {
            scale(TOOLBAR_HEIGHT, creation_dpi)
        } else {
            0
        };
    let (initial_outer_width, initial_outer_height) = outer_size_for_content(
        initial_width,
        initial_height,
        window_style,
        extended,
        creation_dpi,
    )?;
    // SAFETY: the registered class and UTF-16 strings remain valid through the call.
    let hwnd = unsafe {
        CreateWindowExW(
            extended,
            class_name.as_ptr(),
            title.as_ptr(),
            window_style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            initial_outer_width,
            initial_outer_height,
            owner,
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        return Err(last_error("CreateWindowExW(top-level)"));
    }
    let mut pending_window = PendingTopLevel(hwnd);
    set_dark_title_bar(hwnd, dark);
    let dpi = dpi_for_window(hwnd);
    let font = system_message_font(dpi)?;
    let symbol_font = system_symbol_font(dpi)?;
    let root = create_window(
        STATIC_CLASS,
        "Rinka content root",
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        WS_EX_CONTROLPARENT,
        hwnd,
        null_mut(),
    )?;
    set_native_font(root, font.0);
    apply_native_theme(root, dark);
    let backend = WindowsBackend::new(root, dpi, font.clone(), dark);
    let runtime = WindowRuntime::mount(
        Renderer::new(backend),
        spec.content,
        crate::platform_services(),
    )
    .map_err(|error| WindowsDiagnostic::InvalidNativeState {
        reason: format!("initial Windows render failed: {error}"),
    })?;
    let mut host = Box::new(HostWindow {
        hwnd,
        root,
        runtime,
        commands: HashMap::new(),
        toolbar: Vec::new(),
        tooltip: null_mut(),
        tooltip_texts: Vec::new(),
        next_id: CONTROL_ID_FIRST,
        dpi,
        minimum_width: spec.minimum_size.width.round() as i32,
        minimum_height: spec.minimum_size.height.round() as i32,
        window_style,
        window_extended_style: extended,
        dark,
        font,
        symbol_font,
        panel_behavior,
        // SAFETY: this brush remains owned by HostWindow until WM_NCDESTROY.
        background_brush: if dark {
            unsafe { CreateSolidBrush(DARK_BACKGROUND) }
        } else {
            null_mut()
        },
    });
    build_toolbar(
        &mut host,
        &spec.toolbar,
        spec.toolbar_display,
        workspace_panes,
    )?;
    // SAFETY: `host` remains allocated until WM_NCDESTROY reclaims the pointer.
    unsafe {
        let host_pointer = Box::into_raw(host);
        let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, host_pointer as isize);
        TOP_LEVEL_WINDOWS.with(|windows| windows.borrow_mut().push(hwnd));
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        (*host_pointer).relayout(rect.right - rect.left, rect.bottom - rect.top);
        ShowWindow(
            hwnd,
            if panel_behavior.is_some_and(|behavior| !behavior.accepts_keyboard) {
                SW_SHOWNOACTIVATE
            } else {
                SW_SHOWNORMAL
            },
        );
        let _ = UpdateWindow(hwnd);
    }
    pending_window.0 = null_mut();
    Ok(hwnd)
}

fn build_toolbar(
    host: &mut HostWindow,
    items: &[ToolbarItem],
    display: ToolbarDisplay,
    workspace_panes: Option<(bool, bool)>,
) -> Result<(), WindowsDiagnostic> {
    for item in items {
        let right_aligned = matches!(item.placement, rinka_core::ToolbarPlacement::Trailing);
        match &item.kind {
            ToolbarItemKind::Action {
                symbol,
                on_activate,
            } => {
                let events = EventBindings::activate(on_activate.clone());
                let presentation = toolbar_presentation(display, *symbol, &item.label);
                add_toolbar_button(
                    host,
                    &presentation,
                    &item.label,
                    &item.help,
                    item.enabled,
                    right_aligned,
                    events,
                )?;
            }
            ToolbarItemKind::ActionGroup { actions } => {
                for action in actions {
                    add_action_button(host, action, item.enabled, display, right_aligned)?;
                }
            }
            ToolbarItemKind::SelectionGroup {
                choices,
                selected_id,
                on_select,
            } => {
                for (choice_index, choice) in choices.iter().enumerate() {
                    let id = host.next_id;
                    host.next_id += 1;
                    let presentation = toolbar_presentation(display, choice.symbol, &choice.label);
                    let hwnd = toolbar_control(
                        host.hwnd,
                        BUTTON_CLASS,
                        &presentation.text,
                        WS_CHILD
                            | WS_VISIBLE
                            | WS_TABSTOP
                            | BS_AUTORADIOBUTTON
                            | BS_PUSHLIKE
                            | BS_FLAT
                            | if choice_index == 0 { WS_GROUP } else { 0 },
                        id,
                        if presentation.symbol_only {
                            host.symbol_font.0
                        } else {
                            host.font.0
                        },
                        host.dark,
                    )?;
                    set_accessible_name(hwnd, &choice.label)?;
                    add_toolbar_tooltip(host, hwnd, &choice.label)?;
                    set_enabled(hwnd, item.enabled && choice.enabled);
                    if choice.id == *selected_id {
                        // SAFETY: the button is live and accepts BM_SETCHECK.
                        unsafe {
                            let _ = send_message(hwnd, BM_SETCHECK, BST_CHECKED, 0);
                        }
                    }
                    host.commands.insert(
                        id,
                        Command::Select {
                            value: choice.id.clone(),
                            events: EventBindings::input(on_select.clone()),
                        },
                    );
                    host.toolbar.push(ToolbarControl {
                        hwnd,
                        width: presentation.width,
                        right_aligned,
                        essential: false,
                        symbol_only: presentation.symbol_only,
                    });
                }
            }
            ToolbarItemKind::Menu {
                symbol, entries, ..
            } => {
                let id = host.next_id;
                host.next_id += 1;
                let mut presentation = toolbar_presentation(display, *symbol, &item.label);
                if !presentation.symbol_only {
                    presentation.text.push_str(" ▾");
                    presentation.width = (presentation.width + 14).min(180);
                }
                let hwnd = toolbar_control(
                    host.hwnd,
                    BUTTON_CLASS,
                    &presentation.text,
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
                    id,
                    if presentation.symbol_only {
                        host.symbol_font.0
                    } else {
                        host.font.0
                    },
                    host.dark,
                )?;
                set_accessible_name(hwnd, &item.label)?;
                add_toolbar_tooltip(host, hwnd, &item.help)?;
                set_enabled(hwnd, item.enabled);
                let mut commands = Vec::new();
                append_menu_commands(entries, item.enabled, &mut commands)?;
                host.commands.insert(
                    id,
                    Command::Menu {
                        hwnd,
                        entries: commands,
                    },
                );
                host.toolbar.push(ToolbarControl {
                    hwnd,
                    width: presentation.width,
                    right_aligned,
                    essential: false,
                    symbol_only: presentation.symbol_only,
                });
            }
            ToolbarItemKind::Search {
                value,
                placeholder,
                accessibility_label,
                on_input,
                ..
            } => {
                let id = host.next_id;
                host.next_id += 1;
                let hwnd = toolbar_control(
                    host.hwnd,
                    EDIT_CLASS,
                    value,
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_SEARCH,
                    id,
                    host.font.0,
                    host.dark,
                )?;
                set_cue_banner(hwnd, placeholder);
                set_accessible_name(hwnd, accessibility_label)?;
                add_toolbar_tooltip(host, hwnd, &item.help)?;
                set_enabled(hwnd, item.enabled);
                host.commands.insert(
                    id,
                    Command::Input {
                        hwnd,
                        events: EventBindings::input(on_input.clone()),
                    },
                );
                host.toolbar.push(ToolbarControl {
                    hwnd,
                    width: 190,
                    right_aligned: true,
                    essential: true,
                    symbol_only: false,
                });
            }
        }
    }
    if let Some((sidebar_collapsible, inspector_collapsible)) = workspace_panes {
        if sidebar_collapsible {
            add_pane_toggle(host, "Navigation pane", true, true)?;
        }
        if inspector_collapsible {
            add_pane_toggle(host, "Details pane", false, true)?;
        }
    }
    Ok(())
}

struct ToolbarPresentation {
    text: String,
    width: i32,
    symbol_only: bool,
}

fn toolbar_presentation(
    display: ToolbarDisplay,
    symbol: Symbol,
    label: &str,
) -> ToolbarPresentation {
    let symbol_only = display == ToolbarDisplay::IconOnly;
    ToolbarPresentation {
        text: if symbol_only {
            symbol_glyph(symbol).to_string()
        } else {
            label.to_owned()
        },
        width: if symbol_only {
            40
        } else {
            (label.chars().count() as i32 * 8 + 30).clamp(64, 160)
        },
        symbol_only,
    }
}

fn symbol_glyph(symbol: Symbol) -> char {
    match symbol {
        Symbol::Back => '\u{e72b}',
        Symbol::Forward => '\u{e72a}',
        Symbol::Add => '\u{e710}',
        Symbol::Refresh => '\u{e72c}',
        Symbol::Search => '\u{e721}',
        Symbol::Home => '\u{e80f}',
        Symbol::Folder => '\u{e8b7}',
        Symbol::File => '\u{e8a5}',
        Symbol::Code => '\u{e8a5}',
        Symbol::Image => '\u{e8b9}',
        Symbol::Terminal => '\u{e756}',
        Symbol::Settings => '\u{e713}',
        Symbol::More => '\u{e712}',
        Symbol::Grid => '\u{e8a9}',
        Symbol::List => '\u{e8fd}',
        Symbol::Columns => '\u{e89f}',
        Symbol::Gallery => '\u{e7aa}',
        Symbol::Sort => '\u{e8cb}',
        Symbol::Share => '\u{e72d}',
        Symbol::Tag => '\u{e8ec}',
        Symbol::Disclosure => '\u{e76c}',
        Symbol::Warning => '\u{e7ba}',
    }
}

fn add_action_button(
    host: &mut HostWindow,
    action: &ToolbarAction,
    item_enabled: bool,
    display: ToolbarDisplay,
    right_aligned: bool,
) -> Result<(), WindowsDiagnostic> {
    let presentation = toolbar_presentation(display, action.symbol, &action.label);
    add_toolbar_button(
        host,
        &presentation,
        &action.label,
        &action.help,
        item_enabled && action.enabled,
        right_aligned,
        EventBindings::activate(action.on_activate.clone()),
    )
}

fn add_toolbar_button(
    host: &mut HostWindow,
    presentation: &ToolbarPresentation,
    accessibility_label: &str,
    tooltip: &str,
    enabled: bool,
    right_aligned: bool,
    events: EventBindings,
) -> Result<(), WindowsDiagnostic> {
    let id = host.next_id;
    host.next_id += 1;
    let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON;
    let hwnd = toolbar_control(
        host.hwnd,
        BUTTON_CLASS,
        &presentation.text,
        style,
        id,
        if presentation.symbol_only {
            host.symbol_font.0
        } else {
            host.font.0
        },
        host.dark,
    )?;
    set_accessible_name(hwnd, accessibility_label)?;
    add_toolbar_tooltip(host, hwnd, tooltip)?;
    set_enabled(hwnd, enabled);
    host.commands.insert(id, Command::Activate(events));
    host.toolbar.push(ToolbarControl {
        hwnd,
        width: presentation.width,
        right_aligned,
        essential: false,
        symbol_only: presentation.symbol_only,
    });
    Ok(())
}

fn add_pane_toggle(
    host: &mut HostWindow,
    label: &str,
    sidebar: bool,
    right_aligned: bool,
) -> Result<(), WindowsDiagnostic> {
    let id = host.next_id;
    host.next_id += 1;
    let text = if sidebar { '\u{e8a0}' } else { '\u{e90d}' }.to_string();
    let hwnd = toolbar_control(
        host.hwnd,
        BUTTON_CLASS,
        &text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX | BS_PUSHLIKE,
        id,
        host.symbol_font.0,
        host.dark,
    )?;
    set_accessible_name(hwnd, label)?;
    add_toolbar_tooltip(host, hwnd, label)?;
    // SAFETY: the button accepts BM_SETCHECK and is initialized visible.
    unsafe {
        let _ = send_message(hwnd, BM_SETCHECK, BST_CHECKED, 0);
    }
    host.commands.insert(
        id,
        if sidebar {
            Command::ToggleSidebar { hwnd }
        } else {
            Command::ToggleInspector { hwnd }
        },
    );
    host.toolbar.push(ToolbarControl {
        hwnd,
        width: 40,
        right_aligned,
        essential: true,
        symbol_only: true,
    });
    Ok(())
}

fn add_toolbar_tooltip(
    host: &mut HostWindow,
    control: HWND,
    text: &str,
) -> Result<(), WindowsDiagnostic> {
    if text.trim().is_empty() {
        return Ok(());
    }
    if host.tooltip.is_null() {
        host.tooltip = create_window(
            TOOLTIP_CLASS,
            "",
            WS_POPUP | TTS_ALWAYSTIP,
            WS_EX_TOPMOST,
            host.hwnd,
            null_mut(),
        )?;
        apply_native_theme(host.tooltip, host.dark);
    }
    let mut text = wide(text).into_boxed_slice();
    let mut tool = TTTOOLINFOW {
        // `TTM_ADDTOOLW` consumes the version-2 prefix. Windows Server 2025 rejects the
        // newer allocation size that includes `lpReserved`, so derive the SDK's
        // `TTTOOLINFO_V2_SIZE` boundary from the field layout instead of hard-coding it.
        cbSize: u32::try_from(std::mem::offset_of!(TTTOOLINFOW, lpReserved)).unwrap_or(u32::MAX),
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: host.hwnd,
        uId: control as usize,
        rect: RECT::default(),
        hinst: null_mut(),
        lpszText: text.as_mut_ptr(),
        lParam: 0,
        lpReserved: null_mut(),
    };
    // SAFETY: the tooltip and control are live on the UI thread; the retained text allocation
    // remains stable until the host destroys the tooltip.
    let added = unsafe { send_message(host.tooltip, TTM_ADDTOOLW, 0, (&raw mut tool) as isize) };
    if added == 0 {
        return Err(WindowsDiagnostic::InvalidNativeState {
            reason: "native toolbar tooltip registration failed".to_owned(),
        });
    }
    host.tooltip_texts.push(text);
    Ok(())
}

/// Collects the shared menu vocabulary into flat popup commands.
///
/// `ancestors_enabled` folds the enabled state of the owning toolbar item and
/// every enclosing submenu into each command. The classic popup probe has no
/// nested-menu realization yet, so a submenu entry is a typed diagnostic
/// rather than a silent drop; a destructive role keeps the standard popup
/// appearance, as recorded in reports/context-menus.
fn append_menu_commands(
    entries: &[MenuEntry],
    ancestors_enabled: bool,
    commands: &mut Vec<MenuCommand>,
) -> Result<(), WindowsDiagnostic> {
    for entry in entries {
        match entry {
            MenuEntry::Item(item) => commands.push(MenuCommand::Action {
                label: item.label.clone(),
                enabled: ancestors_enabled && item.enabled,
                checked: item.checked,
                events: EventBindings::activate(item.on_activate.clone()),
            }),
            MenuEntry::Separator => commands.push(MenuCommand::Separator),
            MenuEntry::Submenu(_) => {
                return Err(WindowsDiagnostic::UnsupportedToolbarCapability {
                    capability: "submenu entries in a toolbar menu",
                });
            }
        }
    }
    Ok(())
}

fn show_command_menu(owner: HWND, button: HWND, entries: &[MenuCommand]) {
    // SAFETY: the menu exists only for this synchronous popup interaction.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        for (index, entry) in entries.iter().enumerate() {
            match entry {
                MenuCommand::Action {
                    label,
                    enabled,
                    checked,
                    ..
                } => {
                    let label = wide(label);
                    let flags = MF_STRING
                        | if *enabled { 0 } else { MF_GRAYED }
                        | if *checked { MF_CHECKED } else { 0 };
                    let _ = AppendMenuW(menu, flags, index + 1, label.as_ptr());
                }
                MenuCommand::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, null());
                }
            }
        }
        let mut rect = RECT::default();
        let _ = GetWindowRect(button, &mut rect);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            rect.left,
            rect.bottom,
            0,
            owner,
            null(),
        );
        let _ = DestroyMenu(menu);
        if selected > 0
            && let Some(MenuCommand::Action {
                enabled: true,
                events,
                ..
            }) = entries.get(selected as usize - 1)
        {
            events.emit_activate();
        }
    }
}

fn toolbar_control(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    id: usize,
    font: HFONT,
    dark: bool,
) -> Result<HWND, WindowsDiagnostic> {
    let hwnd = create_window(class, text, style, 0, parent, id as HMENU)?;
    set_native_font(hwnd, font);
    apply_native_theme(hwnd, dark);
    Ok(hwnd)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            // SAFETY: DefWindowProc handles initial creation until HostWindow is installed.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_CREATE => 0,
        WM_SIZE => {
            if let Some(host) = host_window(hwnd) {
                let width = low_word(lparam as usize) as i32;
                let height = high_word(lparam as usize) as i32;
                host.relayout(width, height);
            }
            0
        }
        WM_ACTIVATEAPP => {
            set_inactive_panels_visible(wparam != 0);
            // SAFETY: activation bookkeeping is complete; the default procedure retains
            // standard top-level activation behavior.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_DPICHANGED => {
            if let Some(host) = host_window(hwnd) {
                host.set_dpi(high_word(wparam) as u32);
                let suggested = lparam as *const RECT;
                if !suggested.is_null() {
                    // SAFETY: WM_DPICHANGED supplies a valid RECT for this synchronous call.
                    let rect = unsafe { &*suggested };
                    move_window(
                        hwnd,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        true,
                    );
                }
            }
            0
        }
        WM_GETMINMAXINFO => {
            if let Some(host) = host_window(hwnd) {
                let info = lparam as *mut MINMAXINFO;
                if !info.is_null() {
                    let content_width = scale(host.minimum_width, host.dpi);
                    let content_height =
                        scale(host.minimum_height, host.dpi) + host.toolbar_height();
                    if let Ok((outer_width, outer_height)) = outer_size_for_content(
                        content_width,
                        content_height,
                        host.window_style,
                        host.window_extended_style,
                        host.dpi,
                    ) {
                        // SAFETY: WM_GETMINMAXINFO supplies writable MINMAXINFO storage.
                        unsafe {
                            (*info).ptMinTrackSize.x = outer_width;
                            (*info).ptMinTrackSize.y = outer_height;
                        }
                    }
                }
            }
            0
        }
        WM_COMMAND => {
            if let Some(host) = host_window(hwnd) {
                host.command(low_word(wparam) as usize, high_word(wparam));
            }
            0
        }
        WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            if let Some(host) = host_window(hwnd)
                && host.dark
            {
                return configure_dark_device_context(wparam as HDC, host.background_brush);
            }
            // SAFETY: unhandled color messages use the registered default procedure.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_ERASE_BACKGROUND => {
            if let Some(host) = host_window(hwnd)
                && host.dark
            {
                paint_dark_background(hwnd, wparam as HDC, host.background_brush);
                return 1;
            }
            // SAFETY: unhandled erase messages use the registered default procedure.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_DESTROY => {
            let is_managed_window = host_window(hwnd).is_some();
            let last_window = is_managed_window
                && TOP_LEVEL_WINDOWS.with(|windows| {
                    let mut windows = windows.borrow_mut();
                    windows.retain(|candidate| *candidate != hwnd);
                    windows.is_empty()
                });
            if last_window && MESSAGE_LOOP_ACTIVE.with(Cell::get) {
                // SAFETY: the application terminates only after its final managed window closes.
                unsafe { PostQuitMessage(0) };
            }
            0
        }
        WM_NCDESTROY => {
            // SAFETY: the pointer was allocated by Box::into_raw exactly once.
            unsafe {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HostWindow;
                if !pointer.is_null() {
                    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        _ => {
            // SAFETY: unhandled messages are delegated to the registered window procedure.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}
