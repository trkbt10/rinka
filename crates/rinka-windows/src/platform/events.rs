fn set_inactive_panels_visible(application_active: bool) {
    let windows = TOP_LEVEL_WINDOWS.with(|windows| windows.borrow().clone());
    for hwnd in windows {
        let Some(host) = host_window(hwnd) else {
            continue;
        };
        let Some(behavior) = host.panel_behavior else {
            continue;
        };
        if !behavior.hides_when_inactive {
            continue;
        }
        let command = if application_active {
            if behavior.accepts_keyboard {
                SW_SHOW
            } else {
                SW_SHOWNOACTIVATE
            }
        } else {
            SW_HIDE
        };
        // SAFETY: the registry contains live top-level HWND values on the current UI thread.
        unsafe {
            ShowWindow(hwnd, command);
        }
    }
}

fn host_window(hwnd: HWND) -> Option<&'static mut HostWindow> {
    // SAFETY: the pointer is owned by the window between installation and WM_NCDESTROY.
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HostWindow;
        pointer.as_mut()
    }
}

unsafe extern "system" fn element_subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    let inner = reference_data as *const HandleInner;
    if inner.is_null() {
        // SAFETY: Windows owns the default subclass procedure.
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    }
    // For controls whose state changes in the default procedure, run the default
    // handler first and then emit through the stable Rust event slot.
    // SAFETY: the native subclass API permits calling DefSubclassProc once per message.
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    // SAFETY: reference_data points at the live Rc allocation until subclass removal.
    let handle = unsafe { &*inner };
    if handle.dark {
        match message {
            WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
                return configure_dark_device_context(wparam as HDC, handle.background_brush);
            }
            WM_ERASE_BACKGROUND => {
                paint_dark_background(hwnd, wparam as HDC, handle.background_brush);
                return 1;
            }
            _ => {}
        }
    }
    if message == WM_NOTIFY_MESSAGE {
        emit_list_notification(handle, lparam);
    }
    if message == WM_COMMAND {
        emit_control_command(handle, wparam, lparam);
    }
    match (handle.kind, message) {
        (HostKind::Element(ElementKind::ListRow), WM_LBUTTONUP) => {
            handle.events.emit_activate();
        }
        (HostKind::Element(ElementKind::ListRow), WM_KEYUP)
            if wparam == VK_SPACE as usize || wparam == VK_RETURN as usize =>
        {
            handle.events.emit_activate();
        }
        (HostKind::Element(ElementKind::List), WM_KEYUP) if wparam == VK_RETURN as usize => {
            activate_selected_list_row(handle);
        }
        (HostKind::Element(ElementKind::List), WM_LBUTTONDBLCLK) => {
            toggle_selected_list_row(handle);
        }
        (HostKind::Element(ElementKind::List), WM_KEYUP) if wparam == VK_SPACE as usize => {
            toggle_selected_list_row(handle);
        }
        _ => {}
    }
    result
}

fn emit_control_command(parent: &HandleInner, wparam: WPARAM, lparam: LPARAM) {
    let source = lparam as HWND;
    if source.is_null() {
        return;
    }
    let child = parent
        .children
        .borrow()
        .iter()
        .find(|child| child.0.hwnd == source)
        .cloned();
    let Some(child) = child else {
        return;
    };
    let notification = high_word(wparam);
    match child.0.kind {
        HostKind::Element(ElementKind::Button) if notification == BN_CLICKED => {
            child.0.events.emit_activate();
        }
        HostKind::Element(ElementKind::Toggle) if notification == BN_CLICKED => {
            child.0.events.emit_toggle(button_checked(source));
        }
        HostKind::Element(ElementKind::Input) if notification == EN_CHANGE => {
            child.0.events.emit_input(window_text(source));
        }
        _ => {}
    }
}

