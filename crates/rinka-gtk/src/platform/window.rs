struct BuiltWindow {
    window: gtk::Window,
    _runtime: WindowRuntime<GtkBackend>,
}

fn build_window(
    app: &adw::Application,
    spec: &WindowSpec,
    main_window: Option<&gtk::Window>,
) -> Result<BuiltWindow, GtkError> {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let layout_context = match spec.kind {
        WindowKind::Panel(_) => LayoutContext::AuxiliaryPanel,
        WindowKind::Main | WindowKind::Preferences => LayoutContext::Standard,
    };
    // The GTK host injects no dialog service yet: a component raising a
    // dialog surfaces the typed RenderError::Dialog(NoPresenter) through the
    // runtime instead of a silent substitute. The adw::AlertDialog and
    // GtkFileDialog realization is tracked in reports/dialogs-and-sheets.
    let runtime = WindowRuntime::mount(
        Renderer::new(GtkBackend::new(&root, layout_context)),
        spec.content.clone(),
        display_platform_services()?,
    )
    .map_err(|error| GtkError(error.to_string()))?;
    let narrow_layout = Rc::new(Cell::new(false));
    let (toolbar, header, adaptive_toolbar_items) =
        runtime.with_renderer(|renderer| build_toolbar(spec, renderer, narrow_layout.clone()));
    toolbar.set_content(Some(&root));

    let initial_content_width = spec.initial_size.width.round() as i32;
    let initial_content_height = spec.initial_size.height.round() as i32;
    let minimum_content_width = spec.minimum_size.width.round() as i32;
    let minimum_content_height = spec.minimum_size.height.round() as i32;
    let (_, header_height, _, _) =
        header.measure(gtk::Orientation::Vertical, initial_content_width);
    let (_, minimum_header_height, _, _) =
        header.measure(gtk::Orientation::Vertical, minimum_content_width);
    let initial_window_height = initial_content_height.saturating_add(header_height);
    let minimum_window_height = minimum_content_height.saturating_add(minimum_header_height);

    let window: gtk::Window = match spec.kind {
        WindowKind::Main => adw::ApplicationWindow::builder()
            .application(app)
            .title(&spec.title)
            .default_width(initial_content_width)
            .default_height(initial_window_height)
            .content(&toolbar)
            .build()
            .upcast(),
        WindowKind::Preferences | WindowKind::Panel(_) => adw::Window::builder()
            .application(app)
            .title(&spec.title)
            .default_width(initial_content_width)
            .default_height(initial_window_height)
            .content(&toolbar)
            .build()
            .upcast(),
    };
    // WindowSpec sizes describe the application content below native chrome.
    // Keep the window minimum limited to that content plus the measured
    // header so users can freely resize to any larger extent.
    window.set_size_request(minimum_content_width, minimum_window_height);
    install_initial_content_extent(
        &window,
        &root,
        initial_content_width,
        initial_content_height,
    );
    schedule_layout_probe(
        &window,
        &toolbar,
        &header,
        &root,
        initial_content_width,
        initial_content_height,
    );
    if let WindowKind::Panel(behavior) = spec.kind {
        configure_panel(&window, behavior, main_window);
    }
    install_adaptive_breakpoint(
        &window,
        spec.minimum_size.width,
        &runtime,
        &adaptive_toolbar_items,
        narrow_layout,
    );
    Ok(BuiltWindow {
        window,
        _runtime: runtime,
    })
}

fn install_initial_content_extent(
    window: &gtk::Window,
    root: &gtk::Box,
    expected_content_width: i32,
    expected_content_height: i32,
) {
    // X11 and Wayland compositors can reserve different decoration extents
    // around the first GtkWindow allocation. Correct the first presentation
    // once from the live root allocation; later user resizes are never
    // observed or rewritten by this path.
    let window = window.clone();
    let root = root.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(25), move || {
        if root.width() <= 0 || root.height() <= 0 {
            return glib::ControlFlow::Continue;
        }
        let width_delta = expected_content_width.saturating_sub(root.width());
        let height_delta = expected_content_height.saturating_sub(root.height());
        if width_delta <= 0 && height_delta <= 0 {
            return glib::ControlFlow::Break;
        }
        window.set_default_size(
            window.default_width().saturating_add(width_delta.max(0)),
            window.default_height().saturating_add(height_delta.max(0)),
        );
        glib::ControlFlow::Break
    });
}

