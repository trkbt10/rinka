//! Deterministic consumer scenes shared by both native hosts.

use rinka::{
    Align, ApplicationSpec, Axis, ButtonRole, CollectionPattern, Component, ControlSize, Dispatch,
    Element, Justify, PanelBehavior, Size, SortDirection, Spacing, StatusTone, Symbol, TableColumn,
    TableSort, TextRole, ToolbarAction, ToolbarChoice, ToolbarDisplay, ToolbarGroupDisplay,
    ToolbarItem, ToolbarMenuEntry, ToolbarPlacement, UiPattern, WindowContent, WindowId,
    WindowKind, WindowSpec, button, column, label, list, list_row, mount_pattern, progress, row,
    separator, spacer, status, toggle,
};

/// Meaningful UI state used by the consumer verification matrix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scene {
    /// Populated file listing.
    Ready,
    /// Directory with no entries.
    Empty,
    /// Directory refresh in progress.
    Busy,
    /// Directory read failure.
    Error,
}

impl Scene {
    /// Stable extraction identifier.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Empty => "empty",
            Self::Busy => "busy",
            Self::Error => "error",
        }
    }

    /// Parses a command-line scene.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(Self::Ready),
            "empty" => Some(Self::Empty),
            "busy" => Some(Self::Busy),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    /// Returns every required state in deterministic order.
    pub const fn all() -> [Self; 4] {
        [Self::Ready, Self::Empty, Self::Busy, Self::Error]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Location {
    Home,
    Documents,
    Downloads,
    RemoteProject,
}

impl Location {
    const fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Documents => "Documents",
            Self::Downloads => "Downloads",
            Self::RemoteProject => "Remote Project",
        }
    }

    const fn path(self) -> &'static str {
        match self {
            Self::Home if cfg!(target_os = "linux") => "/home/ubuntu",
            Self::Documents if cfg!(target_os = "linux") => "/home/ubuntu/Documents",
            Self::Downloads if cfg!(target_os = "linux") => "/home/ubuntu/Downloads",
            Self::RemoteProject if cfg!(target_os = "linux") => "/home/ubuntu/rinka",
            Self::Home => "/Users/trkbt10",
            Self::Documents => "/Users/trkbt10/Documents",
            Self::Downloads => "/Users/trkbt10/Downloads",
            Self::RemoteProject => "/home/trkbt10/project",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileKey {
    Src,
    Lib,
    Main,
    Assets,
    AppIcon,
    PreviewAssets,
    Cargo,
    Readme,
    Preview,
    HiddenEnvironment,
}

#[derive(Clone, Copy)]
struct FileRecord {
    key: FileKey,
    title: &'static str,
    modified: &'static str,
    size: &'static str,
    kind: &'static str,
    symbol: Symbol,
}

struct ExplorerComponent {
    scene: Scene,
    location: Location,
    selected_file: Option<FileKey>,
    show_hidden: bool,
    favorites_expanded: bool,
    locations_expanded: bool,
    src_expanded: bool,
    assets_expanded: bool,
    sort: TableSort,
}

impl ExplorerComponent {
    fn new(scene: Scene) -> Self {
        Self {
            scene,
            location: Location::RemoteProject,
            selected_file: (scene == Scene::Ready).then_some(FileKey::Cargo),
            show_hidden: false,
            favorites_expanded: true,
            locations_expanded: true,
            src_expanded: false,
            assets_expanded: false,
            sort: TableSort {
                column_id: "name".to_owned(),
                direction: SortDirection::Ascending,
            },
        }
    }
}

enum ExplorerMessage {
    SelectLocation(Location),
    SelectFile(FileKey),
    SetShowHidden(bool),
    SetSort(TableSort),
    SetSectionExpanded(&'static str, bool),
    SetFileExpanded(FileKey, bool),
}

impl Component for ExplorerComponent {
    type Message = ExplorerMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            ExplorerMessage::SelectLocation(location) => {
                self.location = location;
                self.selected_file = (location == Location::RemoteProject
                    && self.scene == Scene::Ready)
                    .then_some(FileKey::Cargo);
            }
            ExplorerMessage::SelectFile(file) => self.selected_file = Some(file),
            ExplorerMessage::SetShowHidden(value) => {
                self.show_hidden = value;
                if !value && self.selected_file == Some(FileKey::HiddenEnvironment) {
                    self.selected_file = None;
                }
            }
            ExplorerMessage::SetSort(sort) => self.sort = sort,
            ExplorerMessage::SetSectionExpanded(section, expanded) => match section {
                "favorites" => self.favorites_expanded = expanded,
                "locations" => self.locations_expanded = expanded,
                _ => {}
            },
            ExplorerMessage::SetFileExpanded(file, expanded) => match file {
                FileKey::Src => self.src_expanded = expanded,
                FileKey::Assets => self.assets_expanded = expanded,
                _ => {}
            },
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        explorer_content(self, dispatch)
    }
}

fn announce(action: &'static str) -> impl Fn() {
    move || eprintln!("action={action}")
}

/// Builds the complete native application contract.
pub fn application(scene: Scene) -> ApplicationSpec {
    let mut windows = vec![main_window(scene)];
    if scene == Scene::Busy {
        windows.push(activity_panel());
    }
    ApplicationSpec {
        id: "jp.bunko.rinka.explorer".to_owned(),
        name: "Rinka Explorer".to_owned(),
        windows,
    }
}

fn main_window(scene: Scene) -> WindowSpec {
    let content_fit_probe = std::env::var_os("RINKA_APPKIT_CONTENT_FIT_PROBE").is_some();
    WindowSpec {
        id: WindowId::new("explorer-main"),
        title: "Rinka Explorer".to_owned(),
        kind: WindowKind::Main,
        initial_size: if content_fit_probe {
            Size::new(760.0, 520.0)
        } else {
            Size::new(1120.0, 720.0)
        },
        minimum_size: Size::new(760.0, 520.0),
        toolbar_display: ToolbarDisplay::IconOnly,
        toolbar: if content_fit_probe {
            Vec::new()
        } else {
            vec![
                ToolbarItem::action_group(
                    "navigation",
                    "Navigation",
                    "Move backward or forward through location history",
                    ToolbarPlacement::Leading,
                    [
                        ToolbarAction::new(
                            "back",
                            "Back",
                            Symbol::Back,
                            "Return to the previous location",
                            announce("navigate-back"),
                        ),
                        ToolbarAction::new(
                            "forward",
                            "Forward",
                            Symbol::Forward,
                            "Move to the next location",
                            announce("navigate-forward"),
                        )
                        .enabled(false),
                    ],
                ),
                ToolbarItem::new(
                    "add-folder",
                    "New Folder",
                    Symbol::Add,
                    "Create a folder in the current location",
                    ToolbarPlacement::Leading,
                    announce("new-folder"),
                ),
                ToolbarItem::selection_group(
                    "view-mode",
                    "View",
                    "Choose the file presentation",
                    ToolbarPlacement::Center,
                    [
                        ToolbarChoice::new("grid", "Grid", Symbol::Grid),
                        ToolbarChoice::new("list", "List", Symbol::List),
                        ToolbarChoice::new("columns", "Columns", Symbol::Columns),
                        ToolbarChoice::new("gallery", "Gallery", Symbol::Gallery),
                    ],
                    "list",
                    |selection| eprintln!("view-mode={selection}"),
                )
                .group_display(ToolbarGroupDisplay::Expanded),
                ToolbarItem::menu(
                    "arrange",
                    "Arrange",
                    Symbol::Sort,
                    "Sort and group the file list",
                    ToolbarPlacement::Trailing,
                    [
                        ToolbarMenuEntry::action(ToolbarAction::new(
                            "sort-name",
                            "Name",
                            Symbol::List,
                            "Sort by name",
                            announce("sort-name"),
                        )),
                        ToolbarMenuEntry::action(ToolbarAction::new(
                            "sort-modified",
                            "Date Modified",
                            Symbol::Refresh,
                            "Sort by modification date",
                            announce("sort-modified"),
                        )),
                        ToolbarMenuEntry::separator(),
                        ToolbarMenuEntry::action(ToolbarAction::new(
                            "group-kind",
                            "Group by Kind",
                            Symbol::Columns,
                            "Group files by kind",
                            announce("group-kind"),
                        )),
                    ],
                ),
                ToolbarItem::action_group(
                    "file-actions",
                    "File Actions",
                    "Share, tag, or open more actions",
                    ToolbarPlacement::Trailing,
                    [
                        ToolbarAction::new(
                            "share",
                            "Share",
                            Symbol::Share,
                            "Share the selected file",
                            announce("share"),
                        ),
                        ToolbarAction::new(
                            "tag",
                            "Tags",
                            Symbol::Tag,
                            "Tag the selected file",
                            announce("tag"),
                        ),
                        ToolbarAction::new(
                            "more",
                            "More",
                            Symbol::More,
                            "More actions for the selected file",
                            announce("more"),
                        ),
                    ],
                ),
                ToolbarItem::search(
                    "search",
                    "Search",
                    "",
                    "Search",
                    "Search files",
                    "Search files in Remote Project",
                    ToolbarPlacement::Trailing,
                    |query| eprintln!("search={query}"),
                ),
            ]
        },
        content: WindowContent::component(ExplorerComponent::new(scene)),
    }
}

fn explorer_content(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    mount_pattern(
        UiPattern::NavigationWorkspace {
            sidebar_collapsible: true,
            inspector_collapsible: true,
        },
        [
            sidebar(model, dispatch.clone()),
            directory_content(model, dispatch),
            inspector(model),
        ],
    )
    .with_key("explorer-workspace")
}

fn sidebar(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    let favorites_dispatch = dispatch.clone();
    let locations_dispatch = dispatch.clone();
    column([
        list(
            "Locations",
            [
                list_row(
                    "Favorites",
                    None,
                    None,
                    false,
                    false,
                    "Favorites",
                    announce("section-favorites"),
                )
                .section_header()
                .expanded(model.favorites_expanded)
                .on_expansion_change(move |expanded| {
                    favorites_dispatch
                        .emit(ExplorerMessage::SetSectionExpanded("favorites", expanded));
                })
                .outline_children([
                    location_row(Location::Home, Symbol::Home, model, dispatch.clone()),
                    location_row(Location::Documents, Symbol::Folder, model, dispatch.clone()),
                    location_row(Location::Downloads, Symbol::Folder, model, dispatch.clone()),
                ])
                .with_key("section-favorites"),
                list_row(
                    "Locations",
                    None,
                    None,
                    false,
                    false,
                    "Locations",
                    announce("section-locations"),
                )
                .section_header()
                .expanded(model.locations_expanded)
                .on_expansion_change(move |expanded| {
                    locations_dispatch
                        .emit(ExplorerMessage::SetSectionExpanded("locations", expanded));
                })
                .outline_children([location_row(
                    Location::RemoteProject,
                    Symbol::Folder,
                    model,
                    dispatch.clone(),
                )])
                .with_key("section-locations"),
            ],
        )
        .collection_pattern(CollectionPattern::NavigationSidebar)
        .with_key("locations-list"),
        column([
            separator(Axis::Horizontal).with_key("sidebar-separator"),
            toggle(
                "Show hidden files",
                model.show_hidden,
                "Show hidden files",
                move |value| dispatch.emit(ExplorerMessage::SetShowHidden(value)),
            )
            .control_size(ControlSize::Small)
            .with_key("show-hidden"),
        ])
        .spacing(Spacing::Section)
        .padding(Spacing::Content)
        .with_key("sidebar-footer"),
    ])
    .spacing(Spacing::Joined)
    .with_key("sidebar")
}

fn location_row(
    location: Location,
    symbol: Symbol,
    model: &ExplorerComponent,
    dispatch: Dispatch<ExplorerMessage>,
) -> Element {
    let title = if location == Location::RemoteProject
        && std::env::var_os("RINKA_APPKIT_CONTENT_FIT_PROBE").is_some()
    {
        "Remote Project — Content Fit Ownership Verification Location Identifier"
    } else {
        location.title()
    };
    list_row(
        title,
        None,
        Some(symbol),
        model.location == location,
        false,
        title,
        move || dispatch.emit(ExplorerMessage::SelectLocation(location)),
    )
    .with_key(format!(
        "location-{}",
        title.to_lowercase().replace(' ', "-")
    ))
}

fn directory_content(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    column([
        column([
            label(model.location.title())
                .text_role(TextRole::Title)
                .with_key("directory-title"),
            label(model.location.path())
                .text_role(TextRole::Monospace)
                .selectable(true)
                .with_key("directory-path"),
        ])
        .spacing(Spacing::Compact)
        .padding(Spacing::Content)
        .with_key("directory-header"),
        separator(Axis::Horizontal).with_key("directory-separator"),
        scene_body(model, dispatch),
        separator(Axis::Horizontal).with_key("status-separator"),
        row([
            label(scene_summary(model))
                .text_role(TextRole::Secondary)
                .with_key("item-summary"),
            spacer(true, false).with_key("status-space"),
            label(connection_status(model.scene))
                .text_role(TextRole::Secondary)
                .with_key("connection-status"),
        ])
        .align(Align::Center)
        .padding(Spacing::Content)
        .with_key("status-row"),
    ])
    .spacing(Spacing::Joined)
    .with_key("directory-content")
}

fn scene_body(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    match model.scene {
        Scene::Ready => file_list(model, dispatch),
        Scene::Empty => column([status(
            "This folder is empty",
            "Create a folder or drop files here to begin.",
            StatusTone::Empty,
        )
        .with_key("directory-empty")])
        .justify(Justify::Center)
        .with_key("directory-empty-layout"),
        Scene::Busy => column([
            status(
                "Refreshing Remote Project",
                "Reading directory metadata over the existing SSH connection.",
                StatusTone::Busy,
            )
            .with_key("directory-busy"),
            row([
                spacer(true, false).with_key("refresh-progress-leading-space"),
                progress(0.58, "Directory refresh 58 percent").with_key("refresh-progress"),
                spacer(true, false).with_key("refresh-progress-trailing-space"),
            ])
            .with_key("refresh-progress-row"),
        ])
        .justify(Justify::Center)
        .spacing(Spacing::Section)
        .with_key("directory-busy-stack"),
        Scene::Error => column([
            status(
                "Remote Project is unavailable",
                "The SSH session closed before the directory response completed.",
                StatusTone::Error,
            )
            .with_key("directory-error"),
            row([
                spacer(true, false).with_key("reconnect-leading-space"),
                button(
                    "Reconnect",
                    "Reconnect to Remote Project",
                    announce("reconnect"),
                )
                .button_role(ButtonRole::Primary)
                .control_size(ControlSize::Large)
                .with_key("reconnect"),
                spacer(true, false).with_key("reconnect-trailing-space"),
            ])
            .align(Align::Center)
            .with_key("reconnect-actions"),
        ])
        .justify(Justify::Center)
        .spacing(Spacing::Section)
        .with_key("directory-error-stack"),
    }
}

fn file_list(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    let rows = file_records(model)
        .into_iter()
        .map(|record| file_row(record, model, dispatch.clone()))
        .collect::<Vec<_>>();

    let column = |id: &'static str, title: &'static str| {
        if model.sort.column_id == id {
            TableColumn::new(id, title).sorted(model.sort.direction)
        } else {
            TableColumn::new(id, title).sortable(true)
        }
    };
    list(format!("Files in {}", model.location.title()), rows)
        .table_columns([
            column("name", "Name"),
            column("modified", "Date Modified"),
            column("size", "Size"),
            column("kind", "Kind"),
        ])
        .collection_pattern(CollectionPattern::DataTable)
        .on_sort_change(move |sort| dispatch.emit(ExplorerMessage::SetSort(sort)))
        .with_key("file-list")
}

fn file_row(
    record: FileRecord,
    model: &ExplorerComponent,
    dispatch: Dispatch<ExplorerMessage>,
) -> Element {
    let select_dispatch = dispatch.clone();
    let mut row = list_row(
        record.title,
        Some(format!("{} · {}", record.kind, record.size)),
        Some(record.symbol),
        model.selected_file == Some(record.key),
        false,
        format!("{}, {}, {}", record.title, record.kind, record.size),
        move || select_dispatch.emit(ExplorerMessage::SelectFile(record.key)),
    )
    .table_cells([record.modified, record.size, record.kind])
    .with_key(format!("file-{:?}", record.key));

    let children = child_file_records(record.key);
    if !children.is_empty() {
        let expanded = match record.key {
            FileKey::Src => model.src_expanded,
            FileKey::Assets => model.assets_expanded,
            _ => false,
        };
        let expansion_dispatch = dispatch.clone();
        row = row
            .list_children(
                children
                    .into_iter()
                    .map(|child| file_row(child, model, dispatch.clone())),
            )
            .expanded(expanded)
            .on_expansion_change(move |value| {
                expansion_dispatch.emit(ExplorerMessage::SetFileExpanded(record.key, value));
            });
    }
    row
}

fn file_records(model: &ExplorerComponent) -> Vec<FileRecord> {
    let mut records = vec![
        FileRecord {
            key: FileKey::Src,
            title: "src",
            modified: "Today",
            size: "—",
            kind: "Folder",
            symbol: Symbol::Folder,
        },
        FileRecord {
            key: FileKey::Assets,
            title: "assets",
            modified: "Today",
            size: "18 items",
            kind: "Folder",
            symbol: Symbol::Folder,
        },
        FileRecord {
            key: FileKey::Cargo,
            title: "Cargo.toml",
            modified: "Today, 10:42",
            size: "2.4 KB",
            kind: "TOML document",
            symbol: Symbol::Code,
        },
        FileRecord {
            key: FileKey::Readme,
            title: "README.md",
            modified: "Yesterday",
            size: "6.8 KB",
            kind: "Markdown document",
            symbol: Symbol::File,
        },
        FileRecord {
            key: FileKey::Preview,
            title: "design-preview.png",
            modified: "Today",
            size: "842 KB",
            kind: "PNG image",
            symbol: Symbol::Image,
        },
    ];
    if model.show_hidden {
        records.push(FileRecord {
            key: FileKey::HiddenEnvironment,
            title: ".env",
            modified: "Today, 09:14",
            size: "312 bytes",
            kind: "Environment file",
            symbol: Symbol::File,
        });
    }
    records.sort_by(|left, right| {
        let order = match model.sort.column_id.as_str() {
            "modified" => left.modified.cmp(right.modified),
            "size" => left.size.cmp(right.size),
            "kind" => left.kind.cmp(right.kind),
            _ => left.title.to_lowercase().cmp(&right.title.to_lowercase()),
        };
        match model.sort.direction {
            SortDirection::Ascending => order,
            SortDirection::Descending => order.reverse(),
        }
    });
    records
}

fn child_file_records(parent: FileKey) -> Vec<FileRecord> {
    match parent {
        FileKey::Src => vec![
            FileRecord {
                key: FileKey::Lib,
                title: "lib.rs",
                modified: "Today, 10:38",
                size: "9.1 KB",
                kind: "Rust source",
                symbol: Symbol::Code,
            },
            FileRecord {
                key: FileKey::Main,
                title: "main.rs",
                modified: "Today, 10:41",
                size: "3.7 KB",
                kind: "Rust source",
                symbol: Symbol::Code,
            },
        ],
        FileKey::Assets => vec![
            FileRecord {
                key: FileKey::AppIcon,
                title: "AppIcon.icon",
                modified: "Yesterday",
                size: "128 KB",
                kind: "Icon Composer document",
                symbol: Symbol::Image,
            },
            FileRecord {
                key: FileKey::PreviewAssets,
                title: "preview",
                modified: "Yesterday",
                size: "12 items",
                kind: "Folder",
                symbol: Symbol::Folder,
            },
        ],
        _ => Vec::new(),
    }
}

fn file_record_for_key(model: &ExplorerComponent, key: FileKey) -> Option<FileRecord> {
    file_records(model).into_iter().find_map(|record| {
        (record.key == key).then_some(record).or_else(|| {
            child_file_records(record.key)
                .into_iter()
                .find(|child| child.key == key)
        })
    })
}

fn inspector(model: &ExplorerComponent) -> Element {
    let detail = match model.scene {
        Scene::Ready if model.selected_file.is_some() => {
            let record = file_record_for_key(model, model.selected_file.expect("selection exists"))
                .expect("selected file must remain visible");
            column([
                label(record.title)
                    .text_role(TextRole::Heading)
                    .with_key("inspector-name"),
                label(record.kind)
                    .text_role(TextRole::Secondary)
                    .with_key("inspector-kind"),
                label(record.size)
                    .text_role(TextRole::Body)
                    .with_key("inspector-size"),
                label(format!("Modified {}", record.modified))
                    .text_role(TextRole::Body)
                    .with_key("inspector-modified"),
                spacer(false, true).with_key("inspector-space"),
                row([
                    button(
                        "Delete…",
                        format!("Delete {}", record.title),
                        announce("delete"),
                    )
                    .button_role(ButtonRole::Destructive)
                    .with_key("delete-file"),
                    spacer(true, false).with_key("inspector-action-space"),
                    button(
                        "Open in Editor",
                        format!("Open {} in editor", record.title),
                        announce("open-editor"),
                    )
                    .button_role(ButtonRole::Primary)
                    .with_key("open-editor"),
                ])
                .align(Align::Center)
                .with_key("inspector-actions"),
            ])
            .spacing(Spacing::Section)
            .with_key("inspector-ready")
        }
        Scene::Ready => inspector_status(
            "No Selection",
            "Select a file to inspect it.",
            StatusTone::Empty,
        ),
        Scene::Empty => inspector_status(
            "No Selection",
            "There are no files to inspect.",
            StatusTone::Empty,
        ),
        Scene::Busy => inspector_status(
            "Refreshing",
            "Selection details are updating.",
            StatusTone::Busy,
        ),
        Scene::Error => inspector_status(
            "Unavailable",
            "Reconnect to inspect files.",
            StatusTone::Error,
        ),
    };

    column([
        label("Inspector")
            .text_role(TextRole::Heading)
            .with_key("inspector-title"),
        separator(Axis::Horizontal).with_key("inspector-separator"),
        detail,
    ])
    .spacing(Spacing::Section)
    .padding(Spacing::Content)
    .with_key("inspector")
}

fn inspector_status(title: &str, message: &str, tone: StatusTone) -> Element {
    column([
        status(title, message, tone).with_key("inspector-state"),
        spacer(false, true).with_key("inspector-state-space"),
    ])
    .with_key("inspector-state-layout")
}

fn activity_panel() -> WindowSpec {
    WindowSpec {
        id: WindowId::new("transfer-activity"),
        title: "Connection Activity".to_owned(),
        kind: WindowKind::Panel(PanelBehavior {
            floating: true,
            hides_when_inactive: false,
            accepts_keyboard: true,
        }),
        initial_size: Size::new(380.0, 160.0),
        minimum_size: Size::new(320.0, 150.0),
        toolbar_display: ToolbarDisplay::Automatic,
        toolbar: Vec::new(),
        content: column([
            label("Refreshing Remote Project")
                .text_role(TextRole::Heading)
                .with_key("activity-file"),
            progress(0.58, "Directory refresh 58 percent").with_key("activity-progress"),
            spacer(false, true).with_key("activity-vertical-space"),
            row([
                label("Reading directory metadata")
                    .text_role(TextRole::Secondary)
                    .with_key("activity-detail"),
                spacer(true, false).with_key("activity-space"),
                button("Stop", "Stop directory refresh", announce("cancel-refresh"))
                    .with_key("cancel-transfer"),
            ])
            .align(Align::Center)
            .with_key("activity-actions"),
        ])
        .spacing(Spacing::Section)
        .padding(Spacing::Content)
        .with_key("activity-panel")
        .into(),
    }
}

fn scene_summary(model: &ExplorerComponent) -> String {
    match model.scene {
        Scene::Ready => {
            let count = file_records(model).len();
            let selected = usize::from(model.selected_file.is_some());
            format!("{count} items · {selected} selected")
        }
        Scene::Empty => "0 items".to_owned(),
        Scene::Busy => "Refreshing…".to_owned(),
        Scene::Error => "No items available".to_owned(),
    }
}

const fn connection_status(scene: Scene) -> &'static str {
    match scene {
        Scene::Ready | Scene::Empty | Scene::Busy => "Connected securely",
        Scene::Error => "Connection interrupted",
    }
}

#[cfg(test)]
mod tests {
    use super::{Scene, application};

    #[test]
    fn activity_panel_reserves_space_for_its_native_content() {
        let application = application(Scene::Busy);
        let panel = application
            .windows
            .iter()
            .find(|window| window.id.as_str() == "transfer-activity")
            .expect("busy scene must include the activity panel");

        assert_eq!(panel.initial_size.width, 380.0);
        assert_eq!(panel.initial_size.height, 160.0);
        assert_eq!(panel.minimum_size.width, 320.0);
        assert_eq!(panel.minimum_size.height, 150.0);
    }
}
