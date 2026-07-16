const WINDOW_CONTENT_HEIGHT_CHROME: f64 = 18.0;

thread_local! {
    static APPLICATION: RefCell<Option<PreparedApplication>> = const { RefCell::new(None) };
    static PANEL_HOSTS: RefCell<Vec<ui::ReactorHost>> = const { RefCell::new(Vec::new()) };
    static PANEL_THEME_TIMERS: RefCell<Vec<ui::DispatcherTimer>> = const { RefCell::new(Vec::new()) };
}

struct PreparedApplication {
    main: WindowSpec,
    panels: Vec<WindowSpec>,
}

struct WindowComponent {
    window: WindowSpec,
    panels: Vec<WindowSpec>,
}

windows_core::imp::define_interface!(
    IWindowNative,
    IWindowNativeVtable,
    0xeecdbf0e_bae9_4cb6_a68e_9598e1cb57bb
);
windows_core::imp::interface_hierarchy!(IWindowNative, windows_core::IUnknown);

impl IWindowNative {
    unsafe fn window_handle(&self, hwnd: *mut *mut c_void) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).window_handle)(
                windows_core::Interface::as_raw(self),
                hwnd,
            )
        }
    }
}

#[repr(C)]
pub struct IWindowNativeVtable {
    base__: windows_core::IUnknown_Vtbl,
    window_handle:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> windows_core::HRESULT,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetWindow(hwnd: *mut c_void, command: u32) -> *mut c_void;
    fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, value: isize) -> isize;
}

const GW_OWNER: u32 = 4;
const GWLP_HWNDPARENT: i32 = -8;

#[derive(Clone)]
struct TableRowModel {
    key: String,
    title: String,
    cells: Vec<String>,
    selected: bool,
    events: EventBindings,
}

#[derive(Clone)]
enum ToolbarTarget {
    Activate(EventBindings),
    Input(EventBindings, String),
    Inspector(ui::SetState<bool>, bool),
}

struct ToolbarProjection {
    back: Option<EventBindings>,
    search: Option<ui::Element>,
    commands: Vec<ui::CommandBarCommandDef>,
    secondary_commands: Vec<ui::CommandBarCommandDef>,
    supplementary_controls: Vec<ui::Element>,
    targets: Vec<(String, ToolbarTarget)>,
}

#[derive(Clone)]
enum NavigationTarget {
    Activate(EventBindings),
    Toggle(EventBindings, bool),
}

impl ui::Component for WindowComponent {
    fn render(&self, _props: &(), cx: &mut ui::RenderCx) -> ui::Element {
        let appearance = requested_theme();
        cx.use_effect((), move || ui::set_requested_theme(appearance));
        let inner_size = cx.use_inner_size();
        let (_revision, request_redraw) = cx.use_reducer(0_u64);
        let (pane_open, set_pane_open) = cx.use_state(inner_size.width >= 900.0);
        let (inspector_open, set_inspector_open) = cx.use_state(inner_size.width >= 960.0);
        let sidebar_wide = inner_size.width >= 900.0;
        let inspector_wide = inner_size.width >= 960.0;
        let responsive_pane = set_pane_open.clone();
        let responsive_inspector = set_inspector_open.clone();
        cx.use_effect((sidebar_wide, inspector_wide), move || {
            responsive_pane.call(sidebar_wide);
            responsive_inspector.call(inspector_wide);
        });

        let panels = self.panels.clone();
        let panel_timer = cx.use_memo((), || Rc::new(RefCell::new(None::<ui::DispatcherTimer>)));
        cx.use_effect((), move || {
            if panels.is_empty() {
                return;
            }
            let timer_slot = Rc::clone(&panel_timer);
            match ui::DispatcherTimer::new_one_shot(Duration::from_millis(1), move || {
                if let Err(error) = mount_panels(&panels) {
                    write_diagnostic(&error.to_string());
                }
                // Retain the stopped one-shot timer until the main component is
                // released; detaching it inside its callback is not required.
                let _ = &timer_slot;
            }) {
                Ok(timer) => *panel_timer.borrow_mut() = Some(timer),
                Err(error) => write_diagnostic(&format!("panel timer: {error}")),
            }
        });

        let redraw = request_redraw.clone();
        let projection = cx.use_memo((), || {
            WindowProjection::mount(self.window.content.clone(), crate::platform_services())
                .map(|projection| {
                    let projection = Rc::new(projection);
                    projection.set_reconciled_handler(move || {
                        redraw.call(|revision| revision.wrapping_add(1));
                    });
                    projection
                })
                .map_err(|error| error.to_string())
        });
        let projection = match projection {
            Ok(projection) => projection,
            Err(error) => return render_projection_error(&error),
        };
        if let Some(error) = projection.take_error() {
            return render_projection_error(&error.to_string());
        }
        if let Some(diagnostic) = projection.with_root(first_unsupported_element).flatten() {
            // Typed unsupported-capability rejection: the platform pass never
            // substitutes a visually unrelated control for the element.
            return render_projection_error(&diagnostic.to_string());
        }

        projection
            .with_root(|root| {
                render_window(
                    root,
                    &self.window,
                    pane_open,
                    set_pane_open,
                    inspector_open,
                    set_inspector_open,
                    inner_size,
                )
            })
            .unwrap_or_else(|| render_projection_error("projected root is missing"))
    }
}

