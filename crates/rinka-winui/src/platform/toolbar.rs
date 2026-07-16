fn project_toolbar(
    window: &WindowSpec,
    inspector_collapsible: bool,
    set_inspector_open: ui::SetState<bool>,
    inspector_open: bool,
) -> ToolbarProjection {
    let mut back = None;
    let mut search = None;
    let mut commands = Vec::new();
    let mut secondary_commands = Vec::new();
    let mut supplementary_controls = Vec::new();
    let mut targets = Vec::new();

    for item in &window.toolbar {
        match &item.kind {
            ToolbarItemKind::Search {
                value,
                placeholder,
                accessibility_label,
                on_input,
            } => {
                let events = EventBindings::input(on_input.clone());
                let input_events = events.clone();
                let native_search: ui::Element = ui::auto_suggest_box(value)
                    .placeholder_text(placeholder)
                    .enabled(item.enabled)
                    .on_text_changed(move |value: String| input_events.emit_input(value))
                    .into();
                search = Some(
                    native_search
                        .automation_name(accessibility_label)
                        .help_text(&item.help)
                        .with_key(format!("toolbar-search-{}", item.id)),
                );
            }
            ToolbarItemKind::ActionGroup { actions } => {
                for action in actions {
                    if action.symbol == CommonSymbol::Back && back.is_none() {
                        if action.enabled {
                            back = Some(EventBindings::activate(action.on_activate.clone()));
                        }
                        continue;
                    }
                    push_action(
                        action,
                        &mut commands,
                        &mut supplementary_controls,
                        &mut targets,
                    );
                }
            }
            ToolbarItemKind::Action {
                symbol,
                on_activate,
            } => {
                let action = ToolbarAction {
                    id: item.id.clone(),
                    label: item.label.clone(),
                    symbol: *symbol,
                    help: item.help.clone(),
                    enabled: item.enabled,
                    on_activate: on_activate.clone(),
                };
                push_action(
                    &action,
                    &mut commands,
                    &mut supplementary_controls,
                    &mut targets,
                );
            }
            ToolbarItemKind::SelectionGroup {
                choices,
                selected_id: _,
                on_select,
            } => {
                let binding = EventBindings::input(on_select.clone());
                for choice in choices.iter().filter(|choice| choice.enabled) {
                    commands.push(ui::app_bar_button_icon(
                        &choice.label,
                        native_symbol(choice.symbol),
                    ));
                    targets.push((
                        choice.label.clone(),
                        ToolbarTarget::Input(binding.clone(), choice.id.clone()),
                    ));
                }
            }
            ToolbarItemKind::Menu { symbol: _, entries } => {
                push_menu_entries(
                    entries,
                    item.enabled,
                    &mut secondary_commands,
                    &mut supplementary_controls,
                    &mut targets,
                );
            }
        }
    }

    if inspector_collapsible {
        commands.push(ui::app_bar_separator());
        commands.push(ui::app_bar_button_icon("Details", ui::Symbol::ContactInfo));
        targets.push((
            "Details".to_owned(),
            ToolbarTarget::Inspector(set_inspector_open, !inspector_open),
        ));
    }
    ToolbarProjection {
        back,
        search,
        commands,
        secondary_commands,
        supplementary_controls,
        targets,
    }
}

/// Flattens the shared menu vocabulary into app-bar secondary commands.
///
/// This adapter's declared strategy projects toolbar menus into the command
/// bar's secondary command list. Submenu entries are flattened in order and
/// framed by separators; `ancestors_enabled` folds the enabled state of the
/// owning item and every enclosing submenu into each command. A checkmark has
/// no app-bar representation yet and a symbol-less item falls back to the
/// generic More glyph; both limits are recorded in reports/context-menus.
fn push_menu_entries(
    entries: &[MenuEntry],
    ancestors_enabled: bool,
    secondary_commands: &mut Vec<ui::CommandBarCommandDef>,
    supplementary_controls: &mut Vec<ui::Element>,
    targets: &mut Vec<(String, ToolbarTarget)>,
) {
    for entry in entries {
        match entry {
            MenuEntry::Item(item) => {
                let action = ToolbarAction {
                    id: item.id.clone(),
                    label: item.label.clone(),
                    symbol: item.symbol.unwrap_or(CommonSymbol::More),
                    help: item.help.clone(),
                    enabled: ancestors_enabled && item.enabled,
                    on_activate: item.on_activate.clone(),
                };
                push_action(
                    &action,
                    secondary_commands,
                    supplementary_controls,
                    targets,
                );
            }
            MenuEntry::Separator => {
                secondary_commands.push(ui::app_bar_separator());
            }
            MenuEntry::Submenu(submenu) => {
                secondary_commands.push(ui::app_bar_separator());
                push_menu_entries(
                    &submenu.entries,
                    ancestors_enabled && submenu.enabled,
                    secondary_commands,
                    supplementary_controls,
                    targets,
                );
                secondary_commands.push(ui::app_bar_separator());
            }
        }
    }
}

