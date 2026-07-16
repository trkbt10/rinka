impl NativeBackend for WindowsBackend {
    type Handle = WindowsHandle;
    type Error = WindowsDiagnostic;

    fn root(&self) -> Self::Handle {
        self.root.clone()
    }

    fn validate(&self, element: &Element) -> Result<(), Self::Error> {
        validate_element(element)
    }

    fn create(
        &mut self,
        element: &Element,
        events: EventBindings,
    ) -> Result<Self::Handle, Self::Error> {
        let (class, text, style, extended) = native_description(element);
        let hwnd = create_window(class, &text, style, extended, self.root.0.hwnd, null_mut())?;
        set_native_font(hwnd, self.font.0);
        apply_native_theme(hwnd, self.dark);
        let handle = WindowsHandle::new(
            hwnd,
            HostKind::Element(element.kind()),
            Some(element.props().clone()),
            events,
            false,
            self.dark,
        );
        apply_semantic_font(&handle, self.dpi.get(), self.font.0)?;
        apply_initial_properties(&handle);
        if let Some(name) = element.props().accessibility_name() {
            set_accessible_name(hwnd, name)?;
        }
        if let Props::List { pattern, .. } = element.props()
            && !pattern.supports_hierarchy()
        {
            // SAFETY: the message configures a live SysListView32 instance.
            unsafe {
                let _ = send_message(
                    hwnd,
                    LVM_SETEXTENDEDLISTVIEWSTYLE,
                    0,
                    (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
                );
            }
        }
        Ok(handle)
    }

    fn apply(&mut self, handle: &Self::Handle, patch: &PropertyPatch) -> Result<(), Self::Error> {
        let accessible_name = {
            let mut retained = handle.0.props.borrow_mut();
            let props = retained
                .as_mut()
                .ok_or_else(|| WindowsDiagnostic::InvalidNativeState {
                    reason: "property patch target has no retained properties".to_owned(),
                })?;
            props.clone_from(patch.props());
            props.accessibility_name().map(str::to_owned)
        };
        apply_semantic_font(handle, self.dpi.get(), self.font.0)?;
        apply_patch_to_native(handle, patch);
        if let Some(name) = accessible_name {
            set_accessible_name(handle.0.hwnd, &name)?;
        }
        if handle.0.kind == HostKind::Element(ElementKind::List) {
            self.rebuild_list(handle);
        }
        if handle.0.kind == HostKind::Element(ElementKind::ListRow) {
            let owner = handle
                .0
                .row
                .as_ref()
                .map_or(null_mut(), |state| state.owner.get());
            if !owner.is_null()
                && let Some(list) = find_ancestor_list(&self.root, owner)
            {
                self.rebuild_list(&list);
            }
        }
        self.layout_root();
        Ok(())
    }

    fn insert_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        // SAFETY: both HWND values are live children on this UI thread.
        unsafe {
            let _ = SetParent(child.0.hwnd, parent.0.hwnd);
        }
        let mut children = parent.0.children.borrow_mut();
        let insertion = index.min(children.len());
        children.insert(insertion, child.clone());
        drop(children);
        if parent.0.kind == HostKind::Element(ElementKind::List) {
            self.rebuild_list(parent);
        }
        self.layout_root();
        Ok(())
    }

    fn remove_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        let mut children = parent.0.children.borrow_mut();
        if children
            .get(index)
            .is_some_and(|value| Rc::ptr_eq(&value.0, &child.0))
        {
            children.remove(index);
        } else if let Some(position) = children
            .iter()
            .position(|value| Rc::ptr_eq(&value.0, &child.0))
        {
            children.remove(position);
        }
        drop(children);
        if parent.0.kind == HostKind::Element(ElementKind::List) {
            self.rebuild_list(parent);
        }
        self.layout_root();
        Ok(())
    }

    fn move_child(
        &mut self,
        parent: &Self::Handle,
        _child: &Self::Handle,
        from: usize,
        to: usize,
    ) -> Result<(), Self::Error> {
        let mut children = parent.0.children.borrow_mut();
        if from < children.len() {
            let child = children.remove(from);
            let destination = to.min(children.len());
            children.insert(destination, child);
        }
        drop(children);
        if parent.0.kind == HostKind::Element(ElementKind::List) {
            self.rebuild_list(parent);
        }
        self.layout_root();
        Ok(())
    }
}

enum Command {
    Activate(EventBindings),
    Input {
        hwnd: HWND,
        events: EventBindings,
    },
    Select {
        value: String,
        events: EventBindings,
    },
    Menu {
        hwnd: HWND,
        entries: Vec<MenuCommand>,
    },
    ToggleSidebar {
        hwnd: HWND,
    },
    ToggleInspector {
        hwnd: HWND,
    },
}

enum MenuCommand {
    Action {
        label: String,
        enabled: bool,
        events: EventBindings,
    },
    Separator,
}

