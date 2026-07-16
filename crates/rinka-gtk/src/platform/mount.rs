fn insert_child(parent: &GtkHandle, child: &GtkHandle, index: usize) -> Result<(), GtkError> {
    let mut presentations = parent.0.presentations.borrow_mut();
    if index > presentations.len() {
        return Err(GtkError(format!(
            "cannot insert GTK child at {index}; count is {}",
            presentations.len()
        )));
    }
    let presentation = Presentation {
        source: child.widget().clone(),
        view: child.widget().clone(),
    };
    match parent.0.host_kind {
        HostKind::Root => {
            presentation.view.set_hexpand(true);
            presentation.view.set_vexpand(true);
            let container = downcast::<gtk::Box>(parent)?;
            let sibling = index
                .checked_sub(1)
                .and_then(|previous| presentations.get(previous))
                .map(|item| &item.view);
            container.insert_child_after(&presentation.view, sibling);
        }
        HostKind::Element(ElementKind::Stack) => {
            let container = downcast::<gtk::Box>(parent)?;
            let sibling = index
                .checked_sub(1)
                .and_then(|previous| presentations.get(previous))
                .map(|item| &item.view);
            container.insert_child_after(&presentation.view, sibling);
        }
        HostKind::Element(ElementKind::List) => {
            let list = parent
                .0
                .list
                .as_ref()
                .ok_or_else(|| GtkError("list has no native model".to_owned()))?;
            let object = child
                .0
                .row_object
                .as_ref()
                .ok_or_else(|| GtkError("list accepts only native row items".to_owned()))?;
            list.store
                .insert(u32::try_from(index).unwrap_or(u32::MAX), object);
            if let Some(row) = child.0.row.as_ref() {
                row.attach_owner(list);
            }
            list.sync_selection();
        }
        HostKind::Element(ElementKind::ListRow) => {
            let row = parent
                .0
                .row
                .as_ref()
                .ok_or_else(|| GtkError("list row has no native model item".to_owned()))?;
            let object = child.0.row_object.as_ref().ok_or_else(|| {
                GtkError("list hierarchy accepts only native row items".to_owned())
            })?;
            row.children
                .insert(u32::try_from(index).unwrap_or(u32::MAX), object);
            if let Some(child_row) = child.0.row.as_ref() {
                for owner in row
                    .list_owners
                    .borrow()
                    .iter()
                    .filter_map(std::rc::Weak::upgrade)
                {
                    child_row.attach_owner(&owner);
                }
            }
            row.refresh();
        }
        HostKind::Element(ElementKind::Scroll) => {
            if index != 0 || !presentations.is_empty() {
                return Err(GtkError("scroll view accepts exactly one child".to_owned()));
            }
            downcast::<gtk::ScrolledWindow>(parent)?.set_child(Some(&presentation.view));
        }
        HostKind::Element(ElementKind::Pattern) => {
            presentation.view.set_hexpand(true);
            presentation.view.set_vexpand(true);
            let pattern = parent
                .0
                .pattern
                .borrow()
                .ok_or_else(|| GtkError("pattern host has no semantic pattern".to_owned()))?;
            if let Some(workspace) = parent.0.workspace.as_ref() {
                match index {
                    0 => workspace.navigation.set_sidebar(Some(&presentation.view)),
                    1 => workspace.inspector.set_content(Some(&presentation.view)),
                    2 => workspace.inspector.set_sidebar(Some(&presentation.view)),
                    _ => return Err(GtkError("workspace accepts three regions".to_owned())),
                }
            } else {
                let split = downcast::<adw::OverlaySplitView>(parent)?;
                match (pattern.regions().get(index), index) {
                    (Some(PatternRegion::NavigationSidebar | PatternRegion::Inspector), _) => {
                        split.set_sidebar(Some(&presentation.view));
                    }
                    (Some(PatternRegion::Content), _) => {
                        split.set_content(Some(&presentation.view));
                    }
                    _ => return Err(GtkError("split pattern accepts two regions".to_owned())),
                }
            }
        }
        HostKind::Element(kind) => {
            return Err(GtkError(format!("{kind:?} cannot contain children")));
        }
    }
    presentations.insert(index, presentation);
    Ok(())
}

