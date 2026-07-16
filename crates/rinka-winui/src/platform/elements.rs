fn render_node(node: &MountedNode<ProjectedHandle>) -> ui::Element {
    let key = node.handle().value().to_string();
    match node.element().props() {
        Props::Label {
            text,
            role,
            selectable,
        } => render_label(text, *role, *selectable, key),
        Props::Button {
            label,
            role,
            size,
            enabled,
            tooltip,
            accessibility_label,
            ..
        } => {
            let events = node.events().clone();
            let mut button = ui::button(label)
                .enabled(*enabled)
                .on_click(move || events.emit_activate())
                .automation_name(accessibility_label)
                .min_height(control_height(*size))
                .with_key(key);
            if let Some(tooltip) = tooltip {
                button = button.tooltip(tooltip);
            }
            button = match role {
                ButtonRole::Primary => button.accent(),
                ButtonRole::Toolbar => button.subtle(),
                ButtonRole::Standard | ButtonRole::Destructive => button,
            };
            button.into()
        }
        Props::Input {
            value,
            placeholder,
            kind,
            enabled,
            accessibility_label,
        } => render_input(
            value,
            placeholder,
            *kind,
            *enabled,
            accessibility_label,
            node.events().clone(),
            key,
        ),
        Props::Toggle {
            label,
            value,
            enabled,
            accessibility_label,
            ..
        } => {
            let events = node.events().clone();
            ui::ToggleSwitch::new(*value)
                .header(label)
                .enabled(*enabled)
                .on_toggled(move |value: bool| events.emit_toggle(value))
                .automation_name(accessibility_label)
                .with_key(key)
                .into()
        }
        Props::Progress {
            fraction,
            accessibility_label,
        } => ui::vstack((
            ui::ProgressBar::new(fraction * 100.0)
                .range(0.0, 100.0)
                .automation_name(accessibility_label)
                .horizontal_alignment(ui::HorizontalAlignment::Stretch)
                .with_key(format!("{key}-bar")),
            ui::caption(format!("{:.0}%", fraction * 100.0))
                .foreground(ui::ThemeRef::SecondaryText)
                .horizontal_alignment(ui::HorizontalAlignment::Right)
                .with_key(format!("{key}-percentage")),
        ))
        .spacing(4.0)
        .horizontal_alignment(ui::HorizontalAlignment::Stretch)
        .with_key(key)
        .into(),
        Props::Separator { axis } => render_separator(*axis, key),
        Props::Spacer {
            horizontal,
            vertical,
        } => {
            let mut spacer = ui::grid(()).with_key(key);
            if *horizontal {
                spacer = spacer.min_width(1.0);
            }
            if *vertical {
                spacer = spacer.min_height(1.0);
            }
            spacer.into()
        }
        Props::Stack {
            axis,
            spacing,
            padding,
            align,
            justify,
        } => render_stack(node, *axis, *spacing, *padding, *align, *justify, key),
        Props::Scroll { axis } => {
            let child = node
                .children()
                .first()
                .map(render_node)
                .unwrap_or(ui::Element::Empty);
            let orientation = match axis {
                Axis::Horizontal => ui::ScrollViewContentOrientation::Horizontal,
                Axis::Vertical => ui::ScrollViewContentOrientation::Vertical,
            };
            ui::scroll_view(child)
                .content_orientation(orientation)
                .horizontal_alignment(ui::HorizontalAlignment::Stretch)
                .vertical_alignment(ui::VerticalAlignment::Stretch)
                .with_key(key)
                .into()
        }
        Props::Pattern {
            pattern: pattern @ (UiPattern::NavigationSplit { .. } | UiPattern::UtilitySplit { .. }),
        } => render_split(node, *pattern, key),
        Props::Pattern {
            pattern: UiPattern::NavigationWorkspace { .. },
        } => ui::vstack(node.children().iter().map(render_node).collect::<Vec<_>>())
            .with_key(key)
            .into(),
        Props::List {
            pattern,
            columns,
            accessibility_label,
        } => match pattern {
            CollectionPattern::DataTable => {
                render_table(node, columns, accessibility_label, key)
            }
            CollectionPattern::NavigationSidebar
            | CollectionPattern::Outline
            | CollectionPattern::ContentList
            | CollectionPattern::EmbeddedList => {
                render_plain_list(node, accessibility_label, key)
            }
        },
        Props::ListRow { title, .. } => ui::text_block(title).with_key(key).into(),
        Props::Status {
            title,
            message,
            tone,
        } => render_status(title, message, *tone, key),
    }
}

fn render_label(text: &str, role: TextRole, selectable: bool, key: String) -> ui::Element {
    let mut label = match role {
        TextRole::Title => ui::title(text),
        TextRole::Heading => ui::subtitle(text),
        TextRole::Body => ui::body(text),
        TextRole::Secondary => ui::caption(text).foreground(ui::ThemeRef::SecondaryText),
        TextRole::Monospace => ui::body(text).font_family("Cascadia Mono"),
    };
    if selectable {
        label = label.selectable();
    }
    label.with_key(key).into()
}