struct ToolbarControl {
    hwnd: HWND,
    width: i32,
    right_aligned: bool,
    essential: bool,
    symbol_only: bool,
}

struct HostWindow {
    hwnd: HWND,
    root: HWND,
    runtime: WindowRuntime<WindowsBackend>,
    commands: HashMap<usize, Command>,
    toolbar: Vec<ToolbarControl>,
    tooltip: HWND,
    tooltip_texts: Vec<Box<[u16]>>,
    next_id: usize,
    dpi: u32,
    minimum_width: i32,
    minimum_height: i32,
    window_style: u32,
    window_extended_style: u32,
    dark: bool,
    font: Rc<NativeFont>,
    symbol_font: Rc<NativeFont>,
    panel_behavior: Option<PanelBehavior>,
    background_brush: HBRUSH,
}

impl Drop for HostWindow {
    fn drop(&mut self) {
        if !self.tooltip.is_null() {
            // SAFETY: the tooltip is owned only by this host window.
            unsafe {
                let _ = DestroyWindow(self.tooltip);
            }
        }
        if !self.background_brush.is_null() {
            // SAFETY: the brush is owned only by this host and no paint can run after teardown.
            unsafe {
                let _ = DeleteObject(self.background_brush as HGDIOBJ);
            }
        }
    }
}

impl HostWindow {
    fn toolbar_height(&self) -> i32 {
        if self.toolbar.is_empty() {
            0
        } else {
            scale(TOOLBAR_HEIGHT, self.dpi)
        }
    }

    fn command(&mut self, id: usize, notification: u16) {
        let Some(command) = self.commands.get(&id) else {
            return;
        };
        match command {
            Command::Activate(events) if notification == BN_CLICKED => events.emit_activate(),
            Command::Input { hwnd, events } if notification == EN_CHANGE => {
                events.emit_input(window_text(*hwnd));
            }
            Command::Select { value, events } if notification == BN_CLICKED => {
                events.emit_input(value.clone());
            }
            Command::Menu { hwnd, entries } if notification == BN_CLICKED => {
                show_command_menu(self.hwnd, *hwnd, entries);
            }
            Command::ToggleSidebar { hwnd } if notification == BN_CLICKED => {
                let checked = button_checked(*hwnd);
                self.runtime.with_renderer_mut(|renderer| {
                    renderer.backend_mut().sidebar_visible.set(checked);
                    renderer.backend().layout_root();
                });
            }
            Command::ToggleInspector { hwnd } if notification == BN_CLICKED => {
                let checked = button_checked(*hwnd);
                self.runtime.with_renderer_mut(|renderer| {
                    renderer.backend_mut().inspector_visible.set(checked);
                    renderer.backend().layout_root();
                });
            }
            _ => {}
        }
    }

    fn relayout(&mut self, width: i32, height: i32) {
        let toolbar_height = self.toolbar_height();
        move_window(
            self.root,
            0,
            toolbar_height,
            width,
            (height - toolbar_height).max(0),
            true,
        );
        let padding = scale(10, self.dpi);
        let gap = scale(6, self.dpi);
        let control_height = scale(32, self.dpi);
        let mut left = padding;
        let mut right = width - padding;
        for essential in [true, false] {
            for control in self
                .toolbar
                .iter_mut()
                .filter(|value| value.right_aligned && value.essential == essential)
            {
                let control_width = scale(control.width, self.dpi);
                right -= control_width;
                let visible = right > width / 2;
                show(control.hwnd, visible);
                if visible {
                    move_window(
                        control.hwnd,
                        right,
                        scale(10, self.dpi),
                        control_width,
                        control_height,
                        true,
                    );
                }
                right -= gap;
            }
        }
        for control in self.toolbar.iter_mut().filter(|value| !value.right_aligned) {
            let control_width = scale(control.width, self.dpi);
            let visible = left + control_width < right;
            show(control.hwnd, visible);
            if visible {
                move_window(
                    control.hwnd,
                    left,
                    scale(10, self.dpi),
                    control_width,
                    control_height,
                    true,
                );
                left += control_width + gap;
            }
        }
        self.runtime.with_renderer_mut(|renderer| {
            renderer.backend().layout_root();
        });
    }

    fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi.max(96);
        let Ok(font) = system_message_font(self.dpi) else {
            self.runtime
                .with_renderer_mut(|renderer| renderer.backend_mut().dpi.set(self.dpi));
            return;
        };
        let Ok(symbol_font) = system_symbol_font(self.dpi) else {
            self.runtime
                .with_renderer_mut(|renderer| renderer.backend_mut().dpi.set(self.dpi));
            return;
        };
        for control in &self.toolbar {
            set_native_font(
                control.hwnd,
                if control.symbol_only {
                    symbol_font.0
                } else {
                    font.0
                },
            );
        }
        self.runtime.with_renderer_mut(|renderer| {
            renderer.backend_mut().set_dpi(self.dpi, font.clone());
        });
        self.font = font;
        self.symbol_font = symbol_font;
    }
}
