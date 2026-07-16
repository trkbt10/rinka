use crate::{WinUiDiagnostic, resolve_workspace_visibility};
use rinka_core::{
    Align, ApplicationSpec, Axis, ButtonRole, ControlSize, EventBindings, InputKind, Justify,
    ListRowRole, ListStyle, MountedNode, ProjectedHandle, Props, SortDirection, Spacing, SplitRole,
    StatusTone, Symbol as CommonSymbol, TableColumn, TableSort, TextRole, ToolbarAction,
    ToolbarItemKind, ToolbarMenuEntry, WindowKind, WindowProjection, WindowSpec,
};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use std::time::Duration;
use ui::ElementExt as _;
use windows_reactor as ui;

// UI Automation measures the native TitleBar at 48 epx. Extending the pinned
// host into that row adds 30 epx to the pre-sized client, so an 18 epx reserve
// preserves the exact WindowSpec content height below the title bar.
const WINDOW_CONTENT_HEIGHT_CHROME: f64 = 18.0;

thread_local! {
    static APPLICATION: RefCell<Option<PreparedApplication>> = const { RefCell::new(None) };
    static PANEL_HOSTS: RefCell<Vec<ui::ReactorHost>> = const { RefCell::new(Vec::new()) };
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
            WindowProjection::mount(self.window.content.clone())
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

fn prepare(application: ApplicationSpec) -> Result<PreparedApplication, WinUiDiagnostic> {
    let mut main = None;
    let mut panels = Vec::new();
    for window in application.windows {
        WindowProjection::mount(window.content.clone())
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
        hosts.push(host);
    }
    PANEL_HOSTS.with(|slot| *slot.borrow_mut() = hosts);
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
    if matches!(root.element().props(), Props::Workspace { .. }) && root.children().len() == 3 {
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
    let Props::Workspace {
        sidebar_collapsible,
        inspector_collapsible,
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
    .padding(ui::Thickness::xy(12.0, 8.0))
    .horizontal_alignment(ui::HorizontalAlignment::Stretch)
    .with_key(format!("status-row-{}", status.handle().value()))
    .into()
}

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
                for entry in entries {
                    match entry {
                        ToolbarMenuEntry::Action(action) => {
                            push_action(
                                action,
                                &mut secondary_commands,
                                &mut supplementary_controls,
                                &mut targets,
                            );
                        }
                        ToolbarMenuEntry::Separator => {
                            secondary_commands.push(ui::app_bar_separator());
                        }
                    }
                }
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
        Props::Split { role, .. } => render_split(node, *role, key),
        Props::Workspace { .. } => {
            ui::vstack(node.children().iter().map(render_node).collect::<Vec<_>>())
                .with_key(key)
                .into()
        }
        Props::List { style, columns, .. } => match style {
            ListStyle::Table => render_table(node, columns, key),
            ListStyle::Source | ListStyle::Content | ListStyle::Plain => {
                render_plain_list(node, key)
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

fn render_split(node: &MountedNode<ProjectedHandle>, role: SplitRole, key: String) -> ui::Element {
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
    let columns = match role {
        SplitRole::Navigation => [ui::GridLength::Pixel(280.0), ui::GridLength::Star(1.0)],
        SplitRole::Utility => [ui::GridLength::Star(1.0), ui::GridLength::Pixel(280.0)],
    };
    ui::grid((leading, trailing))
        .columns(columns)
        .horizontal_alignment(ui::HorizontalAlignment::Stretch)
        .vertical_alignment(ui::VerticalAlignment::Stretch)
        .with_key(key)
        .into()
}

fn render_plain_list(node: &MountedNode<ProjectedHandle>, key: String) -> ui::Element {
    let rows = table_rows(node);
    let selection_events = rows
        .iter()
        .map(|row| row.events.clone())
        .collect::<Vec<_>>();
    let selected = rows
        .iter()
        .position(|row| row.selected)
        .map_or(-1, |index| index as i32);
    ui::list_view(rows, |row, _| {
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
    .into()
}

fn render_table(
    node: &MountedNode<ProjectedHandle>,
    columns: &[TableColumn],
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
        .padding(ui::Thickness::xy(12.0, 8.0))
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