fn remove_child(parent: &GtkHandle, child: &GtkHandle, index: usize) -> Result<(), GtkError> {
    let mut presentations = parent.0.presentations.borrow_mut();
    let Some(presentation) = presentations.get(index) else {
        return Err(GtkError(format!("no GTK child at index {index}")));
    };
    if presentation.source != *child.widget() {
        return Err(GtkError(format!("GTK child mismatch at index {index}")));
    }
    match parent.0.host_kind {
        HostKind::Root | HostKind::Element(ElementKind::Stack) => {
            downcast::<gtk::Box>(parent)?.remove(&presentation.view);
        }
        HostKind::Element(ElementKind::List) => {
            let list = parent
                .0
                .list
                .as_ref()
                .ok_or_else(|| GtkError("list has no native model".to_owned()))?;
            list.store.remove(u32::try_from(index).unwrap_or(u32::MAX));
            list.sync_selection();
        }
        HostKind::Element(ElementKind::ListRow) => {
            let row = parent
                .0
                .row
                .as_ref()
                .ok_or_else(|| GtkError("list row has no native model item".to_owned()))?;
            row.children
                .remove(u32::try_from(index).unwrap_or(u32::MAX));
            row.refresh();
        }
        HostKind::Element(ElementKind::Scroll) => {
            downcast::<gtk::ScrolledWindow>(parent)?.set_child(gtk::Widget::NONE);
        }
        HostKind::Element(ElementKind::Pattern) => {
            let pattern = parent
                .0
                .pattern
                .borrow()
                .ok_or_else(|| GtkError("pattern host has no semantic pattern".to_owned()))?;
            if let Some(workspace) = parent.0.workspace.as_ref() {
                match index {
                    0 => workspace.navigation.set_sidebar(gtk::Widget::NONE),
                    1 => workspace.inspector.set_content(gtk::Widget::NONE),
                    2 => workspace.inspector.set_sidebar(gtk::Widget::NONE),
                    _ => return Err(GtkError("workspace has no requested region".to_owned())),
                }
            } else {
                let split = downcast::<adw::OverlaySplitView>(parent)?;
                match pattern.regions().get(index) {
                    Some(PatternRegion::NavigationSidebar | PatternRegion::Inspector) => {
                        split.set_sidebar(gtk::Widget::NONE);
                    }
                    Some(PatternRegion::Content) => split.set_content(gtk::Widget::NONE),
                    None => return Err(GtkError("split has no requested region".to_owned())),
                }
            }
        }
        HostKind::Element(kind) => {
            return Err(GtkError(format!("{kind:?} cannot remove children")));
        }
    }
    presentations.remove(index);
    Ok(())
}

fn move_child(
    parent: &GtkHandle,
    child: &GtkHandle,
    from: usize,
    to: usize,
) -> Result<(), GtkError> {
    if from == to {
        return Ok(());
    }
    let mut presentations = parent.0.presentations.borrow_mut();
    if from >= presentations.len() || to >= presentations.len() {
        return Err(GtkError(format!(
            "cannot move GTK child from {from} to {to}; count is {}",
            presentations.len()
        )));
    }
    if presentations[from].source != *child.widget() {
        return Err(GtkError(format!("GTK child mismatch at index {from}")));
    }
    let moved = presentations.remove(from);
    presentations.insert(to, moved);
    match parent.0.host_kind {
        HostKind::Root | HostKind::Element(ElementKind::Stack) => {
            let container = downcast::<gtk::Box>(parent)?;
            let sibling = to
                .checked_sub(1)
                .and_then(|previous| presentations.get(previous))
                .map(|item| &item.view);
            container.reorder_child_after(&presentations[to].view, sibling);
        }
        HostKind::Element(ElementKind::List) => {
            let list = parent
                .0
                .list
                .as_ref()
                .ok_or_else(|| GtkError("list has no native model".to_owned()))?;
            move_model_item(&list.store, from, to)?;
            list.sync_selection();
        }
        HostKind::Element(ElementKind::ListRow) => {
            let row = parent
                .0
                .row
                .as_ref()
                .ok_or_else(|| GtkError("list row has no native model item".to_owned()))?;
            move_model_item(&row.children, from, to)?;
            row.refresh();
        }
        HostKind::Element(ElementKind::Pattern) => {
            if let Some(workspace) = parent.0.workspace.as_ref() {
                workspace
                    .navigation
                    .set_sidebar(Some(&presentations[0].view));
                workspace
                    .inspector
                    .set_content(Some(&presentations[1].view));
                workspace
                    .inspector
                    .set_sidebar(Some(&presentations[2].view));
            } else {
                let split = downcast::<adw::OverlaySplitView>(parent)?;
                let pattern =
                    parent.0.pattern.borrow().ok_or_else(|| {
                        GtkError("pattern host has no semantic pattern".to_owned())
                    })?;
                for (region, presentation) in pattern.regions().iter().zip(presentations.iter()) {
                    match region {
                        PatternRegion::NavigationSidebar | PatternRegion::Inspector => {
                            split.set_sidebar(Some(&presentation.view));
                        }
                        PatternRegion::Content => split.set_content(Some(&presentation.view)),
                    }
                }
            }
        }
        kind => return Err(GtkError(format!("{kind:?} does not support child moves"))),
    }
    Ok(())
}

