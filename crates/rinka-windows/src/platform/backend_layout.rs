impl WindowsBackend {
    fn new(root: HWND, dpi: u32, font: Rc<NativeFont>, dark: bool) -> Self {
        Self {
            root: WindowsHandle::new(
                root,
                HostKind::Root,
                None,
                EventBindings::default(),
                true,
                dark,
            ),
            font,
            dpi: Cell::new(dpi),
            dark,
            sidebar_visible: Cell::new(true),
            inspector_visible: Cell::new(true),
        }
    }

    fn set_dpi(&mut self, dpi: u32, font: Rc<NativeFont>) {
        self.dpi.set(dpi.max(96));
        self.font = font;
        if apply_semantic_font_tree(&self.root, self.dpi.get(), self.font.0).is_err() {
            clear_semantic_fonts(&self.root, self.font.0);
        }
    }

    fn layout_root(&self) {
        let mut rect = RECT::default();
        // SAFETY: root is a live child HWND while the window runtime owns this backend.
        unsafe {
            let _ = GetClientRect(self.root.0.hwnd, &mut rect);
        }
        if let Some(child) = self.root.0.children.borrow().first() {
            self.layout_handle(child, 0, 0, rect.right - rect.left, rect.bottom - rect.top);
        }
    }

    fn layout_handle(&self, handle: &WindowsHandle, x: i32, y: i32, width: i32, height: i32) {
        move_window(handle.0.hwnd, x, y, width.max(0), height.max(0), true);
        let props = handle.0.props.borrow().clone();
        let children = handle.0.children.borrow().clone();
        match props {
            Some(Props::Stack {
                axis,
                spacing,
                padding,
                align,
                justify,
            }) => self.layout_stack(
                &children, width, height, axis, spacing, padding, align, justify,
            ),
            Some(Props::Scroll { .. }) => {
                if let Some(child) = children.first() {
                    self.layout_handle(child, 0, 0, width, height);
                }
            }
            Some(Props::Pattern {
                pattern: UiPattern::NavigationWorkspace { .. },
            }) => self.layout_workspace(&children, width, height),
            Some(Props::Pattern { pattern }) => {
                self.layout_split(&children, width, height, pattern);
            }
            Some(Props::List {
                pattern: CollectionPattern::DataTable,
                ..
            }) => self.size_table_columns(handle, width),
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_stack(
        &self,
        children: &[WindowsHandle],
        width: i32,
        height: i32,
        axis: Axis,
        spacing: Spacing,
        padding: Option<Spacing>,
        align: Align,
        justify: Justify,
    ) {
        let gap = self.metric(spacing);
        let inset = padding.map_or(0, |value| self.metric(value));
        let available_width = (width - inset * 2).max(0);
        let available_height = (height - inset * 2).max(0);
        let main_extent = if axis == Axis::Horizontal {
            available_width
        } else {
            available_height
        };
        let gap_total = gap * i32::try_from(children.len().saturating_sub(1)).unwrap_or(i32::MAX);
        let fixed = children
            .iter()
            .filter(|child| !is_flexible(child, axis))
            .map(|child| self.desired(child, axis))
            .sum::<i32>();
        let flexible_count = children
            .iter()
            .filter(|child| is_flexible(child, axis))
            .count();
        let remaining = (main_extent - fixed - gap_total).max(0);
        let flexible_extent = if flexible_count == 0 {
            0
        } else {
            remaining / i32::try_from(flexible_count).unwrap_or(1)
        };
        let occupied =
            fixed + gap_total + flexible_extent * i32::try_from(flexible_count).unwrap_or_default();
        let mut cursor = inset
            + match justify {
                Justify::Start => 0,
                Justify::Center => (main_extent - occupied).max(0) / 2,
                Justify::End => (main_extent - occupied).max(0),
            };
        for child in children {
            let primary = if is_flexible(child, axis) {
                flexible_extent
            } else {
                self.desired(child, axis).min(main_extent)
            };
            let cross_available = if axis == Axis::Horizontal {
                available_height
            } else {
                available_width
            };
            let desired_cross = self.desired(
                child,
                if axis == Axis::Horizontal {
                    Axis::Vertical
                } else {
                    Axis::Horizontal
                },
            );
            let cross = if align == Align::Stretch || structural(child) {
                cross_available
            } else {
                desired_cross.min(cross_available)
            };
            let cross_origin = inset
                + match align {
                    Align::Start | Align::Stretch => 0,
                    Align::Center => (cross_available - cross).max(0) / 2,
                    Align::End => (cross_available - cross).max(0),
                };
            if axis == Axis::Horizontal {
                self.layout_handle(child, cursor, cross_origin, primary, cross);
            } else {
                self.layout_handle(child, cross_origin, cursor, cross, primary);
            }
            cursor += primary + gap;
        }
    }

    fn layout_split(
        &self,
        children: &[WindowsHandle],
        width: i32,
        height: i32,
        pattern: UiPattern,
    ) {
        let secondary = match pattern {
            UiPattern::NavigationSplit { .. } => self.scale(240),
            UiPattern::UtilitySplit { .. } => self.scale(288),
            UiPattern::NavigationWorkspace { .. } => unreachable!(),
        }
        .min((width / 2).max(0));
        let primary = width - secondary - 1;
        match pattern {
            UiPattern::NavigationSplit { .. } => {
                if let Some(sidebar) = children.first() {
                    self.layout_handle(sidebar, 0, 0, secondary, height);
                }
                if let Some(content) = children.get(1) {
                    self.layout_handle(content, secondary + 1, 0, primary, height);
                }
            }
            UiPattern::UtilitySplit { .. } => {
                if let Some(content) = children.first() {
                    self.layout_handle(content, 0, 0, primary, height);
                }
                if let Some(inspector) = children.get(1) {
                    self.layout_handle(inspector, primary + 1, 0, secondary, height);
                }
            }
            UiPattern::NavigationWorkspace { .. } => unreachable!(),
        }
    }

    fn layout_workspace(&self, children: &[WindowsHandle], width: i32, height: i32) {
        let mut sidebar = if self.sidebar_visible.get() {
            self.scale(236)
        } else {
            0
        };
        let mut inspector = if self.inspector_visible.get() {
            self.scale(284)
        } else {
            0
        };
        if width < self.scale(900) {
            sidebar = sidebar.min(self.scale(190));
            inspector = inspector.min(self.scale(220));
        }
        let minimum_content = self.scale(320);
        let side_total = (width - minimum_content).max(0);
        if sidebar + inspector > side_total && sidebar + inspector > 0 {
            let original = sidebar + inspector;
            sidebar = sidebar * side_total / original;
            inspector = side_total - sidebar;
        }
        let content_width = (width - sidebar - inspector).max(0);
        if let Some(value) = children.first() {
            show(value.0.hwnd, sidebar > 0);
            if sidebar > 0 {
                self.layout_handle(value, 0, 0, sidebar, height);
            }
        }
        if let Some(value) = children.get(1) {
            self.layout_handle(value, sidebar, 0, content_width, height);
        }
        if let Some(value) = children.get(2) {
            show(value.0.hwnd, inspector > 0);
            if inspector > 0 {
                self.layout_handle(value, sidebar + content_width, 0, inspector, height);
            }
        }
    }

    fn desired(&self, handle: &WindowsHandle, axis: Axis) -> i32 {
        match handle.0.props.borrow().as_ref() {
            Some(Props::Label { role, text, .. }) => match axis {
                Axis::Horizontal => {
                    self.scale((text.chars().count() as i32 * 7 + 12).clamp(40, 440))
                }
                Axis::Vertical => self.scale(match role {
                    TextRole::Title => 32,
                    TextRole::Heading => 26,
                    _ => 22,
                }),
            },
            Some(Props::Button { label, size, .. }) => match axis {
                Axis::Horizontal => {
                    self.scale((label.chars().count() as i32 * 8 + 28).clamp(72, 220))
                }
                Axis::Vertical => self.control_height(*size),
            },
            Some(Props::Input { .. }) => match axis {
                Axis::Horizontal => self.scale(180),
                Axis::Vertical => self.scale(30),
            },
            Some(Props::Toggle { label, size, .. }) => match axis {
                Axis::Horizontal => {
                    self.scale((label.chars().count() as i32 * 8 + 28).clamp(100, 260))
                }
                Axis::Vertical => self.control_height(*size),
            },
            Some(Props::Progress { .. }) => match axis {
                Axis::Horizontal => self.scale(220),
                Axis::Vertical => self.scale(18),
            },
            Some(Props::Separator { .. }) => self.scale(1),
            Some(Props::Spacer {
                horizontal,
                vertical,
            }) => match axis {
                Axis::Horizontal if *horizontal => 0,
                Axis::Vertical if *vertical => 0,
                _ => 1,
            },
            Some(Props::Stack {
                axis: stack_axis,
                spacing,
                padding,
                ..
            }) if *stack_axis == axis => {
                let child_sum = handle
                    .0
                    .children
                    .borrow()
                    .iter()
                    .map(|child| self.desired(child, axis))
                    .sum::<i32>();
                let count = handle.0.children.borrow().len();
                child_sum
                    + self.metric(*spacing)
                        * i32::try_from(count.saturating_sub(1)).unwrap_or_default()
                    + padding.map_or(0, |value| self.metric(value) * 2)
            }
            Some(Props::Stack { padding, .. }) => {
                handle
                    .0
                    .children
                    .borrow()
                    .iter()
                    .map(|child| self.desired(child, axis))
                    .max()
                    .unwrap_or(0)
                    + padding.map_or(0, |value| self.metric(value) * 2)
            }
            Some(Props::Status { title, message, .. }) => match axis {
                Axis::Horizontal => self.scale(
                    (title.chars().count().max(message.chars().count()) as i32 * 7 + 40)
                        .clamp(180, 560),
                ),
                Axis::Vertical => self.scale(86),
            },
            _ => match axis {
                Axis::Horizontal => self.scale(160),
                Axis::Vertical => self.scale(32),
            },
        }
    }

    fn metric(&self, spacing: Spacing) -> i32 {
        self.scale(match spacing {
            Spacing::Joined => 0,
            Spacing::Compact => 4,
            Spacing::Related => 8,
            Spacing::Section => 16,
            Spacing::Content => 20,
        })
    }

    fn control_height(&self, size: ControlSize) -> i32 {
        self.scale(match size {
            ControlSize::Mini => 22,
            ControlSize::Small => 26,
            ControlSize::Regular => 30,
            ControlSize::Large => 36,
            ControlSize::ExtraLarge => 42,
        })
    }

    fn scale(&self, value: i32) -> i32 {
        value.saturating_mul(self.dpi.get() as i32) / 96
    }

    fn size_table_columns(&self, handle: &WindowsHandle, width: i32) {
        let Some(Props::List { columns, .. }) = handle.0.props.borrow().clone() else {
            return;
        };
        if columns.is_empty() {
            return;
        }
        let remaining = width.max(self.scale(400));
        let first = (remaining * 40 / 100).max(self.scale(180));
        // SAFETY: the messages target a live SysListView32 HWND and use integer widths.
        unsafe {
            for index in 0..columns.len() {
                let column_width = if index == 0 {
                    first
                } else {
                    ((remaining - first) / i32::try_from(columns.len() - 1).unwrap_or(1))
                        .max(self.scale(100))
                };
                let _ = send_message(
                    handle.0.hwnd,
                    LVM_SETCOLUMNWIDTH,
                    index,
                    column_width as isize,
                );
            }
        }
    }

    fn rebuild_list(&self, list: &WindowsHandle) {
        let props = list.0.props.borrow().clone();
        let Some(Props::List {
            pattern,
            columns,
            ..
        }) = props
        else {
            return;
        };
        let rows = list.0.children.borrow().clone();
        list.0.list_rebuilding.set(true);
        // SAFETY: all message structures remain alive for the synchronous SendMessage calls.
        unsafe {
            if pattern.supports_hierarchy() {
                let _ = send_message(list.0.hwnd, TVM_DELETEITEM, 0, TVI_ROOT);
                for row in &rows {
                    self.insert_tree_row(list.0.hwnd, row, TVI_ROOT);
                }
            } else {
                let _ = send_message(list.0.hwnd, LVM_DELETEALLITEMS, 0, 0);
                if pattern.presents_columns() {
                    for index in 0..32 {
                        if send_message(list.0.hwnd, LVM_DELETECOLUMN, 0, 0) == 0 {
                            break;
                        }
                        if index == 31 {
                            break;
                        }
                    }
                    for (index, column) in columns.iter().enumerate() {
                        let mut title = wide(&column.title);
                        let mut native = LvColumnW {
                            mask: LVCF_FMT | LVCF_WIDTH | LVCF_TEXT | LVCF_SUBITEM,
                            fmt: 0,
                            cx: self.scale(if index == 0 { 260 } else { 140 }),
                            psz_text: title.as_mut_ptr(),
                            cch_text_max: i32::try_from(title.len()).unwrap_or(i32::MAX),
                            i_sub_item: i32::try_from(index).unwrap_or(i32::MAX),
                            i_image: 0,
                            i_order: 0,
                            cx_min: 0,
                            cx_default: 0,
                            cx_ideal: 0,
                        };
                        let _ = send_message(
                            list.0.hwnd,
                            LVM_INSERTCOLUMNW,
                            index,
                            (&raw mut native) as isize,
                        );
                    }
                }
                let mut next = 0;
                for row in &rows {
                    self.insert_list_row(list.0.hwnd, row, pattern, &mut next, 0);
                }
            }
        }
        list.0.list_rebuilding.set(false);
    }

    unsafe fn insert_tree_row(&self, owner: HWND, row: &WindowsHandle, parent: isize) {
        let Some(Props::ListRow {
            title,
            role,
            expanded,
            selected,
            ..
        }) = row.0.props.borrow().clone()
        else {
            return;
        };
        let mut text = wide(&title);
        let mut insertion = TvInsertStructW {
            parent,
            insert_after: TVI_LAST,
            item: TvItemExW {
                mask: TVIF_TEXT | TVIF_PARAM | TVIF_STATE | TVIF_CHILDREN,
                item: 0,
                state: if role == ListRowRole::Section {
                    TVIS_BOLD
                } else {
                    0
                } | if selected { TVIS_SELECTED } else { 0 },
                state_mask: TVIS_BOLD | TVIS_SELECTED,
                psz_text: text.as_mut_ptr(),
                cch_text_max: i32::try_from(text.len()).unwrap_or(i32::MAX),
                i_image: 0,
                i_selected_image: 0,
                c_children: i32::from(!row.0.children.borrow().is_empty()),
                l_param: Rc::as_ptr(&row.0) as LPARAM,
                i_integral: 0,
                state_ex: 0,
                hwnd: null_mut(),
                i_expanded_image: 0,
                i_reserved: 0,
            },
        };
        // SAFETY: insertion points at a fully initialized structure for a live tree view.
        let native_item =
            unsafe { send_message(owner, TVM_INSERTITEMW, 0, (&raw mut insertion) as isize) };
        if let Some(state) = &row.0.row {
            state.owner.set(owner);
            state.tree_item.set(native_item);
        }
        for child in row.0.children.borrow().iter() {
            // SAFETY: child insertion occurs synchronously under the native parent item.
            unsafe { self.insert_tree_row(owner, child, native_item) };
        }
        if expanded {
            // SAFETY: the item was returned by this tree control.
            unsafe {
                let _ = send_message(owner, TVM_EXPAND, TVE_EXPAND, native_item);
            }
        }
    }

    unsafe fn insert_list_row(
        &self,
        owner: HWND,
        row: &WindowsHandle,
        pattern: CollectionPattern,
        next: &mut i32,
        depth: i32,
    ) {
        let Some(Props::ListRow {
            title,
            cells,
            expanded,
            selected,
            ..
        }) = row.0.props.borrow().clone()
        else {
            return;
        };
        let index = *next;
        *next += 1;
        let has_children = !row.0.children.borrow().is_empty();
        let display_title = if has_children {
            format!("{} {title}", if expanded { "▾" } else { "▸" })
        } else {
            title
        };
        let mut title = wide(&display_title);
        let mut native = LvItemW {
            mask: LVIF_TEXT | LVIF_PARAM | LVIF_STATE | LVIF_INDENT,
            i_item: index,
            i_sub_item: 0,
            state: if selected { LVIS_SELECTED } else { 0 },
            state_mask: LVIS_SELECTED,
            psz_text: title.as_mut_ptr(),
            cch_text_max: i32::try_from(title.len()).unwrap_or(i32::MAX),
            i_image: 0,
            l_param: Rc::as_ptr(&row.0) as LPARAM,
            i_indent: depth,
            i_group_id: 0,
            c_columns: 0,
            pu_columns: null_mut(),
            pi_col_fmt: null_mut(),
            i_group: 0,
        };
        // SAFETY: item points at initialized storage for a live list view.
        unsafe {
            let _ = send_message(owner, LVM_INSERTITEMW, 0, (&raw mut native) as isize);
        }
        if pattern.presents_columns() {
            for (offset, cell) in cells.iter().enumerate() {
                let mut value = wide(cell);
                native.i_sub_item = i32::try_from(offset + 1).unwrap_or(i32::MAX);
                native.psz_text = value.as_mut_ptr();
                // SAFETY: each subitem update is synchronous and `value` remains alive.
                unsafe {
                    let _ = send_message(
                        owner,
                        LVM_SETITEMTEXTW,
                        index as usize,
                        (&raw mut native) as isize,
                    );
                }
            }
        }
        if let Some(state) = &row.0.row {
            state.owner.set(owner);
            state.list_index.set(index);
        }
        if expanded {
            for child in row.0.children.borrow().iter() {
                // SAFETY: list rows are inserted synchronously into the same owner control.
                unsafe { self.insert_list_row(owner, child, pattern, next, depth + 1) };
            }
        }
    }
}