fn install_adaptive_breakpoint(
    window: &gtk::Window,
    minimum_width: f64,
    runtime: &WindowRuntime<GtkBackend>,
    adaptive_toolbar_items: &[gtk::Stack],
    narrow_layout: Rc<Cell<bool>>,
) {
    let mut splits = Vec::new();
    runtime.with_renderer(|renderer| collect_adaptive_splits(renderer.mounted(), &mut splits));
    let splits = splits
        .into_iter()
        .filter_map(|(split, collapsible)| collapsible.then_some(split))
        .collect::<Vec<_>>();
    if splits.is_empty() && adaptive_toolbar_items.is_empty() {
        return;
    }
    let condition = adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        minimum_width,
        adw::LengthUnit::Px,
    );
    let breakpoint = adw::Breakpoint::new(condition);
    let applying = narrow_layout.clone();
    breakpoint.connect_apply(move |_| applying.set(true));
    breakpoint.connect_unapply(move |_| narrow_layout.set(false));
    let collapsed = true.to_value();
    for split in &splits {
        breakpoint.add_setter(split, "collapsed", Some(&collapsed));
    }
    let compact = "compact".to_value();
    for item in adaptive_toolbar_items {
        breakpoint.add_setter(item, "visible-child-name", Some(&compact));
    }
    if let Ok(window) = window.clone().downcast::<adw::ApplicationWindow>() {
        window.add_breakpoint(breakpoint);
    } else if let Ok(window) = window.clone().downcast::<adw::Window>() {
        window.add_breakpoint(breakpoint);
    }
}

fn configure_panel(window: &gtk::Window, behavior: PanelBehavior, main: Option<&gtk::Window>) {
    if behavior.floating
        && let Some(main) = main
    {
        window.set_transient_for(Some(main));
    }
    window.set_hide_on_close(true);
    window.set_focusable(behavior.accepts_keyboard);
    if behavior.hides_when_inactive {
        let hidden_for_inactivity = Rc::new(Cell::new(false));
        observe_application_inactivity(window, window, hidden_for_inactivity.clone());
        if let Some(main) = main {
            observe_application_inactivity(main, window, hidden_for_inactivity);
        }
    }
}

fn observe_application_inactivity(
    trigger: &gtk::Window,
    panel: &gtk::Window,
    hidden_for_inactivity: Rc<Cell<bool>>,
) {
    let panel = panel.downgrade();
    trigger.connect_is_active_notify(move |_| {
        let panel = panel.clone();
        let hidden_for_inactivity = hidden_for_inactivity.clone();
        glib::idle_add_local_once(move || {
            let Some(panel) = panel.upgrade() else {
                return;
            };
            let Some(application) = panel.application() else {
                return;
            };
            if application.windows().iter().any(gtk::Window::is_active) {
                if hidden_for_inactivity.replace(false) {
                    panel.set_visible(true);
                }
            } else if panel.is_visible() {
                hidden_for_inactivity.set(true);
                panel.set_visible(false);
            }
        });
    });
}

fn schedule_layout_probe(
    window: &gtk::Window,
    toolbar: &adw::ToolbarView,
    header: &adw::HeaderBar,
    root: &gtk::Box,
    expected_content_width: i32,
    expected_content_height: i32,
) {
    if std::env::var_os("RINKA_GTK_LAYOUT_PROBE").is_none() {
        return;
    }
    let allocation_window = window.downgrade();
    let allocation_header = header.downgrade();
    let allocation_root = root.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        let Some(window) = allocation_window.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let Some(header) = allocation_header.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let Some(root) = allocation_root.upgrade() else {
            return glib::ControlFlow::Break;
        };
        emit_content_allocation(&window, &header, &root);
        glib::ControlFlow::Continue
    });
    let window = window.clone();
    let toolbar = toolbar.clone();
    let header = header.clone();
    let root = root.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(750), move || {
        emit_content_allocation(&window, &header, &root);
        let content_matches =
            root.width() == expected_content_width && root.height() == expected_content_height;
        eprintln!(
            "RINKA_GTK_WINDOW_CONTRACT title={:?} expected-content={}x{} content={}x{} header={}x{} toolbar={}x{} window={}x{} result={}",
            window.title().unwrap_or_default(),
            expected_content_width,
            expected_content_height,
            root.width(),
            root.height(),
            header.width(),
            header.height(),
            toolbar.width(),
            toolbar.height(),
            window.width(),
            window.height(),
            if content_matches { "PASS" } else { "FAIL" },
        );
    });
}