pub(crate) fn run(application: ApplicationSpec) -> Result<(), WinUiDiagnostic> {
    let prepared = prepare(application)?;
    let title = prepared.main.title.clone();
    let initial_size = prepared.main.initial_size;
    let minimum_size = prepared.main.minimum_size;

    APPLICATION.with(|slot| *slot.borrow_mut() = Some(prepared));
    PANEL_HOSTS.with(|hosts| hosts.borrow_mut().clear());
    PANEL_THEME_TIMERS.with(|timers| timers.borrow_mut().clear());

    let result = ui::App::new()
        .title(title)
        .inner_size(
            initial_size.width,
            initial_size.height + WINDOW_CONTENT_HEIGHT_CHROME,
        )
        .inner_constraints(ui::InnerConstraints {
            min_width: Some(minimum_size.width),
            min_height: Some(minimum_size.height + WINDOW_CONTENT_HEIGHT_CHROME),
            max_width: None,
            max_height: None,
        })
        .backdrop(ui::Backdrop::Mica)
        .eager_templated_realization(true)
        .on_fault(|fault| {
            write_diagnostic(&format!("{}: {}", fault.context, fault.message));
        })
        .run(create_root_component)
        .map_err(|error| WinUiDiagnostic::Native(error.to_string()));

    APPLICATION.with(|slot| slot.borrow_mut().take());
    result
}

fn requested_theme() -> ui::RequestedTheme {
    match std::env::var("RINKA_WINDOWS_APPEARANCE") {
        Ok(value) if value.eq_ignore_ascii_case("light") => ui::RequestedTheme::Light,
        Ok(value) if value.eq_ignore_ascii_case("dark") => ui::RequestedTheme::Dark,
        _ => ui::RequestedTheme::Default,
    }
}

fn prepare(application: ApplicationSpec) -> Result<PreparedApplication, WinUiDiagnostic> {
    let mut main = None;
    let mut panels = Vec::new();
    for window in application.windows {
        WindowProjection::mount(window.content.clone(), crate::platform_services())
            .map_err(|error| WinUiDiagnostic::Projection(error.to_string()))?;
        match window.kind {
            WindowKind::Main => main = Some(window),
            WindowKind::Panel(_) => panels.push(window),
            WindowKind::Preferences => {
                return Err(WinUiDiagnostic::UnsupportedWindowKind {
                    window_id: window.id.as_str().to_owned(),
                    kind: window.kind,
                });
            }
        }
    }
    Ok(PreparedApplication {
        main: main.ok_or(WinUiDiagnostic::MissingMainWindow)?,
        panels,
    })
}

fn create_root_component() -> WindowComponent {
    let prepared = APPLICATION
        .with(|slot| slot.borrow_mut().take())
        .expect("validated WinUI application must be present on the UI thread");
    WindowComponent {
        window: prepared.main,
        panels: prepared.panels,
    }
}