#[allow(clippy::too_many_arguments)]
fn render_input(
    value: &str,
    placeholder: &str,
    kind: InputKind,
    enabled: bool,
    accessibility_label: &str,
    events: EventBindings,
    key: String,
) -> ui::Element {
    match kind {
        InputKind::Text => ui::text_box(value)
            .placeholder_text(placeholder)
            .enabled(enabled)
            .on_text_changed(move |value: String| events.emit_input(value))
            .automation_name(accessibility_label)
            .with_key(key)
            .into(),
        InputKind::Search => {
            let native_input: ui::Element = ui::auto_suggest_box(value)
                .placeholder_text(placeholder)
                .enabled(enabled)
                .on_text_changed(move |value: String| events.emit_input(value))
                .into();
            native_input
                .automation_name(accessibility_label)
                .with_key(key)
        }
        InputKind::Secure => ui::PasswordBox::new()
            .value(value)
            .placeholder_text(placeholder)
            .enabled(enabled)
            .on_password_changed(move |value: String| events.emit_input(value))
            .automation_name(accessibility_label)
            .with_key(key)
            .into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_stack(
    node: &MountedNode<ProjectedHandle>,
    axis: Axis,
    spacing: Spacing,
    padding: Option<Spacing>,
    align: Align,
    justify: Justify,
    key: String,
) -> ui::Element {
    let children = node.children().iter().map(render_node).collect::<Vec<_>>();
    let mut stack = match axis {
        Axis::Horizontal => ui::hstack(children),
        Axis::Vertical => ui::vstack(children),
    }
    .spacing(spacing_value(spacing))
    .with_key(key);
    if let Some(padding) = padding {
        stack = stack.padding(ui::Thickness::uniform(spacing_value(padding)));
    }
    stack = match (axis, align) {
        (Axis::Horizontal, Align::Start) => stack.vertical_alignment(ui::VerticalAlignment::Top),
        (Axis::Horizontal, Align::Center) => {
            stack.vertical_alignment(ui::VerticalAlignment::Center)
        }
        (Axis::Horizontal, Align::End) => stack.vertical_alignment(ui::VerticalAlignment::Bottom),
        (Axis::Horizontal, Align::Stretch) => {
            stack.vertical_alignment(ui::VerticalAlignment::Stretch)
        }
        (Axis::Vertical, Align::Start) => stack.horizontal_alignment(ui::HorizontalAlignment::Left),
        (Axis::Vertical, Align::Center) => {
            stack.horizontal_alignment(ui::HorizontalAlignment::Center)
        }
        (Axis::Vertical, Align::End) => stack.horizontal_alignment(ui::HorizontalAlignment::Right),
        (Axis::Vertical, Align::Stretch) => {
            stack.horizontal_alignment(ui::HorizontalAlignment::Stretch)
        }
    };
    stack = match (axis, justify) {
        (Axis::Horizontal, Justify::Center) => {
            stack.horizontal_alignment(ui::HorizontalAlignment::Center)
        }
        (Axis::Horizontal, Justify::End) => {
            stack.horizontal_alignment(ui::HorizontalAlignment::Right)
        }
        (Axis::Vertical, Justify::Center) => {
            stack.vertical_alignment(ui::VerticalAlignment::Center)
        }
        (Axis::Vertical, Justify::End) => stack.vertical_alignment(ui::VerticalAlignment::Bottom),
        _ => stack,
    };
    stack.into()
}

fn render_split(
    node: &MountedNode<ProjectedHandle>,
    pattern: UiPattern,
    key: String,
) -> ui::Element {
    let leading = node
        .children()
        .first()
        .map(render_node)
        .unwrap_or(ui::Element::Empty)
        .grid_column(0);
    let trailing = node
        .children()
        .get(1)
        .map(render_node)
        .unwrap_or(ui::Element::Empty)
        .grid_column(1);
    let columns = match pattern {
        UiPattern::NavigationSplit { .. } => {
            [ui::GridLength::Pixel(280.0), ui::GridLength::Star(1.0)]
        }
        UiPattern::UtilitySplit { .. } => [ui::GridLength::Star(1.0), ui::GridLength::Pixel(280.0)],
        UiPattern::NavigationWorkspace { .. } => unreachable!(),
    };
    ui::grid((leading, trailing))
        .columns(columns)
        .horizontal_alignment(ui::HorizontalAlignment::Stretch)
        .vertical_alignment(ui::VerticalAlignment::Stretch)
        .with_key(key)
        .into()
}

fn render_plain_list(
    node: &MountedNode<ProjectedHandle>,
    accessibility_label: &str,
    key: String,
) -> ui::Element {
    let rows = table_rows(node);
    let selection_events = rows
        .iter()
        .map(|row| row.events.clone())
        .collect::<Vec<_>>();
    let selected = rows
        .iter()
        .position(|row| row.selected)
        .map_or(-1, |index| index as i32);
    let list: ui::Element = ui::list_view(rows, |row, _| {
        ui::text_block(&row.title)
            .padding(ui::Thickness::xy(12.0, 8.0))
            .automation_name(&row.title)
            .with_key(&row.key)
    })
    .with_key_selector(|row| row.key.clone())
    .selected_index(selected)
    .on_selection_changed(move |index: i32| {
        if let Some(events) = usize::try_from(index)
            .ok()
            .and_then(|index| selection_events.get(index))
        {
            events.emit_activate();
        }
    })
    .with_key(key)
    .into();
    list.automation_name(accessibility_label)
}

fn render_table(
    node: &MountedNode<ProjectedHandle>,
    columns: &[TableColumn],
    accessibility_label: &str,
    key: String,
) -> ui::Element {
    let header = render_table_header(node, columns);
    let rows = table_rows(node);
    let selection_events = rows
        .iter()
        .map(|row| row.events.clone())
        .collect::<Vec<_>>();
    let selected = rows
        .iter()
        .position(|row| row.selected)
        .map_or(-1, |index| index as i32);
    let column_count = columns.len();
    let list: ui::Element = ui::list_view(rows, move |row, _| render_table_row(row, column_count))
        .with_key_selector(|row| row.key.clone())
        .selected_index(selected)
        .on_selection_changed(move |index: i32| {
            if let Some(events) = usize::try_from(index)
                .ok()
                .and_then(|index| selection_events.get(index))
            {
                events.emit_activate();
            }
        })
        .with_key(format!("table-list-{key}"))
        .into();
    let list = list.automation_name(accessibility_label);

    ui::grid((header.grid_row(0), list.grid_row(1)))
        .rows([ui::GridLength::Auto, ui::GridLength::Star(1.0)])
        .horizontal_alignment(ui::HorizontalAlignment::Stretch)
        .vertical_alignment(ui::VerticalAlignment::Stretch)
        .with_key(format!("table-{key}"))
        .into()
}

fn render_table_header(
    node: &MountedNode<ProjectedHandle>,
    columns: &[TableColumn],
) -> ui::Element {
    let mut children = Vec::with_capacity(columns.len());
    for (index, column) in columns.iter().enumerate() {
        let binding = node.events().clone();
        let column_id = column.id.clone();
        let next_direction = match column.sort_direction {
            Some(SortDirection::Ascending) => SortDirection::Descending,
            Some(SortDirection::Descending) | None => SortDirection::Ascending,
        };
        let mut label = ui::text_block(&column.title)
            .semibold()
            .grid_column(index as i32)
            .with_key(format!("header-{}-{}", node.handle().value(), column.id));
        if column.sortable || column.sort_direction.is_some() {
            label = label
                .automation_name(format!("Sort by {}", column.title))
                .on_tapped(move || {
                    binding.emit_sort(TableSort {
                        column_id: column_id.clone(),
                        direction: next_direction,
                    });
                });
        }
        children.push(label.into());
    }
    ui::grid(children)
        .columns(table_grid_columns(columns.len()))
        .margin(ui::Thickness {
            left: 16.0,
            top: 0.0,
            right: 12.0,
            bottom: 0.0,
        })
        .horizontal_alignment(ui::HorizontalAlignment::Stretch)
        .with_key(format!("table-header-{}", node.handle().value()))
        .into()
}

fn render_table_row(row: &TableRowModel, column_count: usize) -> ui::Element {
    let mut children = Vec::with_capacity(column_count);
    children.push(
        ui::text_block(&row.title)
            .vertical_alignment(ui::VerticalAlignment::Center)
            .grid_column(0)
            .with_key(format!("{}-name", row.key))
            .into(),
    );
    for (index, value) in row.cells.iter().enumerate() {
        children.push(
            ui::text_block(value)
                .vertical_alignment(ui::VerticalAlignment::Center)
                .grid_column((index + 1) as i32)
                .with_key(format!("{}-cell-{index}", row.key))
                .into(),
        );
    }
    ui::grid(children)
        .columns(table_grid_columns(column_count))
        .margin(ui::Thickness::xy(12.0, 8.0))
        .horizontal_alignment(ui::HorizontalAlignment::Stretch)
        .automation_name(&row.title)
        .with_key(&row.key)
        .into()
}

fn table_grid_columns(count: usize) -> Vec<ui::GridLength> {
    let mut columns = Vec::with_capacity(count.max(1));
    columns.push(ui::GridLength::Star(1.0));
    for index in 1..count {
        columns.push(match index {
            1 => ui::GridLength::Pixel(150.0),
            2 => ui::GridLength::Pixel(88.0),
            3 => ui::GridLength::Pixel(154.0),
            _ => ui::GridLength::Pixel(120.0),
        });
    }
    columns
}