fn emit_content_allocation(window: &gtk::Window, header: &adw::HeaderBar, root: &gtk::Box) {
    eprintln!(
        "RINKA_GTK_CONTENT_ALLOCATION title={:?} content={}x{} header={}x{} window={}x{}",
        window.title().unwrap_or_default(),
        root.width(),
        root.height(),
        header.width(),
        header.height(),
        window.width(),
        window.height(),
    );
}

/// Runs a libadwaita application and returns its process status.
pub fn run(application: ApplicationSpec) -> i32 {
    if application.windows.is_empty() {
        eprintln!("GTK host error: application has no windows");
        return 1;
    }
    if !application.menu_bar.is_empty() {
        // The application-level bar has no GTK realization yet either
        // (reports/app-menu-bar); reject it like a window-declared bar.
        eprintln!(
            "GTK host error: a declared application menu bar is not yet realized by the GTK host"
        );
        return 1;
    }
    let app = adw::Application::builder()
        .application_id(&application.id)
        .build();
    let built_windows: Rc<RefCell<Vec<BuiltWindow>>> = Rc::new(RefCell::new(Vec::new()));
    let startup_failed = Rc::new(Cell::new(false));
    let activation_failed = startup_failed.clone();
    app.connect_activate(move |app| {
        if !built_windows.borrow().is_empty() {
            if let Some(main) = built_windows
                .borrow()
                .iter()
                .find(|built| built.window.transient_for().is_none())
            {
                main.window.present();
            }
            return;
        }
        let mut main_window: Option<gtk::Window> = None;
        for spec in &application.windows {
            match build_window(app, spec, main_window.as_ref()) {
                Ok(built) => {
                    if matches!(spec.kind, WindowKind::Main) {
                        main_window = Some(built.window.clone());
                    }
                    built.window.present();
                    built_windows.borrow_mut().push(built);
                }
                Err(error) => {
                    activation_failed.set(true);
                    eprintln!("GTK host error: {error}");
                }
            }
        }
    });
    // Consumer arguments belong to the declarative application, not to
    // GApplication's option parser. Supply only the executable identity after
    // the consumer has already interpreted its own command line.
    let status = app.run_with_args(&["rinka"]).value();
    if startup_failed.get() { 1 } else { status }
}

const fn orientation(axis: Axis) -> gtk::Orientation {
    match axis {
        Axis::Horizontal => gtk::Orientation::Horizontal,
        Axis::Vertical => gtk::Orientation::Vertical,
    }
}

const fn gtk_align(align: Align) -> gtk::Align {
    match align {
        Align::Start => gtk::Align::Start,
        Align::Center => gtk::Align::Center,
        Align::End => gtk::Align::End,
        Align::Stretch => gtk::Align::Fill,
    }
}

const fn spacing_pixels(spacing: Spacing) -> i32 {
    match spacing {
        Spacing::Joined => 0,
        Spacing::Compact => 6,
        Spacing::Related => 12,
        Spacing::Section => 18,
        Spacing::Content => 24,
    }
}

const fn content_spacing_pixels(context: LayoutContext, spacing: Spacing) -> i32 {
    match (context, spacing) {
        (LayoutContext::AuxiliaryPanel, Spacing::Section) => spacing_pixels(Spacing::Compact),
        (LayoutContext::AuxiliaryPanel, Spacing::Content) => spacing_pixels(Spacing::Related),
        _ => spacing_pixels(spacing),
    }
}

const fn stack_insets(context: LayoutContext, spacing: Spacing) -> (i32, i32) {
    match (context, spacing) {
        (LayoutContext::AuxiliaryPanel, Spacing::Content) => (
            spacing_pixels(Spacing::Related),
            spacing_pixels(Spacing::Related),
        ),
        _ => {
            let inset = content_spacing_pixels(context, spacing);
            (inset, inset)
        }
    }
}

const fn symbol_name(symbol: Symbol) -> &'static str {
    match symbol {
        Symbol::Back => "go-previous-symbolic",
        Symbol::Forward => "go-next-symbolic",
        Symbol::Add => "list-add-symbolic",
        Symbol::Refresh => "view-refresh-symbolic",
        Symbol::Search => "system-search-symbolic",
        Symbol::Home => "user-home-symbolic",
        Symbol::Folder => "folder-symbolic",
        Symbol::File => "text-x-generic-symbolic",
        Symbol::Code => "text-x-script-symbolic",
        Symbol::Image => "image-x-generic-symbolic",
        Symbol::Terminal => "utilities-terminal-symbolic",
        Symbol::Settings => "emblem-system-symbolic",
        Symbol::More => "view-more-symbolic",
        Symbol::Grid => "view-grid-symbolic",
        Symbol::List => "view-list-symbolic",
        Symbol::Columns => "view-dual-symbolic",
        Symbol::Gallery => "view-paged-symbolic",
        Symbol::Sort => "view-sort-ascending-symbolic",
        Symbol::Share => "send-to-symbolic",
        Symbol::Tag => "tag-symbolic",
        Symbol::Disclosure => "go-next-symbolic",
        Symbol::Warning => "dialog-warning-symbolic",
    }
}