fn emit_list_notification(parent: &HandleInner, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    // SAFETY: every WM_NOTIFY structure begins with NMHDR for this synchronous call.
    let header = unsafe { &*(lparam as *const NmHdr) };
    let list = parent
        .children
        .borrow()
        .iter()
        .find(|child| child.0.hwnd == header.hwnd_from)
        .cloned();
    let Some(list) = list else {
        return;
    };
    if list.0.list_rebuilding.get() {
        return;
    }
    if header.code as i32 == TVN_SELCHANGEDW {
        // SAFETY: the notification code identifies NMTREEVIEWW storage.
        let notification = unsafe { &*(lparam as *const NmTreeViewW) };
        let row = notification.item_new.l_param as *const HandleInner;
        // SAFETY: inserted tree items retain their declarative row allocation.
        if let Some(row) = unsafe { row.as_ref() } {
            row.events.emit_activate();
        }
        return;
    }
    if header.code as i32 == TVN_ITEMEXPANDEDW {
        // SAFETY: the notification code identifies NMTREEVIEWW storage.
        let notification = unsafe { &*(lparam as *const NmTreeViewW) };
        let row = notification.item_new.l_param as *const HandleInner;
        // SAFETY: every inserted tree item retains an Rc pointer until it is deleted.
        if let Some(row) = unsafe { row.as_ref() } {
            row.events
                .emit_toggle(notification.action == TVE_EXPAND as u32);
        }
        return;
    }
    if header.code as i32 == LVN_ITEMCHANGED {
        // SAFETY: the notification code identifies NMLISTVIEW storage.
        let notification = unsafe { &*(lparam as *const NmListView) };
        let became_selected = notification.changed & LVIF_STATE != 0
            && notification.new_state & LVIS_SELECTED != 0
            && notification.old_state & LVIS_SELECTED == 0;
        if became_selected {
            let row = notification.l_param as *const HandleInner;
            // SAFETY: inserted list items retain their declarative row allocation.
            if let Some(row) = unsafe { row.as_ref() } {
                row.events.emit_activate();
            }
        }
        return;
    }
    if header.code as i32 != LVN_COLUMNCLICK {
        return;
    }
    // SAFETY: the notification code identifies NMLISTVIEW storage.
    let notification = unsafe { &*(lparam as *const NmListView) };
    let Some(Props::List {
        pattern: CollectionPattern::DataTable,
        columns,
        ..
    }) = list.0.props.borrow().clone()
    else {
        return;
    };
    let Ok(index) = usize::try_from(notification.sub_item) else {
        return;
    };
    let Some(column) = columns.get(index) else {
        return;
    };
    if !column.sortable {
        return;
    }
    let direction = match column.sort_direction {
        Some(SortDirection::Ascending) => SortDirection::Descending,
        Some(SortDirection::Descending) | None => SortDirection::Ascending,
    };
    list.0.events.emit_sort(TableSort {
        column_id: column.id.clone(),
        direction,
    });
}

fn activate_selected_list_row(handle: &HandleInner) {
    if let Some(row) = selected_list_row(handle) {
        row.events.emit_activate();
    }
}

fn toggle_selected_list_row(handle: &HandleInner) {
    if matches!(handle.props.borrow().as_ref(), Some(Props::List { pattern, .. }) if pattern.supports_hierarchy()) {
        return;
    }
    let Some(row) = selected_list_row(handle) else {
        return;
    };
    let Some(Props::ListRow { expanded, .. }) = row.props.borrow().clone() else {
        return;
    };
    if !row.children.borrow().is_empty() {
        row.events.emit_toggle(!expanded);
    }
}