fn move_model_item(store: &gio::ListStore, from: usize, to: usize) -> Result<(), GtkError> {
    let from = u32::try_from(from).unwrap_or(u32::MAX);
    let to = u32::try_from(to).unwrap_or(u32::MAX);
    let item = store
        .item(from)
        .ok_or_else(|| GtkError(format!("native row model has no item at {from}")))?;
    store.remove(from);
    store.insert(to, &item);
    Ok(())
}

fn downcast<T>(handle: &GtkHandle) -> Result<T, GtkError>
where
    T: IsA<gtk::Widget> + glib::types::StaticType,
{
    handle.widget().clone().downcast::<T>().map_err(|widget| {
        GtkError(format!(
            "expected {}, found {}",
            T::static_type().name(),
            widget.type_().name()
        ))
    })
}

fn native_progress(handle: &GtkHandle) -> Result<gtk::ProgressBar, GtkError> {
    handle
        .0
        .auxiliaries
        .first()
        .and_then(|widget| widget.clone().downcast::<gtk::ProgressBar>().ok())
        .ok_or_else(|| GtkError("progress has no native progress bar".to_owned()))
}

fn progress_percentage_text(fraction: f64) -> String {
    format!("{:.0}%", fraction * 100.0)
}

fn build_toolbar(
    spec: &WindowSpec,
    renderer: &Renderer<GtkBackend>,
    narrow_layout: Rc<Cell<bool>>,
) -> (adw::ToolbarView, adw::HeaderBar, Vec<gtk::Stack>) {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let mut adaptive_items = Vec::new();
    if let Some(split) = pane_for(renderer.mounted(), PatternRegion::NavigationSidebar) {
        let expanded = pane_toggle_button(
            "Sidebar",
            "sidebar-show-symbolic",
            spec.toolbar_display,
            "Show or hide the navigation sidebar",
            split.clone(),
            narrow_layout.clone(),
        );
        let compact = pane_toggle_button(
            "Navigation",
            "sidebar-show-symbolic",
            ToolbarDisplay::IconAndLabel,
            "Show or hide the navigation sidebar",
            split,
            narrow_layout.clone(),
        );
        let stack = gtk::Stack::new();
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        stack.add_named(&expanded, Some("expanded"));
        stack.add_named(&compact, Some("compact"));
        stack.set_visible_child_name("expanded");
        header.pack_start(&stack);
        adaptive_items.push(stack);
    }

    let center = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    center.add_css_class("linked");
    let mut has_center = false;
    for item in &spec.toolbar {
        let presentation = toolbar_widget(item, spec.toolbar_display);
        if let Some(stack) = presentation.adaptive_stack {
            adaptive_items.push(stack);
        }
        let widget = presentation.widget;
        match item.placement {
            ToolbarPlacement::Leading => header.pack_start(&widget),
            ToolbarPlacement::Center => {
                center.append(&widget);
                has_center = true;
            }
            ToolbarPlacement::Trailing => header.pack_end(&widget),
        }
    }
    if has_center {
        header.set_title_widget(Some(&center));
    }

    if let Some(split) = pane_for(renderer.mounted(), PatternRegion::Inspector) {
        let button = gtk::Button::new();
        button.set_child(Some(&toolbar_named_item_content(
            "Inspector",
            "sidebar-show-right-symbolic",
            spec.toolbar_display,
        )));
        button.set_tooltip_text(Some("Show or hide the inspector"));
        button.update_property(&[gtk::accessible::Property::Label("Inspector")]);
        let split = split.clone();
        button.connect_clicked(move |_| toggle_split_sidebar(&split, narrow_layout.get()));
        header.pack_end(&button);
    }
    toolbar.add_top_bar(&header);
    (toolbar, header, adaptive_items)
}

