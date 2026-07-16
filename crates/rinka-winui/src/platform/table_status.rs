fn table_rows(node: &MountedNode<ProjectedHandle>) -> Vec<TableRowModel> {
    let mut rows = Vec::new();
    for child in node.children() {
        append_table_row(child, 0, &mut rows);
    }
    rows
}

fn append_table_row(
    node: &MountedNode<ProjectedHandle>,
    depth: usize,
    rows: &mut Vec<TableRowModel>,
) {
    let Props::ListRow {
        title,
        cells,
        selected,
        expanded,
        ..
    } = node.element().props()
    else {
        return;
    };
    rows.push(TableRowModel {
        key: node.handle().value().to_string(),
        title: format!("{}{title}", "    ".repeat(depth)),
        cells: cells.clone(),
        selected: *selected,
        events: node.events().clone(),
    });
    if *expanded {
        for child in node.children() {
            append_table_row(child, depth + 1, rows);
        }
    }
}

fn render_status(title: &str, message: &str, tone: StatusTone, key: String) -> ui::Element {
    let mut heading = ui::subtitle(title);
    heading = match tone {
        StatusTone::Error => heading.foreground(ui::ThemeRef::SystemCritical),
        StatusTone::Busy => heading.foreground(ui::ThemeRef::Accent),
        StatusTone::Empty | StatusTone::Informational => heading,
    };
    ui::vstack((heading, ui::body(message).wrap()))
        .spacing(8.0)
        .max_width(480.0)
        .horizontal_alignment(ui::HorizontalAlignment::Center)
        .vertical_alignment(ui::VerticalAlignment::Center)
        .automation_name(format!("{title}. {message}"))
        .with_key(key)
        .into()
}

fn render_separator(axis: Axis, key: String) -> ui::Element {
    let separator = ui::grid(())
        .background(ui::ThemeRef::DividerStroke)
        .with_key(key);
    match axis {
        Axis::Horizontal => separator
            .height(1.0)
            .horizontal_alignment(ui::HorizontalAlignment::Stretch)
            .into(),
        Axis::Vertical => separator
            .width(1.0)
            .vertical_alignment(ui::VerticalAlignment::Stretch)
            .into(),
    }
}

fn spacing_value(spacing: Spacing) -> f64 {
    match spacing {
        Spacing::Joined => 0.0,
        Spacing::Compact => 4.0,
        Spacing::Related => 8.0,
        Spacing::Section => 16.0,
        Spacing::Content => 24.0,
    }
}

fn control_height(size: ControlSize) -> f64 {
    match size {
        ControlSize::Mini => 24.0,
        ControlSize::Small => 28.0,
        ControlSize::Regular => 32.0,
        ControlSize::Large => 40.0,
        ControlSize::ExtraLarge => 48.0,
    }
}

fn native_symbol(symbol: CommonSymbol) -> ui::Symbol {
    match symbol {
        CommonSymbol::Back => ui::Symbol::Back,
        CommonSymbol::Forward | CommonSymbol::Disclosure => ui::Symbol::Forward,
        CommonSymbol::Add => ui::Symbol::Add,
        CommonSymbol::Refresh => ui::Symbol::Refresh,
        CommonSymbol::Search => ui::Symbol::Find,
        CommonSymbol::Home => ui::Symbol::Home,
        CommonSymbol::Folder => ui::Symbol::Folder,
        CommonSymbol::File => ui::Symbol::Document,
        CommonSymbol::Code => ui::Symbol::Page,
        CommonSymbol::Image | CommonSymbol::Gallery => ui::Symbol::Pictures,
        CommonSymbol::Terminal => ui::Symbol::Remote,
        CommonSymbol::Settings => ui::Symbol::Setting,
        CommonSymbol::More => ui::Symbol::More,
        CommonSymbol::Grid => ui::Symbol::ViewAll,
        CommonSymbol::List => ui::Symbol::List,
        CommonSymbol::Columns => ui::Symbol::DockLeft,
        CommonSymbol::Sort => ui::Symbol::Sort,
        CommonSymbol::Share => ui::Symbol::Share,
        CommonSymbol::Tag => ui::Symbol::Tag,
        CommonSymbol::Warning => ui::Symbol::Important,
    }
}

fn render_projection_error(message: &str) -> ui::Element {
    ui::vstack((
        ui::subtitle("Unable to render this window").foreground(ui::ThemeRef::SystemCritical),
        ui::body(message).wrap(),
    ))
    .spacing(8.0)
    .margin(ui::Thickness::uniform(24.0))
    .horizontal_alignment(ui::HorizontalAlignment::Center)
    .vertical_alignment(ui::VerticalAlignment::Center)
    .automation_name(format!("Unable to render this window. {message}"))
    .with_key("projection-error")
    .into()
}