fn mount_panels(panels: &[WindowSpec]) -> Result<(), WinUiDiagnostic> {
    let owner_hwnd = ui::with_active_host(native_window_handle)
        .ok_or_else(|| WinUiDiagnostic::Native("main host is not active".to_owned()))??;
    let mut hosts = Vec::with_capacity(panels.len());
    let mut theme_timers = Vec::with_capacity(panels.len());
    for panel in panels {
        let WindowKind::Panel(behavior) = panel.kind else {
            return Err(WinUiDiagnostic::UnsupportedWindowKind {
                window_id: panel.id.as_str().to_owned(),
                kind: panel.kind,
            });
        };
        let size = panel.initial_size;
        let minimum = panel.minimum_size;
        let title = panel.title.clone();
        let host = ui::ReactorHost::new_with_window_options(
            title,
            Some(ui::WindowSize {
                width: size.width,
                height: size.height + WINDOW_CONTENT_HEIGHT_CHROME,
            }),
            ui::InnerConstraints {
                min_width: Some(minimum.width),
                min_height: Some(minimum.height + WINDOW_CONTENT_HEIGHT_CHROME),
                max_width: None,
                max_height: None,
            },
            Box::new(WindowComponent {
                window: panel.clone(),
                panels: Vec::new(),
            }),
            |_| {},
        )
        .map_err(|error| {
            WinUiDiagnostic::Native(format!("panel '{}' creation: {error}", panel.id.as_str()))
        })?;
        let panel_hwnd = native_window_handle(&host)?;
        unsafe {
            SetWindowLongPtrW(panel_hwnd, GWLP_HWNDPARENT, owner_hwnd as isize);
            if GetWindow(panel_hwnd, GW_OWNER) != owner_hwnd {
                return Err(WinUiDiagnostic::Native(format!(
                    "panel '{}' owner relationship was not applied",
                    panel.id.as_str()
                )));
            }
        }
        if behavior.floating {
            host.set_presenter(ui::PresenterKind::CompactOverlay);
        }
        host.set_backdrop(ui::Backdrop::Mica);
        host.activate().map_err(|error| {
            WinUiDiagnostic::Native(format!("panel '{}' activation: {error}", panel.id.as_str()))
        })?;
        let appearance = requested_theme();
        let theme_timer = ui::DispatcherTimer::new_one_shot(Duration::from_millis(50), move || {
            ui::set_requested_theme(appearance);
        })
        .map_err(|error| {
            WinUiDiagnostic::Native(format!("panel '{}' theme timer: {error}", panel.id.as_str()))
        })?;
        theme_timers.push(theme_timer);
        hosts.push(host);
    }
    PANEL_HOSTS.with(|slot| *slot.borrow_mut() = hosts);
    PANEL_THEME_TIMERS.with(|slot| *slot.borrow_mut() = theme_timers);
    Ok(())
}

fn native_window_handle(host: &ui::ReactorHost) -> Result<*mut c_void, WinUiDiagnostic> {
    let native = windows_core::Interface::cast::<IWindowNative>(host.window())
        .map_err(|error| WinUiDiagnostic::Native(format!("native window interface: {error}")))?;
    let mut hwnd = std::ptr::null_mut();
    unsafe {
        native
            .window_handle(&mut hwnd)
            .ok()
            .map_err(|error| WinUiDiagnostic::Native(format!("native window handle: {error}")))?;
    }
    if hwnd.is_null() {
        return Err(WinUiDiagnostic::Native(
            "native window handle is null".to_owned(),
        ));
    }
    Ok(hwnd)
}

fn write_diagnostic(message: &str) {
    if let Ok(path) = std::env::var("RINKA_WINUI_DIAGNOSTIC") {
        let _ = std::fs::write(path, message);
    }
}