fn push_action(
    action: &ToolbarAction,
    commands: &mut Vec<ui::CommandBarCommandDef>,
    supplementary_controls: &mut Vec<ui::Element>,
    targets: &mut Vec<(String, ToolbarTarget)>,
) {
    if !action.enabled {
        supplementary_controls.push(
            ui::button("")
                .icon(native_symbol(action.symbol))
                .subtle()
                .enabled(false)
                .automation_name(&action.label)
                .help_text(&action.help)
                .with_key(format!("toolbar-disabled-{}", action.id))
                .into(),
        );
        return;
    }
    commands.push(ui::app_bar_button_icon(
        &action.label,
        native_symbol(action.symbol),
    ));
    targets.push((
        action.label.clone(),
        ToolbarTarget::Activate(EventBindings::activate(action.on_activate.clone())),
    ));
}

fn render_command_bar(
    commands: Vec<ui::CommandBarCommandDef>,
    secondary_commands: Vec<ui::CommandBarCommandDef>,
    supplementary_controls: Vec<ui::Element>,
    targets: Vec<(String, ToolbarTarget)>,
    key: u64,
) -> Option<ui::Element> {
    if commands.is_empty() && supplementary_controls.is_empty() {
        return None;
    }
    let targets = Rc::new(targets);
    let callback_targets = Rc::clone(&targets);
    let command_bar: ui::Element = ui::command_bar(commands)
        .secondary_commands(secondary_commands)
        .on_click(move |label: String| {
            if let Some((_, target)) = callback_targets.iter().find(|(key, _)| key == &label) {
                match target {
                    ToolbarTarget::Activate(binding) => binding.emit_activate(),
                    ToolbarTarget::Input(binding, value) => binding.emit_input(value),
                    ToolbarTarget::Inspector(setter, value) => setter.call(*value),
                }
            }
        })
        .horizontal_alignment(ui::HorizontalAlignment::Left)
        .with_key(format!("commandbar-{key}"))
        .into();
    if supplementary_controls.is_empty() {
        return Some(command_bar);
    }
    Some(
        ui::grid((
            ui::hstack(supplementary_controls)
                .spacing(4.0)
                .vertical_alignment(ui::VerticalAlignment::Center)
                .with_key(format!("supplementary-toolbar-{key}"))
                .grid_column(0),
            command_bar.grid_column(1),
        ))
        .columns([ui::GridLength::Auto, ui::GridLength::Star(1.0)])
        .horizontal_alignment(ui::HorizontalAlignment::Stretch)
        .with_key(format!("toolbar-row-{key}"))
        .into(),
    )
}

fn project_navigation(
    sidebar: &MountedNode<ProjectedHandle>,
) -> (
    Vec<ui::NavViewItem>,
    Vec<(String, NavigationTarget)>,
    Option<String>,
) {
    let list = sidebar.children().first().unwrap_or(sidebar);
    let mut items = Vec::new();
    let mut targets = Vec::new();
    let mut selected = None;
    for row in list.children() {
        let Props::ListRow {
            title,
            role,
            expanded,
            symbol,
            selected: row_selected,
            ..
        } = row.element().props()
        else {
            continue;
        };
        let tag = row.handle().value().to_string();
        let mut item = ui::NavViewItem::new(title).tag(&tag);
        if let Some(symbol) = symbol {
            item = item.icon(native_symbol(*symbol));
        }
        if *role == ListRowRole::Section {
            targets.push((
                tag,
                NavigationTarget::Toggle(row.events().clone(), !expanded),
            ));
            items.push(item);
            if *expanded {
                for child in row.children() {
                    items.push(navigation_item(child, &mut targets, &mut selected));
                }
            }
            continue;
        } else {
            if *row_selected {
                selected = Some(tag.clone());
            }
            targets.push((tag, NavigationTarget::Activate(row.events().clone())));
        }
        items.push(item);
    }
    (items, targets, selected)
}

fn navigation_item(
    row: &MountedNode<ProjectedHandle>,
    targets: &mut Vec<(String, NavigationTarget)>,
    selected: &mut Option<String>,
) -> ui::NavViewItem {
    let Props::ListRow {
        title,
        symbol,
        selected: row_selected,
        ..
    } = row.element().props()
    else {
        return ui::NavViewItem::new("");
    };
    let tag = row.handle().value().to_string();
    if *row_selected {
        *selected = Some(tag.clone());
    }
    targets.push((
        tag.clone(),
        NavigationTarget::Activate(row.events().clone()),
    ));
    let mut item = ui::NavViewItem::new(title).tag(tag);
    if let Some(symbol) = symbol {
        item = item.icon(native_symbol(*symbol));
    }
    item
}
