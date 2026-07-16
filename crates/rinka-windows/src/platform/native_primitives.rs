fn structural(handle: &WindowsHandle) -> bool {
    matches!(
        handle.0.kind,
        HostKind::Root
            | HostKind::Element(
                ElementKind::Stack
                    | ElementKind::Scroll
                    | ElementKind::Pattern
                    | ElementKind::List
                    | ElementKind::Status
            )
    )
}

fn is_flexible(handle: &WindowsHandle, axis: Axis) -> bool {
    match handle.0.props.borrow().as_ref() {
        Some(Props::Spacer {
            horizontal,
            vertical,
        }) => match axis {
            Axis::Horizontal => *horizontal,
            Axis::Vertical => *vertical,
        },
        Some(Props::List { .. } | Props::Scroll { .. } | Props::Pattern { .. }) => true,
        Some(Props::Stack { .. }) => handle
            .0
            .children
            .borrow()
            .iter()
            .any(|child| is_flexible(child, axis)),
        _ => false,
    }
}

fn find_ancestor_list(root: &WindowsHandle, owner: HWND) -> Option<WindowsHandle> {
    if root.0.kind == HostKind::Element(ElementKind::List) && root.0.hwnd == owner {
        return Some(root.clone());
    }
    for child in root.0.children.borrow().iter() {
        if let Some(value) = find_ancestor_list(child, owner) {
            return Some(value);
        }
    }
    None
}

fn create_window(
    class: &str,
    text: &str,
    style: u32,
    extended: u32,
    parent: HWND,
    menu: HMENU,
) -> Result<HWND, WindowsDiagnostic> {
    let class = wide(class);
    let text = wide(text);
    let instance = module_instance()?;
    // SAFETY: the class and title UTF-16 buffers remain alive for the synchronous call.
    let hwnd = unsafe {
        CreateWindowExW(
            extended,
            class.as_ptr(),
            text.as_ptr(),
            style,
            0,
            0,
            1,
            1,
            parent,
            menu,
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        Err(last_error("CreateWindowExW(control)"))
    } else {
        Ok(hwnd)
    }
}

fn module_instance() -> Result<HINSTANCE, WindowsDiagnostic> {
    // SAFETY: a null module name requests the current executable module.
    let instance = unsafe { GetModuleHandleW(null()) };
    if instance.is_null() {
        Err(last_error("GetModuleHandleW"))
    } else {
        Ok(instance)
    }
}

fn last_error(operation: &'static str) -> WindowsDiagnostic {
    // SAFETY: GetLastError reads thread-local operating-system state.
    WindowsDiagnostic::NativeOperation {
        operation,
        code: unsafe { GetLastError() },
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn class_name(hwnd: HWND) -> String {
    let mut storage = [0u16; 128];
    // SAFETY: storage is writable and hwnd remains owned by the handle.
    let length = unsafe { GetClassNameW(hwnd, storage.as_mut_ptr(), storage.len() as i32) };
    String::from_utf16_lossy(&storage[..usize::try_from(length).unwrap_or_default()])
}

fn system_message_font(dpi: u32) -> Result<Rc<NativeFont>, WindowsDiagnostic> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: u32::try_from(size_of::<NONCLIENTMETRICSW>()).unwrap_or(u32::MAX),
        ..Default::default()
    };
    // SAFETY: metrics is writable for the duration of the synchronous system query.
    unsafe {
        if SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast::<c_void>(),
            0,
            dpi.max(96),
        ) == 0
        {
            return Err(last_error("SystemParametersInfoForDpi(message font)"));
        }
        let font = CreateFontIndirectW(&raw const metrics.lfMessageFont);
        if font.is_null() {
            return Err(last_error("CreateFontIndirectW(message font)"));
        }
        Ok(Rc::new(NativeFont(font)))
    }
}

fn system_symbol_font(dpi: u32) -> Result<Rc<NativeFont>, WindowsDiagnostic> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: u32::try_from(size_of::<NONCLIENTMETRICSW>()).unwrap_or(u32::MAX),
        ..Default::default()
    };
    // SAFETY: metrics is writable for the duration of the synchronous system query.
    unsafe {
        if SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast::<c_void>(),
            0,
            dpi.max(96),
        ) == 0
        {
            return Err(last_error("SystemParametersInfoForDpi(symbol font)"));
        }
        metrics.lfMessageFont.lfHeight = -scale(16, dpi.max(96));
        metrics.lfMessageFont.lfWeight = 400;
        let family = wide("Segoe Fluent Icons");
        let copy_length = family.len().min(metrics.lfMessageFont.lfFaceName.len());
        metrics.lfMessageFont.lfFaceName[..copy_length].copy_from_slice(&family[..copy_length]);
        let font = CreateFontIndirectW(&raw const metrics.lfMessageFont);
        if font.is_null() {
            return Err(last_error("CreateFontIndirectW(symbol font)"));
        }
        Ok(Rc::new(NativeFont(font)))
    }
}