fn pane_toggle_button(
    label: &str,
    icon_name: &str,
    display: ToolbarDisplay,
    help: &str,
    split: adw::OverlaySplitView,
    narrow_layout: Rc<Cell<bool>>,
) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&toolbar_named_item_content(label, icon_name, display)));
    button.set_tooltip_text(Some(help));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button.connect_clicked(move |_| toggle_split_sidebar(&split, narrow_layout.get()));
    button
}

fn toggle_split_sidebar(split: &adw::OverlaySplitView, narrow_layout: bool) {
    if split.sidebar().is_none() {
        return;
    }
    if narrow_layout {
        split.set_show_sidebar(!split.shows_sidebar());
    } else if split.is_collapsed() {
        split.set_collapsed(false);
    } else {
        split.set_show_sidebar(false);
        split.set_collapsed(true);
    }
}

struct ToolbarPresentation {
    widget: gtk::Widget,
    adaptive_stack: Option<gtk::Stack>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolbarLayout {
    Expanded,
    Compact,
    Adaptive,
}

fn toolbar_layout(item: &ToolbarItem) -> ToolbarLayout {
    let has_compact_representation = matches!(
        &item.kind,
        ToolbarItemKind::ActionGroup { .. }
            | ToolbarItemKind::SelectionGroup { .. }
            | ToolbarItemKind::Search { .. }
    );
    if !has_compact_representation {
        return ToolbarLayout::Expanded;
    }
    match item.group_display {
        ToolbarGroupDisplay::Automatic => ToolbarLayout::Adaptive,
        ToolbarGroupDisplay::Expanded => ToolbarLayout::Expanded,
        ToolbarGroupDisplay::Collapsed => ToolbarLayout::Compact,
    }
}

fn toolbar_widget(item: &ToolbarItem, display: ToolbarDisplay) -> ToolbarPresentation {
    let expanded = toolbar_expanded_widget(item, display);
    match toolbar_layout(item) {
        ToolbarLayout::Expanded => ToolbarPresentation {
            widget: expanded,
            adaptive_stack: None,
        },
        ToolbarLayout::Compact => ToolbarPresentation {
            widget: toolbar_compact_widget(item, display)
                .expect("compact toolbar layout requires a compact representation"),
            adaptive_stack: None,
        },
        ToolbarLayout::Adaptive => {
            let compact = toolbar_compact_widget(item, display)
                .expect("adaptive toolbar layout requires a compact representation");
            let stack = gtk::Stack::new();
            stack.set_hhomogeneous(false);
            stack.set_vhomogeneous(false);
            stack.add_named(&expanded, Some("expanded"));
            stack.add_named(&compact, Some("compact"));
            stack.set_visible_child_name("expanded");
            ToolbarPresentation {
                widget: stack.clone().upcast(),
                adaptive_stack: Some(stack),
            }
        }
    }
}

fn toolbar_expanded_widget(item: &ToolbarItem, display: ToolbarDisplay) -> gtk::Widget {
    let widget: gtk::Widget = match &item.kind {
        ToolbarItemKind::Action {
            symbol,
            on_activate,
        } => toolbar_action_button(
            &item.label,
            &item.help,
            *symbol,
            display,
            item.enabled,
            on_activate.clone(),
        )
        .upcast(),
        ToolbarItemKind::ActionGroup { actions } => {
            let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            group.add_css_class("linked");
            group.update_property(&[gtk::accessible::Property::Label(&item.label)]);
            for action in actions {
                group.append(&toolbar_action_button(
                    &action.label,
                    &action.help,
                    action.symbol,
                    display,
                    item.enabled && action.enabled,
                    action.on_activate.clone(),
                ));
            }
            group.upcast()
        }
        ToolbarItemKind::SelectionGroup {
            choices,
            selected_id,
            on_select,
        } => {
            let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            group.add_css_class("linked");
            group.update_property(&[gtk::accessible::Property::Label(&item.label)]);
            let mut previous: Option<gtk::ToggleButton> = None;
            for choice in choices {
                let button = gtk::ToggleButton::new();
                button.set_child(Some(&toolbar_item_content(
                    &choice.label,
                    choice.symbol,
                    display,
                )));
                button.set_tooltip_text(Some(&choice.label));
                button.set_sensitive(item.enabled && choice.enabled);
                button.update_property(&[gtk::accessible::Property::Label(&choice.label)]);
                if let Some(previous) = previous.as_ref() {
                    button.set_group(Some(previous));
                }
                button.set_active(choice.id == *selected_id);
                let selected = choice.id.clone();
                let on_select = on_select.clone();
                button.connect_toggled(move |button| {
                    if button.is_active() {
                        on_select(selected.clone());
                    }
                });
                group.append(&button);
                previous = Some(button);
            }
            group.upcast()
        }
        ToolbarItemKind::Menu { symbol, entries } => {
            let button = gtk::MenuButton::new();
            button.set_child(Some(&toolbar_item_content(&item.label, *symbol, display)));
            button.set_tooltip_text(Some(&item.help));
            button.set_sensitive(item.enabled);
            button.update_property(&[gtk::accessible::Property::Label(&item.label)]);
            let prefix = native_action_name(&item.id);
            let actions = gio::SimpleActionGroup::new();
            let menu = build_menu_model(&prefix, entries, item.enabled, &actions);
            button.insert_action_group(&prefix, Some(&actions));
            button.set_menu_model(Some(&menu));
            button.upcast()
        }
        ToolbarItemKind::Search {
            value,
            placeholder,
            accessibility_label,
            on_input,
        } => {
            let search = gtk::SearchEntry::new();
            search.set_text(value);
            search.set_placeholder_text(Some(placeholder));
            search.set_sensitive(item.enabled);
            search.set_tooltip_text(Some(&item.help));
            search.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            let on_input = on_input.clone();
            search.connect_search_changed(move |search| on_input(search.text().to_string()));
            search.upcast()
        }
    };
    widget
}

/// Builds a `gio::Menu` model from the shared menu vocabulary.
///
/// Entries become detailed actions inside `actions` under `prefix`, separators
/// become section boundaries, and submenus become native nested menus.
/// `ancestors_enabled` folds the enabled state of the owning control and every
/// enclosing submenu into each action, matching the semantic contract that a
/// disabled ancestor also disables its entries. A checked item is backed by a
/// stateful boolean action so the native menu shows the platform checkmark;
/// the declarative model stays authoritative for that state.
fn build_menu_model(
    prefix: &str,
    entries: &[MenuEntry],
    ancestors_enabled: bool,
    actions: &gio::SimpleActionGroup,
) -> gio::Menu {
    let menu = gio::Menu::new();
    let mut section = gio::Menu::new();
    for entry in entries {
        match entry {
            MenuEntry::Item(item) => {
                let action_name = native_action_name(&item.id);
                let detailed_action = format!("{prefix}.{action_name}");
                let menu_item = gio::MenuItem::new(Some(&item.label), Some(&detailed_action));
                if let Some(symbol) = item.symbol {
                    let icon = gio::ThemedIcon::new(symbol_name(symbol));
                    menu_item.set_icon(&icon);
                }
                section.append_item(&menu_item);
                let native_action = if item.checked {
                    gio::SimpleAction::new_stateful(&action_name, None, &true.to_variant())
                } else {
                    gio::SimpleAction::new(&action_name, None)
                };
                native_action.set_enabled(ancestors_enabled && item.enabled);
                let handler = item.on_activate.clone();
                native_action.connect_activate(move |_, _| handler());
                actions.add_action(&native_action);
            }
            MenuEntry::Separator => {
                if section.n_items() > 0 {
                    menu.append_section(None, &section);
                    section = gio::Menu::new();
                }
            }
            MenuEntry::Submenu(submenu) => {
                let nested = build_menu_model(
                    prefix,
                    &submenu.entries,
                    ancestors_enabled && submenu.enabled,
                    actions,
                );
                section.append_submenu(Some(&submenu.label), &nested);
            }
        }
    }
    if section.n_items() > 0 {
        menu.append_section(None, &section);
    }
    menu
}

fn toolbar_compact_widget(item: &ToolbarItem, _display: ToolbarDisplay) -> Option<gtk::Widget> {
    match &item.kind {
        ToolbarItemKind::ActionGroup { actions }
            if action_group_uses_direct_compact_buttons(actions.len()) =>
        {
            Some(compact_action_group(item, actions).upcast())
        }
        ToolbarItemKind::ActionGroup { actions } => {
            Some(action_group_menu_button(item, actions).upcast())
        }
        ToolbarItemKind::SelectionGroup {
            choices,
            selected_id,
            on_select,
        } => Some(
            selection_group_menu_button(item, choices, selected_id, on_select.clone()).upcast(),
        ),
        ToolbarItemKind::Search {
            value,
            placeholder,
            accessibility_label,
            on_input,
        } => Some(
            compact_search_button(
                item,
                value,
                placeholder,
                accessibility_label,
                on_input.clone(),
            )
            .upcast(),
        ),
        ToolbarItemKind::Action { .. } | ToolbarItemKind::Menu { .. } => None,
    }
}

fn action_group_uses_direct_compact_buttons(action_count: usize) -> bool {
    (1..=2).contains(&action_count)
}

fn compact_action_group(item: &ToolbarItem, actions: &[ToolbarAction]) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    group.add_css_class("linked");
    group.update_property(&[gtk::accessible::Property::Label(&item.label)]);
    for action in actions {
        group.append(&toolbar_action_button(
            &action.label,
            &action.help,
            action.symbol,
            ToolbarDisplay::IconOnly,
            item.enabled && action.enabled,
            action.on_activate.clone(),
        ));
    }
    group
}