const fn status_icon(tone: StatusTone) -> &'static str {
    match tone {
        StatusTone::Empty => "folder-open-symbolic",
        StatusTone::Busy => "content-loading-symbolic",
        StatusTone::Error => "dialog-warning-symbolic",
        StatusTone::Informational => "dialog-information-symbolic",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LayoutContext, ToolbarLayout, action_group_uses_direct_compact_buttons,
        content_spacing_pixels, progress_percentage_text, run, stack_insets,
        table_cell_accessible_description, toolbar_layout, validate_element,
    };
    use rinka_core::{
        ApplicationSpec, ButtonMaterial, Spacing, ToolbarAction, ToolbarGroupDisplay, ToolbarItem,
        ToolbarPlacement, button, progress,
    };

    #[test]
    fn auxiliary_panel_resolves_compact_native_spacing() {
        assert_eq!(
            content_spacing_pixels(LayoutContext::AuxiliaryPanel, Spacing::Content),
            12
        );
        assert_eq!(
            content_spacing_pixels(LayoutContext::AuxiliaryPanel, Spacing::Section),
            6
        );
        assert_eq!(
            content_spacing_pixels(LayoutContext::AuxiliaryPanel, Spacing::Related),
            12
        );
        assert_eq!(
            content_spacing_pixels(LayoutContext::Standard, Spacing::Content),
            24
        );
        assert_eq!(
            stack_insets(LayoutContext::AuxiliaryPanel, Spacing::Content),
            (12, 12)
        );
    }

    #[test]
    fn validation_rejects_unrepresentable_or_inaccessible_elements() {
        let glass = button("Open", "Open file", || {}).button_material(ButtonMaterial::Glass);
        assert!(validate_element(&glass).is_err());

        let unnamed = button("Open", "", || {});
        assert!(validate_element(&unnamed).is_err());

        let invalid_progress = progress(f64::NAN, "Transfer progress");
        assert!(validate_element(&invalid_progress).is_err());

        let valid = button("Open", "Open file", || {});
        assert!(validate_element(&valid).is_ok());
    }

    #[test]
    fn toolbar_group_display_selects_the_declared_native_representation() {
        let group = || {
            ToolbarItem::action_group(
                "navigation",
                "Navigation",
                "Move through history",
                ToolbarPlacement::Leading,
                [ToolbarAction::new(
                    "back",
                    "Back",
                    rinka_core::Symbol::Back,
                    "Go back",
                    || {},
                )],
            )
        };
        assert_eq!(toolbar_layout(&group()), ToolbarLayout::Adaptive);
        assert_eq!(
            toolbar_layout(&group().group_display(ToolbarGroupDisplay::Expanded)),
            ToolbarLayout::Expanded
        );
        assert_eq!(
            toolbar_layout(&group().group_display(ToolbarGroupDisplay::Collapsed)),
            ToolbarLayout::Compact
        );
    }

    #[test]
    fn compact_navigation_keeps_a_small_action_group_directly_visible() {
        assert!(action_group_uses_direct_compact_buttons(1));
        assert!(action_group_uses_direct_compact_buttons(2));
        assert!(!action_group_uses_direct_compact_buttons(0));
        assert!(!action_group_uses_direct_compact_buttons(3));
    }

    #[test]
    fn progress_text_exposes_the_declared_fraction_as_a_percentage() {
        assert_eq!(progress_percentage_text(0.58), "58%");
        assert_eq!(progress_percentage_text(1.0), "100%");
    }

    #[test]
    fn table_cells_expose_column_specific_descriptions() {
        assert_eq!(table_cell_accessible_description("Name"), "Name column");
        assert_eq!(table_cell_accessible_description("Size"), "Size column");
    }

    #[test]
    fn empty_application_returns_a_failure_status_without_starting_gtk() {
        assert_eq!(
            run(ApplicationSpec {
                id: "jp.bunko.rinka.empty".to_owned(),
                name: "Empty".to_owned(),
                menu_bar: rinka_core::MenuBar::default(),
                windows: Vec::new(),
                last_window_closed: rinka_core::LastWindowClosedPolicy::PlatformDefault,
            }),
            1
        );
    }
}
