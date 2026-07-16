/// Reconciler adapter for GTK widgets.
#[derive(Debug)]
pub struct GtkBackend {
    root: GtkHandle,
    layout_context: LayoutContext,
}

impl GtkBackend {
    fn new(root: &gtk::Box, layout_context: LayoutContext) -> Self {
        Self {
            root: GtkHandle::new(root.clone(), HostKind::Root, None, Vec::new()),
            layout_context,
        }
    }
}

impl NativeBackend for GtkBackend {
    type Handle = GtkHandle;
    type Error = GtkError;

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
        create_element(element, events, self.layout_context)
    }

    fn apply(&mut self, handle: &Self::Handle, patch: &PropertyPatch) -> Result<(), Self::Error> {
        apply_patch(handle, patch, self.layout_context)
    }

    fn insert_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        insert_child(parent, child, index)
    }

    fn remove_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        remove_child(parent, child, index)
    }

    fn move_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        from: usize,
        to: usize,
    ) -> Result<(), Self::Error> {
        move_child(parent, child, from, to)
    }
}

fn validate_element(element: &Element) -> Result<(), GtkError> {
    if !element.accelerator_table().is_empty() {
        // GtkShortcutController mapping is not implemented yet; rejecting the
        // declared table keeps the contract honest instead of silently
        // dropping chords (reports/keyboard-shortcuts-and-key-events). The
        // chord-to-trigger mapping is already provided and unit-tested in
        // crate::accelerator_mapping for that integration.
        return Err(GtkError(
            "declared accelerator tables are not yet delivered by the GTK host".to_owned(),
        ));
    }
    if element.menu_bar_model().is_some() {
        // Neither GMenu + set_menubar nor the libadwaita header-bar primary
        // menu is realized yet; rejecting the declared bar keeps the contract
        // honest instead of silently dropping menus (reports/app-menu-bar).
        return Err(GtkError(
            "a declared application menu bar is not yet realized by the GTK host".to_owned(),
        ));
    }
    if element.context_menu_model().is_some() {
        // The GTK realization (GtkPopoverMenu over the gio::Menu model, with
        // per-row popovers inside the ColumnView factories) does not exist
        // yet; the typed rejection and its follow-up are recorded in
        // reports/context-menus.
        return Err(GtkError(
            "GTK adapter does not realize element context menus yet".to_owned(),
        ));
    }
    if element.file_promise_model().is_some()
        || element.drag_payload_model().is_some()
        || element.drop_target_model().is_some()
    {
        // The GTK realization (DropTarget/DragSource event controllers) does
        // not exist yet; the typed rejection and its follow-up are recorded
        // in reports/drag-and-drop.
        return Err(GtkError(
            "GTK adapter does not realize drag-and-drop declarations yet".to_owned(),
        ));
    }
    if let Some(name) = element.props().accessibility_name() {
        require_text("accessibility name", name)?;
    }
    match element.props() {
        Props::Button {
            label, material, ..
        } => {
            require_text("button label", label)?;
            if *material == ButtonMaterial::Glass {
                return Err(GtkError(
                    "GTK does not provide the requested glass button material".to_owned(),
                ));
            }
        }
        Props::Input { .. } => {}
        Props::TextArea { .. } => {
            // Typed unsupported-capability rejection per the AGENTS contract:
            // the GTK adapter does not yet realize the multi-line text area
            // (GtkTextView over a GtkTextBuffer, with the controlled-text
            // protocol driven from buffer change signals, is the intended
            // mapping) and never substitutes another control for it.
            return Err(GtkError(
                "the GTK host does not yet realize the multi-line text area".to_owned(),
            ));
        }
        Props::Toggle { label, .. } => {
            require_text("toggle label", label)?;
        }
        Props::Progress { fraction, .. } => {
            if !fraction.is_finite() || !(0.0..=1.0).contains(fraction) {
                return Err(GtkError(format!(
                    "progress fraction must be finite and within 0..=1, received {fraction}"
                )));
            }
        }
        Props::List {
            columns,
            ..
        } => {
            for column in columns {
                require_text("table column title", &column.title)?;
            }
        }
        Props::ListRow { .. } => {}
        Props::Status { title, message, .. } => {
            require_text("status title", title)?;
            require_text("status message", message)?;
        }
        Props::Canvas { .. } => {
            // Typed unsupported-capability rejection: the GTK adapter does
            // not yet realize the owned-drawing canvas (GtkDrawingArea +
            // cairo is the intended backing) and never substitutes another
            // control for it.
            return Err(GtkError(
                "GTK adapter does not implement the owned-drawing canvas element yet".to_owned(),
            ));
        }
        // Typed diagnostic per the AGENTS contract: the GTK host does not
        // yet realize the bitmap image element (GtkPicture over a
        // GdkMemoryTexture is the planned mapping), and it must reject the
        // tree instead of substituting an unrelated control.
        Props::Image { .. } => {
            return Err(GtkError(
                "the GTK host does not yet realize the bitmap image element".to_owned(),
            ));
        }
        Props::Dock { .. } => {
            // Typed unsupported-capability rejection per the AGENTS contract:
            // the GTK adapter does not yet realize the tabbed-document dock.
            // The intended mapping is genuinely native — AdwTabBar +
            // AdwTabView per group (native drag reorder, transfer between
            // views, and detach signals) over recursive GtkPaned splits —
            // and is recorded in reports/document-tabs-and-splits; until it
            // exists the declaration is rejected, never substituted.
            return Err(GtkError(
                "the GTK host does not yet realize the tabbed-document dock".to_owned(),
            ));
        }
        Props::Label { .. }
        | Props::Separator { .. }
        | Props::Spacer { .. }
        | Props::Stack { .. }
        | Props::Scroll { .. }
        | Props::Pattern { .. } => {}
    }
    Ok(())
}