fn action_group_menu_button(item: &ToolbarItem, actions: &[ToolbarAction]) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_child(Some(&toolbar_named_item_content(
        &item.label,
        symbol_name(Symbol::More),
        ToolbarDisplay::IconOnly,
    )));
    button.set_tooltip_text(Some(&item.help));
    button.set_sensitive(item.enabled);
    button.update_property(&[gtk::accessible::Property::Label(&item.label)]);

    let prefix = native_action_name(&item.id);
    let action_group = gio::SimpleActionGroup::new();
    let menu = gio::Menu::new();
    for action in actions {
        let action_name = native_action_name(&action.id);
        let detailed_action = format!("{prefix}.{action_name}");
        let menu_item = gio::MenuItem::new(Some(&action.label), Some(&detailed_action));
        menu_item.set_icon(&gio::ThemedIcon::new(symbol_name(action.symbol)));
        menu.append_item(&menu_item);
        let native_action = gio::SimpleAction::new(&action_name, None);
        native_action.set_enabled(item.enabled && action.enabled);
        let handler = action.on_activate.clone();
        native_action.connect_activate(move |_, _| handler());
        action_group.add_action(&native_action);
    }
    button.insert_action_group(&prefix, Some(&action_group));
    button.set_menu_model(Some(&menu));
    button
}