fn apply_semantic_font_tree(
    handle: &WindowsHandle,
    dpi: u32,
    base_font: HFONT,
) -> Result<(), WindowsDiagnostic> {
    apply_semantic_font(handle, dpi, base_font)?;
    for child in handle.0.children.borrow().iter() {
        apply_semantic_font_tree(child, dpi, base_font)?;
    }
    Ok(())
}

fn apply_semantic_font(
    handle: &WindowsHandle,
    dpi: u32,
    base_font: HFONT,
) -> Result<(), WindowsDiagnostic> {
    let role = match handle.0.props.borrow().as_ref() {
        Some(Props::Label { role, .. }) => Some(*role),
        _ => None,
    };
    let font = match role {
        Some(role @ (TextRole::Title | TextRole::Heading | TextRole::Monospace)) => {
            Some(system_text_role_font(dpi, role)?)
        }
        Some(TextRole::Body | TextRole::Secondary) | None => None,
    };
    set_native_font(
        handle.0.hwnd,
        font.as_ref().map_or(base_font, |font| font.0),
    );
    *handle.0.semantic_font.borrow_mut() = font;
    Ok(())
}

fn clear_semantic_fonts(handle: &WindowsHandle, base_font: HFONT) {
    set_native_font(handle.0.hwnd, base_font);
    *handle.0.semantic_font.borrow_mut() = None;
    for child in handle.0.children.borrow().iter() {
        clear_semantic_fonts(child, base_font);
    }
}

fn system_text_role_font(dpi: u32, role: TextRole) -> Result<Rc<NativeFont>, WindowsDiagnostic> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: u32::try_from(size_of::<NONCLIENTMETRICSW>()).unwrap_or(u32::MAX),
        ..Default::default()
    };
    // SAFETY: metrics is writable for the duration of the synchronous system query.
    unsafe {
        if SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast::<c_void>(),
            0,
            dpi.max(96),
        ) == 0
        {
            return Err(last_error("SystemParametersInfoForDpi(text role font)"));
        }
        match role {
            TextRole::Title => {
                metrics.lfMessageFont.lfHeight = -scale(22, dpi.max(96));
                metrics.lfMessageFont.lfWeight = 600;
                set_logfont_family(
                    &mut metrics.lfMessageFont.lfFaceName,
                    "Segoe UI Variable Display",
                );
            }
            TextRole::Heading => {
                metrics.lfMessageFont.lfHeight = -scale(16, dpi.max(96));
                metrics.lfMessageFont.lfWeight = 600;
                set_logfont_family(
                    &mut metrics.lfMessageFont.lfFaceName,
                    "Segoe UI Variable Display",
                );
            }
            TextRole::Monospace => {
                set_logfont_family(&mut metrics.lfMessageFont.lfFaceName, "Cascadia Mono");
            }
            TextRole::Body | TextRole::Secondary => {}
        }
        let font = CreateFontIndirectW(&raw const metrics.lfMessageFont);
        if font.is_null() {
            return Err(last_error("CreateFontIndirectW(text role font)"));
        }
        Ok(Rc::new(NativeFont(font)))
    }
}

fn set_logfont_family(target: &mut [u16], family: &str) {
    target.fill(0);
    let family = wide(family);
    let copy_length = family.len().min(target.len());
    target[..copy_length].copy_from_slice(&family[..copy_length]);
}

fn set_native_font(hwnd: HWND, font: HFONT) {
    // SAFETY: the retained NativeFont outlives every HWND receiving this synchronous message.
    unsafe {
        let _ = send_message(hwnd, WM_SETFONT, font as usize, 1);
    }
}

fn outer_size_for_content(
    width: i32,
    height: i32,
    style: u32,
    extended_style: u32,
    dpi: u32,
) -> Result<(i32, i32), WindowsDiagnostic> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width.max(1),
        bottom: height.max(1),
    };
    // SAFETY: rect is writable for the synchronous content-to-window conversion.
    unsafe {
        if AdjustWindowRectExForDpi(&mut rect, style, 0, extended_style, dpi.max(96)) == 0 {
            return Err(last_error("AdjustWindowRectExForDpi"));
        }
    }
    Ok((rect.right - rect.left, rect.bottom - rect.top))
}

fn set_window_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    // SAFETY: UTF-16 storage remains alive for the synchronous SetWindowTextW call.
    unsafe {
        let _ = SetWindowTextW(hwnd, text.as_ptr());
    }
}

fn window_text(hwnd: HWND) -> String {
    // SAFETY: the queried HWND is live and the allocated buffer includes a terminator slot.
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        let mut storage = vec![0u16; usize::try_from(length).unwrap_or_default() + 1];
        let copied = GetWindowTextW(
            hwnd,
            storage.as_mut_ptr(),
            i32::try_from(storage.len()).unwrap_or(i32::MAX),
        );
        String::from_utf16_lossy(&storage[..usize::try_from(copied).unwrap_or_default()])
    }
}