fn selected_list_row(handle: &HandleInner) -> Option<&HandleInner> {
    let Some(Props::List { pattern, .. }) = handle.props.borrow().clone() else {
        return None;
    };
    // SAFETY: the query messages synchronously populate initialized item structures.
    unsafe {
        let row_pointer = if pattern.supports_hierarchy() {
            let selected = send_message(handle.hwnd, TVM_GETNEXTITEM, TVGN_CARET, 0);
            if selected == 0 {
                return None;
            }
            let mut item = TvItemExW {
                mask: TVIF_PARAM,
                item: selected,
                state: 0,
                state_mask: 0,
                psz_text: null_mut(),
                cch_text_max: 0,
                i_image: 0,
                i_selected_image: 0,
                c_children: 0,
                l_param: 0,
                i_integral: 0,
                state_ex: 0,
                hwnd: null_mut(),
                i_expanded_image: 0,
                i_reserved: 0,
            };
            if send_message(handle.hwnd, TVM_GETITEMW, 0, (&raw mut item) as isize) == 0 {
                return None;
            }
            item.l_param as *const HandleInner
        } else {
            let index = send_message(handle.hwnd, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED);
            if index < 0 {
                return None;
            }
            let mut item = LvItemW {
                mask: LVIF_PARAM,
                i_item: index as i32,
                i_sub_item: 0,
                state: 0,
                state_mask: 0,
                psz_text: null_mut(),
                cch_text_max: 0,
                i_image: 0,
                l_param: 0,
                i_indent: 0,
                i_group_id: 0,
                c_columns: 0,
                pu_columns: null_mut(),
                pi_col_fmt: null_mut(),
                i_group: 0,
            };
            if send_message(handle.hwnd, LVM_GETITEMW, 0, (&raw mut item) as isize) == 0 {
                return None;
            }
            item.l_param as *const HandleInner
        };
        row_pointer.as_ref()
    }
}