fn selection_group_menu_button(
    item: &ToolbarItem,
    choices: &[rinka_core::ToolbarChoice],
    selected_id: &str,
    on_select: rinka_core::InputHandler,
) -> gtk::MenuButton {
    let selected_symbol = choices
        .iter()
        .find(|choice| choice.id == selected_id)
        .map_or(Symbol::More, |choice| choice.symbol);
    let button = gtk::MenuButton::new();
    button.set_child(Some(&toolbar_named_item_content(
        &item.label,
        symbol_name(selected_symbol),
        ToolbarDisplay::IconOnly,
    )));
    button.set_tooltip_text(Some(&item.help));
    button.set_sensitive(item.enabled);
    button.update_property(&[gtk::accessible::Property::Label(&item.label)]);

    let prefix = native_action_name(&item.id);
    let action_group = gio::SimpleActionGroup::new();
    let menu = gio::Menu::new();
    for choice in choices {
        let action_name = native_action_name(&choice.id);
        let detailed_action = format!("{prefix}.{action_name}");
        let menu_item = gio::MenuItem::new(Some(&choice.label), Some(&detailed_action));
        let icon_name = if choice.id == selected_id {
            "object-select-symbolic"
        } else {
            symbol_name(choice.symbol)
        };
        menu_item.set_icon(&gio::ThemedIcon::new(icon_name));
        menu.append_item(&menu_item);
        let native_action = gio::SimpleAction::new(&action_name, None);
        native_action.set_enabled(item.enabled && choice.enabled);
        let selected = choice.id.clone();
        let handler = on_select.clone();
        native_action.connect_activate(move |_, _| handler(selected.clone()));
        action_group.add_action(&native_action);
    }
    button.insert_action_group(&prefix, Some(&action_group));
    button.set_menu_model(Some(&menu));
    button
}