fn set_cue_banner(hwnd: HWND, text: &str) {
    let text = wide(text);
    // SAFETY: EM_SETCUEBANNER synchronously copies text into the native edit control.
    unsafe {
        let _ = send_message(hwnd, EM_SETCUEBANNER, 1, text.as_ptr() as isize);
    }
}

fn set_progress(hwnd: HWND, fraction: f64) {
    let position = (fraction.clamp(0.0, 1.0) * 100.0).round() as usize;
    // SAFETY: the progress control accepts PBM_SETPOS with an integer percentage.
    unsafe {
        let _ = send_message(hwnd, PBM_SETPOS, position, 0);
    }
}

fn set_button_checked(hwnd: HWND, checked: bool) {
    // SAFETY: the button control accepts BM_SETCHECK.
    unsafe {
        let _ = send_message(hwnd, BM_SETCHECK, usize::from(checked), 0);
    }
}

fn set_enabled(hwnd: HWND, enabled: bool) {
    // SAFETY: the HWND is live and owned by the current UI thread.
    unsafe {
        let _ = EnableWindow(hwnd, i32::from(enabled));
    }
}

fn button_checked(hwnd: HWND) -> bool {
    // SAFETY: the button control accepts BM_GETCHECK.
    unsafe { send_message(hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED as isize }
}

fn apply_native_theme(hwnd: HWND, dark: bool) {
    let theme = wide(if dark {
        "DarkMode_Explorer"
    } else {
        "Explorer"
    });
    // SAFETY: the theme strings remain alive during this synchronous call.
    unsafe {
        let _ = SetWindowTheme(hwnd, theme.as_ptr(), null());
    }
}

fn dark_appearance() -> bool {
    match std::env::var("RINKA_WINDOWS_APPEARANCE") {
        Ok(value) if value.eq_ignore_ascii_case("dark") => true,
        Ok(value) if value.eq_ignore_ascii_case("light") => false,
        Ok(_) | Err(_) => system_prefers_dark_apps(),
    }
}

fn system_prefers_dark_apps() -> bool {
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value = 1u32;
    let mut value_size = u32::try_from(size_of::<u32>()).unwrap_or(u32::MAX);
    // SAFETY: RegGetValueW synchronously writes one DWORD into `value`; both UTF-16 strings
    // and the byte-count pointer remain valid for the call.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&raw mut value).cast::<c_void>(),
            &mut value_size,
        )
    };
    result == ERROR_SUCCESS && value == 0
}

fn configure_dark_device_context(device_context: HDC, brush: HBRUSH) -> LRESULT {
    // SAFETY: the device context belongs to the synchronous color message and the brush lives
    // through the current host or retained handle.
    unsafe {
        let _ = SetBkColor(device_context, DARK_BACKGROUND);
        let _ = SetTextColor(device_context, DARK_TEXT);
    }
    brush as LRESULT
}

fn paint_dark_background(hwnd: HWND, device_context: HDC, brush: HBRUSH) {
    let mut rect = RECT::default();
    // SAFETY: the device context belongs to WM_ERASEBKGND and all values are live for the call.
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
        let _ = FillRect(device_context, &rect, brush);
    }
}

fn set_dark_title_bar(hwnd: HWND, dark: bool) {
    let value = i32::from(dark);
    // SAFETY: DwmSetWindowAttribute synchronously reads the supplied i32.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const value).cast::<c_void>(),
            u32::try_from(size_of::<i32>()).unwrap_or(u32::MAX),
        );
    }
}

fn dpi_for_window(hwnd: HWND) -> u32 {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDpiForWindow(hwnd: HWND) -> u32;
    }
    // SAFETY: the HWND is live and owned by the current UI thread.
    unsafe { GetDpiForWindow(hwnd).max(96) }
}

fn scale(value: i32, dpi: u32) -> i32 {
    value.saturating_mul(dpi as i32) / 96
}

fn show(hwnd: HWND, visible: bool) {
    // SAFETY: the HWND remains live while it is laid out.
    unsafe {
        ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }
}

fn move_window(hwnd: HWND, x: i32, y: i32, width: i32, height: i32, repaint: bool) {
    // SAFETY: the HWND remains live while its owning host performs layout.
    unsafe {
        let _ = MoveWindow(hwnd, x, y, width.max(0), height.max(0), i32::from(repaint));
    }
}

fn low_word(value: usize) -> u16 {
    (value & 0xffff) as u16
}

fn high_word(value: usize) -> u16 {
    ((value >> 16) & 0xffff) as u16
}

unsafe fn send_message(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendMessageW(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    }
    // SAFETY: the caller establishes the message-specific pointer and lifetime invariants.
    unsafe { SendMessageW(hwnd, message, wparam, lparam) }
}