fn render_window(
    root: &MountedNode<ProjectedHandle>,
    window: &WindowSpec,
    pane_open: bool,
    set_pane_open: ui::SetState<bool>,
    inspector_open: bool,
    set_inspector_open: ui::SetState<bool>,
    inner_size: ui::WindowSize,
) -> ui::Element {
    if matches!(
        root.element().props(),
        Props::Pattern {
            pattern: UiPattern::NavigationWorkspace { .. }
        }
    ) && root.children().len() == 3
    {
        return render_workspace(
            root,
            window,
            pane_open,
            set_pane_open,
            inspector_open,
            set_inspector_open,
            inner_size,
        );
    }

    let title_bar = ui::TitleBar::new(&window.title)
        .with_key(format!("window-titlebar-{}", root.handle().value()));
    ui::grid((
        title_bar.grid_row(0),
        render_node(root)
            .margin(ui::Thickness::uniform(content_gutter(inner_size.width)))
            .grid_row(1),
    ))
    .rows([ui::GridLength::Auto, ui::GridLength::Star(1.0)])
    .horizontal_alignment(ui::HorizontalAlignment::Stretch)
    .vertical_alignment(ui::VerticalAlignment::Stretch)
    .with_key(format!("window-root-{}", root.handle().value()))
    .into()
}
#[allow(clippy::too_many_arguments)]
fn render_workspace(
    root: &MountedNode<ProjectedHandle>,
    window: &WindowSpec,
    pane_open: bool,
    set_pane_open: ui::SetState<bool>,
    inspector_open: bool,
    set_inspector_open: ui::SetState<bool>,
    inner_size: ui::WindowSize,
) -> ui::Element {
    let sidebar = &root.children()[0];
    let primary = &root.children()[1];
    let inspector = &root.children()[2];
    let Props::Pattern {
        pattern:
            UiPattern::NavigationWorkspace {
                sidebar_collapsible,
                inspector_collapsible,
            },
    } = root.element().props()
    else {
        unreachable!("workspace renderer requires workspace properties")
    };
    let visibility = resolve_workspace_visibility(
        *sidebar_collapsible,
        pane_open,
        *inspector_collapsible,
        inspector_open,
        inner_size.width,
    );
    let (location, path) = primary_location(primary);
    let title = location.unwrap_or_else(|| window.title.clone());
    let gutter = content_gutter(inner_size.width);

    let toolbar = project_toolbar(
        window,
        *inspector_collapsible,
        set_inspector_open.clone(),
        inspector_open,
    );
    let mut title_bar = ui::TitleBar::new(title)
        .pane_toggle_button_visible(*sidebar_collapsible)
        .with_key(format!("titlebar-{}", root.handle().value()));
    if *sidebar_collapsible {
        title_bar = title_bar.on_pane_toggle_requested(move || set_pane_open.call(!pane_open));
    }
    if let Some(path) = path {
        title_bar = title_bar.subtitle(path);
    }
    if let Some(binding) = toolbar.back {
        title_bar = title_bar
            .back_button_visible(true)
            .back_button_enabled(true)
            .on_back_requested(move || binding.emit_activate());
    }
    if let Some(search) = toolbar.search {
        title_bar = title_bar.content(search.width(340.0));
    }

    let command_bar = render_command_bar(
        toolbar.commands,
        toolbar.secondary_commands,
        toolbar.supplementary_controls,
        toolbar.targets,
        root.handle().value(),
    );
    let primary_content = render_primary(primary, command_bar, gutter);
    let content = if visibility.inspector_open {
        ui::grid((
            primary_content.grid_column(0),
            render_separator(
                Axis::Vertical,
                format!("inspector-divider-{}", root.handle().value()),
            )
            .grid_column(1),
            ui::scroll_view(render_node(inspector))
                .horizontal_alignment(ui::HorizontalAlignment::Stretch)
                .vertical_alignment(ui::VerticalAlignment::Stretch)
                .with_key(format!("inspector-scroll-{}", inspector.handle().value()))
                .grid_column(2),
        ))
        .columns([
            ui::GridLength::Star(1.0),
            ui::GridLength::Pixel(1.0),
            ui::GridLength::Pixel(280.0),
        ])
        .horizontal_alignment(ui::HorizontalAlignment::Stretch)
        .vertical_alignment(ui::VerticalAlignment::Stretch)
        .with_key(format!("workspace-content-{}", root.handle().value()))
        .into()
    } else {
        primary_content
    };

    let (navigation_items, navigation_targets, selected_tag) = project_navigation(sidebar);
    let targets = Rc::new(navigation_targets);
    let callback_targets = Rc::clone(&targets);
    let mut navigation = ui::NavigationView::new(navigation_items, content)
        .on_selection_changed(move |tag: String| {
            if let Some((_, target)) = callback_targets.iter().find(|(key, _)| key == &tag) {
                match target {
                    NavigationTarget::Activate(binding) => binding.emit_activate(),
                    NavigationTarget::Toggle(binding, value) => binding.emit_toggle(*value),
                }
            }
        })
        .pane_open(visibility.sidebar_open)
        .pane_display_mode(if *sidebar_collapsible {
            ui::NavigationViewPaneDisplayMode::Auto
        } else {
            ui::NavigationViewPaneDisplayMode::Left
        })
        .pane_toggle_button_visible(false)
        .back_button_visible(false)
        .pane_title("Rinka")
        .open_pane_length(280.0)
        .settings_visible(false)
        .horizontal_alignment(ui::HorizontalAlignment::Stretch)
        .vertical_alignment(ui::VerticalAlignment::Stretch)
        .with_key(format!("navigation-{}", sidebar.handle().value()));
    if let Some(tag) = selected_tag {
        navigation = navigation.selected_tag(tag);
    }
    if let Some(footer) = sidebar.children().get(1) {
        navigation = navigation.pane_footer(render_node(footer));
    }

    ui::grid((title_bar.grid_row(0), navigation.grid_row(1)))
        .rows([ui::GridLength::Auto, ui::GridLength::Star(1.0)])
        .horizontal_alignment(ui::HorizontalAlignment::Stretch)
        .vertical_alignment(ui::VerticalAlignment::Stretch)
        .with_key(format!("workspace-shell-{}", root.handle().value()))
        .into()
}