fn require_text(field: &str, value: &str) -> Result<(), GtkError> {
    if value.trim().is_empty() {
        Err(GtkError(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

fn create_element(
    element: &Element,
    events: EventBindings,
    layout_context: LayoutContext,
) -> Result<GtkHandle, GtkError> {
    match element.props() {
        Props::Label {
            text,
            role,
            selectable,
        } => {
            let label = gtk::Label::new(Some(text));
            label.set_xalign(0.0);
            label.set_selectable(*selectable);
            label.set_wrap(true);
            label.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
            configure_label(&label, *role);
            Ok(GtkHandle::new(
                label,
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
            let button = gtk::Button::with_label(label);
            let action = events.clone();
            button.connect_clicked(move |_| action.emit_activate());
            configure_button(
                &button,
                *role,
                *size,
                *material,
                *enabled,
                tooltip.as_deref(),
                accessibility_label,
            );
            configure_button_context(&button, *size, layout_context);
            Ok(GtkHandle::new(
                button,
                HostKind::Element(ElementKind::Button),
                None,
                Vec::new(),
            ))
        }
        Props::Input {
            value,
            placeholder,
            kind,
            enabled,
            accessibility_label,
        } => create_input(
            value,
            placeholder,
            *kind,
            *enabled,
            accessibility_label,
            events,
        ),
        Props::Toggle {
            label,
            value,
            size: _,
            enabled,
            accessibility_label,
        } => {
            let row = adw::ActionRow::builder().title(label).build();
            let suppress_events = Rc::new(Cell::new(false));
            let toggle = gtk::Switch::builder()
                .active(*value)
                .sensitive(*enabled)
                .valign(gtk::Align::Center)
                .build();
            let action = events.clone();
            let signal_guard = suppress_events.clone();
            toggle.connect_active_notify(move |toggle| {
                if !signal_guard.get() {
                    action.emit_toggle(toggle.is_active());
                }
            });
            toggle.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            row.add_suffix(&toggle);
            row.set_activatable_widget(Some(&toggle));
            Ok(GtkHandle::with_suppression(
                row,
                HostKind::Element(ElementKind::Toggle),
                vec![toggle.upcast()],
                suppress_events,
            ))
        }
        Props::Progress {
            fraction,
            accessibility_label,
        } => {
            let progress = gtk::ProgressBar::new();
            progress.set_fraction(*fraction);
            progress.set_show_text(true);
            progress.set_text(Some(&progress_percentage_text(*fraction)));
            progress.add_css_class("caption-heading");
            progress.set_hexpand(false);
            progress.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            let clamp = adw::Clamp::new();
            clamp.set_hexpand(false);
            clamp.set_maximum_size(240);
            clamp.set_tightening_threshold(240);
            clamp.set_margin_start(content_spacing_pixels(layout_context, Spacing::Content));
            clamp.set_margin_end(content_spacing_pixels(layout_context, Spacing::Content));
            clamp.set_child(Some(&progress));
            Ok(GtkHandle::new(
                clamp,
                HostKind::Element(ElementKind::Progress),
                None,
                vec![progress.upcast()],
            ))
        }
        // Unreachable in practice: validate_element rejects image content
        // before any native mutation. Kept as a typed diagnostic so a
        // bypassed validation cannot silently substitute a control.
        Props::Image { .. } => Err(GtkError(
            "the GTK host does not yet realize the bitmap image element".to_owned(),
        )),
        // Unreachable in practice for the same reason as Image above.
        Props::TextArea { .. } => Err(GtkError(
            "the GTK host does not yet realize the multi-line text area".to_owned(),
        )),
        Props::Separator { axis } => Ok(GtkHandle::new(
            gtk::Separator::new(orientation(*axis)),
            HostKind::Element(ElementKind::Separator),
            None,
            Vec::new(),
        )),
        Props::Spacer {
            horizontal,
            vertical,
        } => {
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_hexpand(*horizontal);
            spacer.set_vexpand(*vertical);
            Ok(GtkHandle::new(
                spacer,
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
        } => {
            let container = gtk::Box::new(
                orientation(*axis),
                content_spacing_pixels(layout_context, *spacing),
            );
            configure_stack(
                &container,
                *axis,
                *padding,
                *align,
                *justify,
                layout_context,
            );
            Ok(GtkHandle::new(
                container,
                HostKind::Element(ElementKind::Stack),
                None,
                Vec::new(),
            ))
        }
        Props::Scroll { axis } => {
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_hexpand(true);
            scroll.set_vexpand(true);
            configure_scroll(&scroll, *axis);
            Ok(GtkHandle::new(
                scroll,
                HostKind::Element(ElementKind::Scroll),
                None,
                Vec::new(),
            ))
        }
        Props::Pattern {
            pattern: pattern @ (UiPattern::NavigationSplit { .. } | UiPattern::UtilitySplit { .. }),
        } => {
            let split = adw::OverlaySplitView::new();
            split.set_hexpand(true);
            split.set_vexpand(true);
            split.set_collapsed(false);
            let region = pattern
                .auxiliary_region()
                .expect("two-region pattern must declare one auxiliary region");
            let collapsible = pattern.region_is_collapsible(region);
            split.set_enable_show_gesture(collapsible);
            split.set_enable_hide_gesture(collapsible);
            if matches!(pattern, UiPattern::UtilitySplit { .. }) {
                split.set_sidebar_position(gtk::PackType::End);
                split.set_sidebar_width_unit(adw::LengthUnit::Sp);
                split.set_min_sidebar_width(UTILITY_PANE_MIN_WIDTH_SP);
            }
            Ok(GtkHandle::new(
                split,
                HostKind::Element(ElementKind::Pattern),
                Some(*pattern),
                Vec::new(),
            ))
        }
        Props::Pattern {
            pattern:
                pattern @ UiPattern::NavigationWorkspace {
                    sidebar_collapsible,
                    inspector_collapsible,
                },
        } => {
            let navigation = adw::OverlaySplitView::new();
            navigation.set_hexpand(true);
            navigation.set_vexpand(true);
            navigation.set_enable_show_gesture(*sidebar_collapsible);
            navigation.set_enable_hide_gesture(*sidebar_collapsible);
            navigation.set_collapsed(false);
            let inspector = adw::OverlaySplitView::new();
            inspector.set_hexpand(true);
            inspector.set_vexpand(true);
            inspector.set_sidebar_position(gtk::PackType::End);
            inspector.set_sidebar_width_unit(adw::LengthUnit::Sp);
            inspector.set_min_sidebar_width(UTILITY_PANE_MIN_WIDTH_SP);
            inspector.set_enable_show_gesture(*inspector_collapsible);
            inspector.set_enable_hide_gesture(*inspector_collapsible);
            inspector.set_collapsed(false);
            navigation.set_content(Some(&inspector));
            let data = WorkspaceData {
                navigation: navigation.clone(),
                inspector,
            };
            Ok(GtkHandle::workspace(navigation, *pattern, data))
        }
        Props::List {
            accessibility_label,
            pattern,
            columns,
        } => {
            let data = ListData::new(accessibility_label, *pattern, columns, events);
            Ok(GtkHandle::list(data.scroll.clone(), data))
        }
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
        } => {
            let data = RowData::new(
                title,
                subtitle.as_deref(),
                cells,
                *role,
                *expanded,
                *symbol,
                *selected,
                *disclosure,
                accessibility_label,
                events,
            );
            let object = glib::BoxedAnyObject::new(data.clone());
            Ok(GtkHandle::row(data, object))
        }
        Props::Status {
            title,
            message,
            tone,
        } => {
            let page = adw::StatusPage::builder()
                .title(title)
                .description(message)
                .icon_name(status_icon(*tone))
                .build();
            page.add_css_class("compact");
            page.set_vexpand(false);
            Ok(GtkHandle::new(
                page,
                HostKind::Element(ElementKind::Status),
                None,
                Vec::new(),
            ))
        }
        Props::Canvas { .. } => Err(GtkError(
            "GTK adapter does not implement the owned-drawing canvas element yet".to_owned(),
        )),
        // Unreachable in practice: validate_element rejects the dock before
        // any native mutation; the AdwTabBar/AdwTabView mapping is recorded
        // in reports/document-tabs-and-splits.
        Props::Dock { .. } => Err(GtkError(
            "the GTK host does not yet realize the tabbed-document dock".to_owned(),
        )),
    }
}

fn create_input(
    value: &str,
    placeholder: &str,
    kind: InputKind,
    enabled: bool,
    accessibility_label: &str,
    events: EventBindings,
) -> Result<GtkHandle, GtkError> {
    let suppress_events = Rc::new(Cell::new(false));
    let widget: gtk::Widget = match kind {
        InputKind::Search => {
            let input = gtk::SearchEntry::new();
            input.set_text(value);
            input.set_placeholder_text(Some(placeholder));
            let action = events.clone();
            let signal_guard = suppress_events.clone();
            input.connect_search_changed(move |input| {
                if !signal_guard.get() {
                    action.emit_input(input.text().to_string());
                }
            });
            input.upcast()
        }
        InputKind::Text => {
            let input = gtk::Entry::new();
            input.set_text(value);
            input.set_placeholder_text(Some(placeholder));
            let action = events.clone();
            let signal_guard = suppress_events.clone();
            input.connect_changed(move |input| {
                if !signal_guard.get() {
                    action.emit_input(input.text().to_string());
                }
            });
            input.upcast()
        }
        InputKind::Secure => {
            let input = gtk::PasswordEntry::new();
            input.set_text(value);
            input.set_placeholder_text(Some(placeholder));
            let action = events.clone();
            let signal_guard = suppress_events.clone();
            input.connect_changed(move |input| {
                if !signal_guard.get() {
                    action.emit_input(input.text().to_string());
                }
            });
            input.upcast()
        }
    };
    widget.set_sensitive(enabled);
    widget.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
    Ok(GtkHandle::with_suppression(
        widget,
        HostKind::Element(ElementKind::Input),
        Vec::new(),
        suppress_events,
    ))
}

fn configure_label(label: &gtk::Label, role: TextRole) {
    for class in [
        "title-1",
        "heading",
        "body",
        "caption",
        "dim-label",
        "monospace",
    ] {
        label.remove_css_class(class);
    }
    match role {
        TextRole::Title => label.add_css_class("title-1"),
        TextRole::Heading => label.add_css_class("heading"),
        TextRole::Body => label.add_css_class("body"),
        // A secondary label still carries actionable application state in a
        // compact status row. Adwaita's standard body token keeps that state
        // legible at narrow sizes without introducing adapter-owned font or
        // color values.
        TextRole::Secondary => label.add_css_class("body"),
        TextRole::Monospace => label.add_css_class("monospace"),
    }
}

fn configure_button(
    button: &gtk::Button,
    role: ButtonRole,
    size: ControlSize,
    material: ButtonMaterial,
    enabled: bool,
    tooltip: Option<&str>,
    accessibility_label: &str,
) {
    for class in [
        "suggested-action",
        "destructive-action",
        "flat",
        "pill",
        "compact",
    ] {
        button.remove_css_class(class);
    }
    match role {
        ButtonRole::Standard => {}
        ButtonRole::Primary => button.add_css_class("suggested-action"),
        ButtonRole::Destructive => button.add_css_class("destructive-action"),
        ButtonRole::Toolbar => button.add_css_class("flat"),
    }
    match size {
        ControlSize::Mini | ControlSize::Small => button.add_css_class("compact"),
        ControlSize::Regular => {}
        ControlSize::Large => button.add_css_class("pill"),
        ControlSize::ExtraLarge => button.add_css_class("pill"),
    }
    if material == ButtonMaterial::Glass {
        button.add_css_class("flat");
    }
    button.set_sensitive(enabled);
    button.set_tooltip_text(tooltip);
    button.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
}

fn configure_button_context(
    button: &gtk::Button,
    size: ControlSize,
    layout_context: LayoutContext,
) {
    if size == ControlSize::Regular && layout_context == LayoutContext::AuxiliaryPanel {
        button.add_css_class("compact");
    }
}

fn configure_stack(
    container: &gtk::Box,
    axis: Axis,
    padding: Option<Spacing>,
    align: Align,
    justify: Justify,
    layout_context: LayoutContext,
) {
    container.set_orientation(orientation(axis));
    if let Some(padding) = padding {
        let (horizontal_inset, vertical_inset) = stack_insets(layout_context, padding);
        container.set_margin_start(horizontal_inset);
        container.set_margin_end(horizontal_inset);
        container.set_margin_top(vertical_inset);
        container.set_margin_bottom(vertical_inset);
    } else {
        container.set_margin_start(0);
        container.set_margin_end(0);
        container.set_margin_top(0);
        container.set_margin_bottom(0);
    }
    match axis {
        Axis::Horizontal => {
            container.set_valign(gtk_align(align));
            container.set_vexpand(align == Align::Stretch);
        }
        Axis::Vertical => {
            container.set_halign(gtk_align(align));
            container.set_hexpand(align == Align::Stretch);
        }
    }
    let main_align = match justify {
        Justify::Start => gtk::Align::Fill,
        Justify::Center => gtk::Align::Center,
        Justify::End => gtk::Align::End,
    };
    match axis {
        Axis::Horizontal => {
            container.set_halign(main_align);
            container.set_hexpand(justify != Justify::Start);
        }
        Axis::Vertical => {
            container.set_valign(main_align);
            container.set_vexpand(justify != Justify::Start);
        }
    }
}

fn configure_scroll(scroll: &gtk::ScrolledWindow, axis: Axis) {
    match axis {
        Axis::Horizontal => scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never),
        Axis::Vertical => scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic),
    }
}

fn apply_patch(
    handle: &GtkHandle,
    patch: &PropertyPatch,
    layout_context: LayoutContext,
) -> Result<(), GtkError> {
    match patch.props() {
        Props::Label {
            text,
            role,
            selectable,
        } => {
            let label = downcast::<gtk::Label>(handle)?;
            label.set_label(text);
            label.set_selectable(*selectable);
            configure_label(&label, *role);
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
            let button = downcast::<gtk::Button>(handle)?;
            button.set_label(label);
            configure_button(
                &button,
                *role,
                *size,
                *material,
                *enabled,
                tooltip.as_deref(),
                accessibility_label,
            );
            configure_button_context(&button, *size, layout_context);
        }
        Props::Input {
            value,
            placeholder,
            enabled,
            accessibility_label,
            ..
        } => {
            handle.0.suppress_events.set(true);
            handle.widget().set_sensitive(*enabled);
            handle
                .widget()
                .update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            if let Ok(input) = handle.widget().clone().downcast::<gtk::SearchEntry>() {
                input.set_text(value);
                input.set_placeholder_text(Some(placeholder));
            } else if let Ok(input) = handle.widget().clone().downcast::<gtk::Entry>() {
                input.set_text(value);
                input.set_placeholder_text(Some(placeholder));
            } else if let Ok(input) = handle.widget().clone().downcast::<gtk::PasswordEntry>() {
                input.set_text(value);
                input.set_placeholder_text(Some(placeholder));
            }
            handle.0.suppress_events.set(false);
        }
        Props::Toggle {
            label,
            value,
            size: _,
            enabled,
            accessibility_label,
        } => {
            let row = downcast::<adw::ActionRow>(handle)?;
            row.set_title(label);
            let toggle = handle
                .0
                .auxiliaries
                .first()
                .and_then(|widget| widget.clone().downcast::<gtk::Switch>().ok())
                .ok_or_else(|| GtkError("toggle has no native switch".to_owned()))?;
            handle.0.suppress_events.set(true);
            toggle.set_active(*value);
            toggle.set_sensitive(*enabled);
            toggle.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            handle.0.suppress_events.set(false);
        }
        Props::Progress {
            fraction,
            accessibility_label,
        } => {
            let progress = native_progress(handle)?;
            progress.set_fraction(*fraction);
            progress.set_text(Some(&progress_percentage_text(*fraction)));
            progress.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
        }
        // Unreachable in practice: validate_element rejects image content
        // before any native mutation.
        Props::Image { .. } => {
            return Err(GtkError(
                "the GTK host does not yet realize the bitmap image element".to_owned(),
            ));
        }
        Props::Separator { axis } => {
            downcast::<gtk::Separator>(handle)?.set_orientation(orientation(*axis));
        }
        Props::Stack {
            axis,
            spacing,
            padding,
            align,
            justify,
        } => {
            let container = downcast::<gtk::Box>(handle)?;
            container.set_spacing(content_spacing_pixels(layout_context, *spacing));
            configure_stack(
                &container,
                *axis,
                *padding,
                *align,
                *justify,
                layout_context,
            );
        }
        Props::Spacer {
            horizontal,
            vertical,
        } => {
            handle.widget().set_hexpand(*horizontal);
            handle.widget().set_vexpand(*vertical);
        }
        Props::Scroll { axis } => configure_scroll(&downcast(handle)?, *axis),
        Props::Pattern {
            pattern: pattern @ (UiPattern::NavigationSplit { .. } | UiPattern::UtilitySplit { .. }),
        } => {
            let split = downcast::<adw::OverlaySplitView>(handle)?;
            *handle.0.pattern.borrow_mut() = Some(*pattern);
            let region = pattern
                .auxiliary_region()
                .expect("two-region pattern must declare one auxiliary region");
            let collapsible = pattern.region_is_collapsible(region);
            if !collapsible {
                split.set_collapsed(false);
            }
            split.set_enable_show_gesture(collapsible);
            split.set_enable_hide_gesture(collapsible);
        }
        Props::Pattern {
            pattern:
                pattern @ UiPattern::NavigationWorkspace {
                    sidebar_collapsible,
                    inspector_collapsible,
                },
        } => {
            let workspace = handle
                .0
                .workspace
                .as_ref()
                .ok_or_else(|| GtkError("workspace has no native split views".to_owned()))?;
            *handle.0.pattern.borrow_mut() = Some(*pattern);
            workspace
                .navigation
                .set_enable_show_gesture(*sidebar_collapsible);
            workspace
                .navigation
                .set_enable_hide_gesture(*sidebar_collapsible);
            workspace
                .inspector
                .set_enable_show_gesture(*inspector_collapsible);
            workspace
                .inspector
                .set_enable_hide_gesture(*inspector_collapsible);
            if !*sidebar_collapsible {
                workspace.navigation.set_collapsed(false);
            }
            if !*inspector_collapsible {
                workspace.inspector.set_collapsed(false);
            }
        }
        Props::List {
            accessibility_label,
            pattern,
            columns,
        } => {
            let list = handle
                .0
                .list
                .as_ref()
                .ok_or_else(|| GtkError("list has no native model".to_owned()))?;
            list.update(accessibility_label, *pattern, columns);
        }
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
        } => {
            let row = handle
                .0
                .row
                .as_ref()
                .ok_or_else(|| GtkError("list row has no native model item".to_owned()))?;
            row.update(
                title,
                subtitle.as_deref(),
                cells,
                *role,
                *expanded,
                *symbol,
                *selected,
                *disclosure,
                accessibility_label,
            );
        }
        Props::Status {
            title,
            message,
            tone,
        } => {
            let page = downcast::<adw::StatusPage>(handle)?;
            page.set_title(title);
            page.set_description(Some(message));
            page.set_icon_name(Some(status_icon(*tone)));
        }
        Props::Canvas { .. } => {
            return Err(GtkError(
                "GTK adapter does not implement the owned-drawing canvas element yet".to_owned(),
            ));
        }
        Props::TextArea { .. } => {
            return Err(GtkError(
                "the GTK host does not yet realize the multi-line text area".to_owned(),
            ));
        }
        Props::Dock { .. } => {
            return Err(GtkError(
                "the GTK host does not yet realize the tabbed-document dock".to_owned(),
            ));
        }
    }
    Ok(())
}