fn compact_search_button(
    item: &ToolbarItem,
    value: &str,
    placeholder: &str,
    accessibility_label: &str,
    on_input: rinka_core::InputHandler,
) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_child(Some(&toolbar_named_item_content(
        &item.label,
        symbol_name(Symbol::Search),
        ToolbarDisplay::IconOnly,
    )));
    button.set_tooltip_text(Some(&item.help));
    button.set_sensitive(item.enabled);
    button.update_property(&[gtk::accessible::Property::Label(&item.label)]);

    let search = gtk::SearchEntry::new();
    search.set_text(value);
    search.set_placeholder_text(Some(placeholder));
    search.set_width_chars(24);
    search.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
    search.connect_search_changed(move |search| on_input(search.text().to_string()));
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let inset = spacing_pixels(Spacing::Related);
    content.set_margin_start(inset);
    content.set_margin_end(inset);
    content.set_margin_top(inset);
    content.set_margin_bottom(inset);
    content.append(&search);
    let popover = gtk::Popover::new();
    popover.set_child(Some(&content));
    button.set_popover(Some(&popover));
    button
}

fn toolbar_action_button(
    label: &str,
    help: &str,
    symbol: Symbol,
    display: ToolbarDisplay,
    enabled: bool,
    action: rinka_core::ActivateHandler,
) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&toolbar_item_content(label, symbol, display)));
    button.set_tooltip_text(Some(help));
    button.set_sensitive(enabled);
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button.connect_clicked(move |_| action());
    button
}

fn toolbar_item_content(label: &str, symbol: Symbol, display: ToolbarDisplay) -> gtk::Widget {
    toolbar_named_item_content(label, symbol_name(symbol), display)
}

fn toolbar_named_item_content(
    label: &str,
    icon_name: &str,
    display: ToolbarDisplay,
) -> gtk::Widget {
    match display {
        ToolbarDisplay::Automatic | ToolbarDisplay::IconOnly => {
            gtk::Image::from_icon_name(icon_name).upcast()
        }
        ToolbarDisplay::IconAndLabel => {
            let content = gtk::Box::new(
                gtk::Orientation::Horizontal,
                spacing_pixels(Spacing::Compact),
            );
            content.append(&gtk::Image::from_icon_name(icon_name));
            content.append(&gtk::Label::new(Some(label)));
            content.upcast()
        }
        ToolbarDisplay::LabelOnly => gtk::Label::new(Some(label)).upcast(),
    }
}

fn native_action_name(identifier: &str) -> String {
    identifier
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn pane_for(
    mounted: Option<&MountedNode<GtkHandle>>,
    region: PatternRegion,
) -> Option<adw::OverlaySplitView> {
    let node = mounted?;
    if let Some(workspace) = node.handle().0.workspace.as_ref() {
        return Some(match region {
            PatternRegion::NavigationSidebar => workspace.navigation.clone(),
            PatternRegion::Inspector => workspace.inspector.clone(),
            PatternRegion::Content => return None,
        });
    }
    if node
        .handle()
        .0
        .pattern
        .borrow()
        .is_some_and(|pattern| pattern.regions().contains(&region))
        && let Ok(split) = node
            .handle()
            .widget()
            .clone()
            .downcast::<adw::OverlaySplitView>()
    {
        return Some(split);
    }
    node.children()
        .iter()
        .find_map(|child| pane_for(Some(child), region))
}

fn collect_adaptive_splits(
    mounted: Option<&MountedNode<GtkHandle>>,
    output: &mut Vec<(adw::OverlaySplitView, bool)>,
) {
    let Some(node) = mounted else {
        return;
    };
    if let Some(workspace) = node.handle().0.workspace.as_ref() {
        let pattern = node.handle().0.pattern.borrow();
        let pattern = pattern.expect("workspace handle must retain its pattern");
        output.push((
            workspace.navigation.clone(),
            pattern.region_is_collapsible(PatternRegion::NavigationSidebar),
        ));
        output.push((
            workspace.inspector.clone(),
            pattern.region_is_collapsible(PatternRegion::Inspector),
        ));
    } else if let Some(pattern) = *node.handle().0.pattern.borrow()
        && let Ok(split) = node
            .handle()
            .widget()
            .clone()
            .downcast::<adw::OverlaySplitView>()
    {
        let Some(region) = pattern.auxiliary_region() else {
            return;
        };
        output.push((split, pattern.region_is_collapsible(region)));
    }
    for child in node.children() {
        collect_adaptive_splits(Some(child), output);
    }
}