fn primary_location(primary: &MountedNode<ProjectedHandle>) -> (Option<String>, Option<String>) {
    let Some(header) = primary.children().first() else {
        return (None, None);
    };
    let mut labels = header
        .children()
        .iter()
        .filter_map(|node| match node.element().props() {
            Props::Label { text, .. } => Some(text.clone()),
            _ => None,
        });
    (labels.next(), labels.next())
}

fn content_gutter(width: f64) -> f64 {
    if width >= 640.0 { 24.0 } else { 12.0 }
}

fn render_primary(
    primary: &MountedNode<ProjectedHandle>,
    command_bar: Option<ui::Element>,
    gutter: f64,
) -> ui::Element {
    let Props::Stack {
        axis: Axis::Vertical,
        ..
    } = primary.element().props()
    else {
        return render_node(primary);
    };
    if primary.children().len() < 5 {
        return render_node(primary);
    }

    let body = &primary.children()[2];
    let status = &primary.children()[4];
    let command = command_bar.unwrap_or_else(|| {
        ui::grid(())
            .height(0.0)
            .with_key(format!("empty-toolbar-{}", primary.handle().value()))
            .into()
    });
    ui::grid((
        command.grid_row(0),
        render_separator(
            Axis::Horizontal,
            format!("content-leading-divider-{}", primary.handle().value()),
        )
        .grid_row(1),
        render_node(body).grid_row(2),
        render_separator(
            Axis::Horizontal,
            format!("content-status-divider-{}", primary.handle().value()),
        )
        .grid_row(3),
        render_status_row(status).grid_row(4),
    ))
    .rows([
        ui::GridLength::Auto,
        ui::GridLength::Pixel(1.0),
        ui::GridLength::Star(1.0),
        ui::GridLength::Pixel(1.0),
        ui::GridLength::Auto,
    ])
    .margin(ui::Thickness {
        left: gutter,
        top: gutter,
        right: gutter,
        bottom: 8.0,
    })
    .horizontal_alignment(ui::HorizontalAlignment::Stretch)
    .vertical_alignment(ui::VerticalAlignment::Stretch)
    .with_key(format!("primary-content-{}", primary.handle().value()))
    .into()
}

fn render_status_row(status: &MountedNode<ProjectedHandle>) -> ui::Element {
    let left = status
        .children()
        .first()
        .map(render_node)
        .unwrap_or(ui::Element::Empty);
    let right = status
        .children()
        .last()
        .map(render_node)
        .unwrap_or(ui::Element::Empty);
    ui::grid((
        left.grid_column(0),
        right
            .horizontal_alignment(ui::HorizontalAlignment::Right)
            .grid_column(1),
    ))
    .columns([ui::GridLength::Star(1.0), ui::GridLength::Auto])
    .margin(ui::Thickness::xy(12.0, 8.0))
    .horizontal_alignment(ui::HorizontalAlignment::Stretch)
    .with_key(format!("status-row-{}", status.handle().value()))
    .into()
}