fn native_description(element: &Element) -> (&'static str, String, u32, u32) {
    match element.props() {
        Props::Label {
            text,
            selectable: true,
            ..
        } => (
            EDIT_CLASS,
            text.clone(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL | ES_READONLY,
            0,
        ),
        Props::Label { text, .. } => (
            STATIC_CLASS,
            text.clone(),
            WS_CHILD | WS_VISIBLE | SS_NOTIFY | SS_LEFT,
            0,
        ),
        Props::Button { label, role, .. } => (
            BUTTON_CLASS,
            label.clone(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | if *role == ButtonRole::Primary {
                    BS_DEFPUSHBUTTON
                } else {
                    BS_PUSHBUTTON
                },
            0,
        ),
        Props::Input { value, kind, .. } => (
            EDIT_CLASS,
            value.clone(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | ES_AUTOHSCROLL
                | if *kind == InputKind::Secure {
                    ES_PASSWORD
                } else {
                    0
                },
            WS_EX_CLIENTEDGE,
        ),
        Props::Toggle { label, .. } => (
            BUTTON_CLASS,
            label.clone(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX,
            0,
        ),
        Props::Progress { .. } => (
            PROGRESS_CLASS,
            String::new(),
            WS_CHILD | WS_VISIBLE | PBS_SMOOTH,
            0,
        ),
        Props::Separator { axis } => (
            STATIC_CLASS,
            String::new(),
            WS_CHILD
                | WS_VISIBLE
                | if *axis == Axis::Horizontal {
                    SS_ETCHEDHORZ
                } else {
                    SS_ETCHEDVERT
                },
            0,
        ),
        Props::Spacer { .. }
        | Props::Stack { .. }
        | Props::Scroll { .. }
        | Props::Pattern { .. } => (
            STATIC_CLASS,
            String::new(),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            WS_EX_CONTROLPARENT,
        ),
        Props::List { pattern, .. } if pattern.supports_hierarchy() =>
        {
            (
                TREE_VIEW_CLASS,
                String::new(),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | WS_VSCROLL
                    | TVS_HASBUTTONS
                    | TVS_HASLINES
                    | TVS_LINESATROOT
                    | TVS_SHOWSELALWAYS
                    | TVS_FULLROWSELECT,
                WS_EX_CLIENTEDGE,
            )
        }
        Props::List { pattern, .. } => (
            LIST_VIEW_CLASS,
            String::new(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | LVS_SINGLESEL
                | LVS_SHOWSELALWAYS
                | if pattern.presents_columns() {
                    LVS_REPORT
                } else {
                    LVS_LIST | LVS_NOSORTHEADER
                },
            WS_EX_CLIENTEDGE,
        ),
        Props::ListRow {
            accessibility_label,
            ..
        } => (
            STATIC_CLASS,
            accessibility_label.clone(),
            WS_CHILD | SS_NOTIFY,
            0,
        ),
        Props::Status {
            title,
            message,
            tone,
        } => (
            STATIC_CLASS,
            format!("{}\r\n{}", status_prefix(*tone, title), message),
            WS_CHILD | WS_VISIBLE | SS_CENTER | SS_NOTIFY,
            WS_EX_CONTROLPARENT,
        ),
    }
}

fn status_prefix(tone: StatusTone, title: &str) -> String {
    match tone {
        StatusTone::Error => format!("⚠ {title}"),
        StatusTone::Busy => format!("… {title}"),
        StatusTone::Empty | StatusTone::Informational => title.to_owned(),
    }
}

fn apply_initial_properties(handle: &WindowsHandle) {
    let Some(props) = handle.0.props.borrow().clone() else {
        return;
    };
    match &props {
        Props::Input { placeholder, .. } => set_cue_banner(handle.0.hwnd, placeholder),
        Props::Toggle { value, .. } => set_button_checked(handle.0.hwnd, *value),
        Props::Progress { fraction, .. } => set_progress(handle.0.hwnd, *fraction),
        _ => {}
    }
    match props {
        Props::Button { enabled, .. }
        | Props::Input { enabled, .. }
        | Props::Toggle { enabled, .. } => set_enabled(handle.0.hwnd, enabled),
        _ => {}
    }
}

fn set_accessible_name(hwnd: HWND, name: &str) -> Result<(), WindowsDiagnostic> {
    let name = wide(name);
    ACCESSIBILITY_SERVICE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            // SAFETY: COM is initialized on the UI thread before native elements are created.
            let service =
                unsafe { CoCreateInstance(&CAccPropServices, None, CLSCTX_INPROC_SERVER) }
                    .map_err(|error| WindowsDiagnostic::InvalidNativeState {
                        reason: format!(
                            "accessibility annotation service creation failed: {error}"
                        ),
                    })?;
            *slot = Some(service);
        }
        let service = slot
            .as_ref()
            .ok_or_else(|| WindowsDiagnostic::InvalidNativeState {
                reason: "accessibility annotation service was not retained".to_owned(),
            })?;
        // SAFETY: the retained service synchronously copies the string and remains alive until
        // every HWND has been destroyed and the UI thread leaves its COM apartment.
        unsafe {
            service.SetHwndPropStr(
                WindowsHwnd(hwnd),
                OBJID_CLIENT.0 as u32,
                CHILDID_SELF,
                Name_Property_GUID,
                PCWSTR(name.as_ptr()),
            )
        }
        .map_err(|error| WindowsDiagnostic::InvalidNativeState {
            reason: format!("UI Automation name annotation failed: {error}"),
        })?;
        // SAFETY: the same retained service also owns the Active Accessibility name used by
        // native Win32 assistive clients and UIA's legacy bridge.
        unsafe {
            service.SetHwndPropStr(
                WindowsHwnd(hwnd),
                OBJID_CLIENT.0 as u32,
                CHILDID_SELF,
                PROPID_ACC_NAME,
                PCWSTR(name.as_ptr()),
            )
        }
        .map_err(|error| WindowsDiagnostic::InvalidNativeState {
            reason: format!("Active Accessibility name annotation failed: {error}"),
        })?;
        Ok(())
    })
}

fn apply_patch_to_native(handle: &WindowsHandle, patch: &PropertyPatch) {
    match patch.props() {
        Props::Label { text, .. } => set_window_text(handle.0.hwnd, text),
        Props::Button { label, enabled, .. } => {
            set_window_text(handle.0.hwnd, label);
            set_enabled(handle.0.hwnd, *enabled);
        }
        Props::Input {
            value,
            placeholder,
            enabled,
            ..
        } => {
            if window_text(handle.0.hwnd) != *value {
                set_window_text(handle.0.hwnd, value);
            }
            set_cue_banner(handle.0.hwnd, placeholder);
            set_enabled(handle.0.hwnd, *enabled);
        }
        Props::Toggle {
            label,
            value,
            enabled,
            ..
        } => {
            set_window_text(handle.0.hwnd, label);
            set_button_checked(handle.0.hwnd, *value);
            set_enabled(handle.0.hwnd, *enabled);
        }
        Props::Progress { fraction, .. } => set_progress(handle.0.hwnd, *fraction),
        Props::Status {
            title,
            message,
            tone,
        } => set_window_text(
            handle.0.hwnd,
            &format!("{}\r\n{}", status_prefix(*tone, title), message),
        ),
        _ => {}
    }
}
