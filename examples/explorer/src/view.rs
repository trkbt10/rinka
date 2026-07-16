//! Deterministic consumer scenes shared by both native hosts.

use crate::editor::EditorState;
use rinka::{
    Accelerator, Alert, Align, ApplicationSpec, Axis, ButtonRole, CanvasColor, CanvasPoint,
    CanvasRect, CanvasSize, ClipboardError, CollectionPattern, Component, ControlSize,
    DialogButtonRole, DialogOutcome, Dispatch, DockEdge, DockEvent, DockGroup, DockLayout, DockTab,
    DragPayload, DrawScene, Element, FileDrop, FilePromise, ImageContent, ImageScaling, ImeEvent,
    InputKind, Justify, KeyChord, KeyEvent, KeyIdentity, LastWindowClosedPolicy, LineWidth,
    MenuBar, MenuBarEntry, MenuBarMenu, MenuBarMenuRole, MenuEntry, MenuItem, OpenPanelDescription,
    PanelBehavior, PointerEvent, PointerPhase, PreeditCaret, SavePanelDescription, Size,
    SortDirection, Spacing, StandardItem, StatusTone, Submenu, Symbol, TableColumn, TableSort,
    TextChange, TextRole, TextSelection, ToolbarAction, ToolbarChoice, ToolbarDisplay,
    ToolbarGroupDisplay, ToolbarItem, ToolbarPlacement, UiPattern, UpdateContext, WindowContent,
    WindowEvent, WindowId, WindowKind, WindowSpec, button, canvas, column, dock, image, input,
    label, list, list_row, mount_pattern, progress, row, separator, spacer, status, text_area,
    toggle,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

/// Serial for runtime-opened window identities.
///
/// Any explorer window can open further windows, and every open needs an
/// identity no other open in this process ever used ([`WindowId`] is the
/// window reconcile key and re-opening an open identity is a typed error),
/// so the counter is process-global and monotonic rather than per-component
/// state.
static NEXT_WINDOW_SERIAL: AtomicU32 = AtomicU32::new(1);

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
    /// Owned-drawing canvas test pattern.
    Canvas,
    /// Native multi-line text editor over a real file.
    Editor,
    /// Tabbed-document dock with user-rearrangeable splits.
    Dock,
}

impl Scene {
    /// Stable extraction identifier.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Empty => "empty",
            Self::Busy => "busy",
            Self::Error => "error",
            Self::Canvas => "canvas",
            Self::Editor => "editor",
            Self::Dock => "dock",
        }
    }

    /// Parses a command-line scene.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(Self::Ready),
            "empty" => Some(Self::Empty),
            "busy" => Some(Self::Busy),
            "error" => Some(Self::Error),
            "canvas" => Some(Self::Canvas),
            "editor" => Some(Self::Editor),
            "dock" => Some(Self::Dock),
            _ => None,
        }
    }

    /// Returns every required state in deterministic order.
    pub const fn all() -> [Self; 7] {
        [
            Self::Ready,
            Self::Empty,
            Self::Busy,
            Self::Error,
            Self::Canvas,
            Self::Editor,
            Self::Dock,
        ]
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
    const ALL: [Self; 4] = [
        Self::Home,
        Self::Documents,
        Self::Downloads,
        Self::RemoteProject,
    ];

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

/// Typed payload identifier for a file row dragged within the explorer.
const EXPLORER_FILE_PAYLOAD_TYPE: &str = "jp.bunko.rinka.explorer.file";

/// Returns whether the explorer attaches its drag-and-drop declarations.
///
/// The escape hatch exists for the accessibility-equivalence evidence of
/// `reports/drag-and-drop`: extracting the AX tree with and without drag
/// modifiers from otherwise identical processes.
fn drag_interactions_enabled() -> bool {
    std::env::var_os("RINKA_EXPLORER_DISABLE_DRAG").is_none()
}

struct ExplorerComponent {
    /// This window's stable identity — the value every window service call
    /// (close, confirm_close, veto_close) addresses.
    window_id: WindowId,
    /// Whether this component runs in a runtime-opened secondary window,
    /// which declares its reconciled title from scene state.
    secondary: bool,
    scene: Scene,
    location: Location,
    selected_file: Option<FileKey>,
    clipboard_note: Option<String>,
    drag_note: Option<String>,
    show_hidden: bool,
    file_filter: String,
    favorites_expanded: bool,
    locations_expanded: bool,
    src_expanded: bool,
    assets_expanded: bool,
    sort: TableSort,
    canvas_pointer: Option<PointerEvent>,
    canvas_focused: bool,
    canvas_echo: String,
    canvas_preedit: Option<(String, Option<PreeditCaret>)>,
    canvas_last_key: Option<KeyEvent>,
    preview_bitmaps: Vec<(FileKey, ImageContent)>,
    scaling_probe: ImageContent,
    deleted: Vec<FileKey>,
    uploads: Vec<PathBuf>,
    download_target: Option<PathBuf>,
    duplicated: Vec<FileKey>,
    favorite_files: Vec<FileKey>,
    last_file_action: Option<String>,
    window_note: Option<String>,
    editor: EditorState,
    dock_layout: DockLayout,
    dock_saved: Option<String>,
    dock_note: Option<String>,
    dock_next_group: u32,
}

impl ExplorerComponent {
    fn new(scene: Scene) -> Self {
        Self::for_window(scene, WindowId::new("explorer-main"), false)
    }

    /// Builds the component of a runtime-opened secondary window.
    fn secondary(scene: Scene, serial: u32) -> Self {
        Self::for_window(
            scene,
            WindowId::new(format!("explorer-secondary-{serial}")),
            true,
        )
    }

    fn for_window(scene: Scene, window_id: WindowId, secondary: bool) -> Self {
        // Deterministic capture aid: preselecting the generated PNG preview
        // lets the visual matrix photograph the inspector bitmap without
        // synthetic input, following the RINKA_APPKIT_CONTENT_FIT_PROBE
        // precedent.
        let preselect_preview = std::env::var_os("RINKA_EXPLORER_SELECT_PREVIEW").is_some();
        Self {
            window_id,
            secondary,
            scene,
            location: Location::RemoteProject,
            selected_file: (scene == Scene::Ready).then_some(if preselect_preview {
                FileKey::Preview
            } else {
                FileKey::Cargo
            }),
            clipboard_note: None,
            drag_note: None,
            show_hidden: false,
            file_filter: String::new(),
            favorites_expanded: true,
            locations_expanded: true,
            src_expanded: false,
            assets_expanded: false,
            sort: TableSort {
                column_id: "name".to_owned(),
                direction: SortDirection::Ascending,
            },
            canvas_pointer: None,
            canvas_focused: false,
            canvas_echo: String::new(),
            canvas_preedit: None,
            canvas_last_key: None,
            // Generated once so every reconcile hands the runtime the same
            // shared buffers under the same revision, exercising the
            // "identical revision means no re-upload" contract.
            preview_bitmaps: vec![
                (FileKey::Preview, preview_bitmap(FileKey::Preview)),
                (FileKey::AppIcon, preview_bitmap(FileKey::AppIcon)),
            ],
            scaling_probe: scaling_probe_bitmap(),
            deleted: Vec::new(),
            uploads: Vec::new(),
            download_target: None,
            duplicated: Vec::new(),
            favorite_files: Vec::new(),
            last_file_action: None,
            window_note: None,
            editor: EditorState::load(),
            dock_layout: initial_dock_layout(),
            dock_saved: None,
            dock_note: None,
            dock_next_group: 0,
        }
    }

    fn preview_content(&self, key: FileKey) -> Option<&ImageContent> {
        self.preview_bitmaps
            .iter()
            .find_map(|(candidate, content)| (*candidate == key).then_some(content))
    }

    /// Applies one semantic dock request from a native gesture. Every
    /// mutation goes through the layout's standard semantics; a dirty tab's
    /// close is vetoed pending an explicit dialog answer.
    fn apply_dock_event(&mut self, event: DockEvent, context: &UpdateContext<ExplorerMessage>) {
        match event {
            DockEvent::SelectTab { tab, .. } => {
                self.dock_layout.select_tab(&tab);
                self.dock_note = Some(format!("dock: selected {tab}"));
            }
            DockEvent::CloseTab { tab, .. } => {
                if self.dock_layout.tab(&tab).is_some_and(|tab| tab.dirty) {
                    let title = self
                        .dock_layout
                        .tab(&tab)
                        .map(|tab| tab.title.clone())
                        .unwrap_or_default();
                    self.dock_note = Some(format!("dock: close requested for dirty {tab}"));
                    context.dialogs().alert(
                        Alert::new(
                            format!("Close \u{201c}{title}\u{201d} without saving?"),
                            "Unsaved changes will be lost.",
                        )
                        .button(
                            "Cancel",
                            DialogButtonRole::Cancel,
                            ExplorerMessage::DockCloseCancelled,
                        )
                        .button(
                            "Close",
                            DialogButtonRole::Destructive,
                            ExplorerMessage::DockCloseConfirmed(tab),
                        )
                        .default_button(0),
                    );
                } else {
                    self.dock_layout.close_tab(&tab);
                    self.dock_note = Some(format!("dock: closed {tab}"));
                }
            }
            DockEvent::MoveTab {
                tab,
                to_group,
                index,
                ..
            } => {
                if self.dock_layout.move_tab(&tab, &to_group, index) {
                    self.dock_note = Some(format!("dock: moved {tab} to {to_group}@{index}"));
                }
            }
            DockEvent::SplitGroup {
                tab,
                target_group,
                edge,
                ..
            } => {
                self.dock_next_group += 1;
                let new_group = format!("group-{}", self.dock_next_group);
                if self
                    .dock_layout
                    .split_with_tab(&target_group, edge, &new_group, &tab)
                {
                    self.dock_note =
                        Some(format!("dock: split {target_group} {edge:?} with {tab}"));
                }
            }
        }
    }
}

/// Stable tab id of the editor document in the dock scene.
const DOCK_TAB_EDITOR: &str = "editor";
/// Stable tab id of the canvas document in the dock scene.
const DOCK_TAB_CANVAS: &str = "canvas";
/// Stable tab id of the notes document in the dock scene.
const DOCK_TAB_NOTES: &str = "notes";

/// The dock scene's initial layout: one group hosting the editor scene
/// content, the canvas scene content, and a dirty notes document.
fn initial_dock_layout() -> DockLayout {
    DockLayout::single_group(DockGroup::new(
        "documents",
        [
            DockTab::new(DOCK_TAB_EDITOR, "view.rs"),
            DockTab::new(DOCK_TAB_CANVAS, "Test Pattern"),
            DockTab::new(DOCK_TAB_NOTES, "notes.md").dirty(true),
        ],
        DOCK_TAB_EDITOR,
    ))
}

/// Pixel density of every generated preview bitmap: two pixels per point,
/// proving the HiDPI mapping on a Retina display.
const PREVIEW_BITMAP_SCALE: f64 = 2.0;

/// Builds a deterministic preview picture for one image file: a per-file
/// color gradient under a checker pattern, with straight-alpha transparent
/// corners so the window background composites through.
///
/// The revision is a per-file constant because each file's generated
/// picture never changes; switching files patches the mounted element with
/// a different revision and geometry-identical buffers re-use the retained
/// native image.
fn preview_bitmap(key: FileKey) -> ImageContent {
    // Sized so the complete inspector detail column, including the four
    // scaling probes, fits the pane without overconstraining Auto Layout.
    let logical = 64_u32;
    let side = (f64::from(logical) * PREVIEW_BITMAP_SCALE) as u32;
    let stride = side * 4;
    let (base, revision) = match key {
        FileKey::AppIcon => ([0xC5_u8, 0x63, 0x2A], 2_u64),
        _ => ([0x2E_u8, 0x6F, 0xC1], 1_u64),
    };
    let mut bytes = Vec::with_capacity((stride * side) as usize);
    let center = f64::from(side) / 2.0;
    for y in 0..side {
        for x in 0..side {
            let ramp = f64::from(x) / f64::from(side.max(1));
            let checker = ((x / 16) + (y / 16)) % 2 == 0;
            let lift = if checker { 0.18 } else { 0.0 };
            let channel = |value: u8| {
                let base = f64::from(value) / 255.0;
                let mixed = base + (1.0 - base) * (ramp * 0.6 + lift);
                (mixed * 255.0).round().clamp(0.0, 255.0) as u8
            };
            // Straight alpha: opaque center falling off to transparent
            // corners, so compositing over the window background is visible.
            let distance =
                ((f64::from(x) - center).powi(2) + (f64::from(y) - center).powi(2)).sqrt() / center;
            let alpha = ((1.35 - distance) * 255.0).round().clamp(0.0, 255.0) as u8;
            bytes.extend_from_slice(&[channel(base[0]), channel(base[1]), channel(base[2]), alpha]);
        }
    }
    ImageContent::from_rgba8(side, side, stride, bytes, revision).with_scale(PREVIEW_BITMAP_SCALE)
}

/// Builds the wide deterministic ribbon used to verify all four scaling
/// modes: a horizontal hue ramp with fixed-size tick marks, wider than the
/// inspector, so fit letterboxes, fill distorts, and actual versus center
/// crop different regions.
fn scaling_probe_bitmap() -> ImageContent {
    let logical_width = 280_u32;
    let logical_height = 24_u32;
    let width = (f64::from(logical_width) * PREVIEW_BITMAP_SCALE) as u32;
    let height = (f64::from(logical_height) * PREVIEW_BITMAP_SCALE) as u32;
    let stride = width * 4;
    let mut bytes = Vec::with_capacity((stride * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let ramp = f64::from(x) / f64::from(width.max(1));
            let tick = x % 80 < 4 || y % 24 < 4;
            let (red, green, blue) = if tick {
                (0x20, 0x20, 0x20)
            } else {
                (
                    (ramp * 255.0).round() as u8,
                    0x60,
                    ((1.0 - ramp) * 255.0).round() as u8,
                )
            };
            bytes.extend_from_slice(&[red, green, blue, 0xFF]);
        }
    }
    ImageContent::from_rgba8(width, height, stride, bytes, 3).with_scale(PREVIEW_BITMAP_SCALE)
}

enum ExplorerMessage {
    SelectLocation(Location),
    SelectFile(FileKey),
    SetScene(Scene),
    NewWindow,
    CloseThisWindow,
    WindowObserved(WindowEvent),
    WindowCloseRequested,
    WindowCloseConfirmed,
    WindowCloseVetoed,
    SetShowHidden(bool),
    SetFileFilter(String),
    NewFolder,
    ShowHelp,
    SetSort(TableSort),
    SetSectionExpanded(&'static str, bool),
    SetFileExpanded(FileKey, bool),
    CanvasPointer(PointerEvent),
    ConfirmDelete(FileKey),
    DeleteConfirmed(FileKey),
    DeleteCancelled,
    RequestUpload,
    UploadsChosen(Vec<PathBuf>),
    RequestDownload(FileKey),
    DownloadTargetChosen(PathBuf),
    CanvasFocus(bool),
    CanvasKey(KeyEvent),
    CanvasIme(ImeEvent),
    RenameFile(FileKey),
    DuplicateFile(FileKey),
    DeleteFile(FileKey),
    ToggleFavoriteFile(FileKey),
    OpenFileWith(FileKey, &'static str),
    CopyPath,
    PastePath,
    ClipboardRead(Result<Option<String>, ClipboardError>),
    EditorChanged(TextChange),
    EditorSelectionChanged(TextSelection),
    EditorSetReadOnly(bool),
    EditorJumpEnd,
    EditorRehighlight,
    EditorReload,
    FilesDropped(FileDrop),
    FileExported(Result<String, String>),
    MoveFileToLocation(Location, String),
    Dock(DockEvent),
    DockCloseConfirmed(String),
    DockCloseCancelled,
    DockSplitActive(DockEdge),
    DockMarkActiveDirty,
    DockSaveLayout,
    DockRestoreLayout,
    DockCloseOthers(String),
    DockCloseToTheRight(String),
}

impl Component for ExplorerComponent {
    type Message = ExplorerMessage;

    fn update(&mut self, message: Self::Message, context: &UpdateContext<Self::Message>) {
        match message {
            ExplorerMessage::SelectLocation(location) => {
                self.location = location;
                self.selected_file = (location == Location::RemoteProject
                    && self.scene == Scene::Ready)
                    .then_some(FileKey::Cargo);
            }
            ExplorerMessage::SelectFile(file) => self.selected_file = Some(file),
            ExplorerMessage::SetScene(scene) => {
                self.scene = scene;
                self.selected_file = (scene == Scene::Ready
                    && self.location == Location::RemoteProject)
                    .then_some(FileKey::Cargo);
            }
            ExplorerMessage::SetShowHidden(value) => {
                self.show_hidden = value;
                if !value && self.selected_file == Some(FileKey::HiddenEnvironment) {
                    self.selected_file = None;
                }
            }
            ExplorerMessage::SetFileFilter(filter) => {
                self.file_filter = filter;
                // The inspector requires the selected file to stay visible;
                // a selection the filter hides is released, exactly like the
                // hidden-files toggle above.
                if let Some(selected) = self.selected_file
                    && file_record_for_key(self, selected).is_none()
                {
                    self.selected_file = None;
                }
            }
            ExplorerMessage::NewWindow => {
                let serial = NEXT_WINDOW_SERIAL.fetch_add(1, Ordering::Relaxed);
                context.windows().open(secondary_window(self.scene, serial));
                self.last_file_action = Some(format!("Opened window explorer-secondary-{serial}"));
            }
            ExplorerMessage::CloseThisWindow => {
                // The imperative close path: unconditional, bypassing the
                // close-request interception (which governs user gestures).
                context.windows().close(&self.window_id);
            }
            ExplorerMessage::WindowObserved(event) => {
                let observed = match event {
                    WindowEvent::Focused => Some("focused"),
                    WindowEvent::Resigned => Some("resigned"),
                    WindowEvent::Resized(_) | WindowEvent::Moved(_) => None,
                };
                if let Some(observed) = observed {
                    self.window_note =
                        Some(format!("window {observed}: {}", self.window_id.as_str()));
                }
            }
            ExplorerMessage::WindowCloseRequested => {
                if self.scene == Scene::Editor {
                    // The dirty-ish state: an editor session lives in this
                    // window, so the deferred close is answered only after
                    // an explicit decision through the confirm sheet.
                    context.dialogs().alert(
                        Alert::new(
                            format!("Close \u{201c}{}\u{201d}?", self.editor.file_name()),
                            "The editor session in this window will end and unsaved \
                             changes will be discarded.",
                        )
                        .button(
                            "Cancel",
                            DialogButtonRole::Cancel,
                            ExplorerMessage::WindowCloseVetoed,
                        )
                        .button(
                            "Close",
                            DialogButtonRole::Destructive,
                            ExplorerMessage::WindowCloseConfirmed,
                        )
                        .default_button(0),
                    );
                } else {
                    // The interception is declared per-state; a request that
                    // arrives as the scene leaves the editor is honored.
                    context.windows().confirm_close(&self.window_id);
                }
            }
            ExplorerMessage::WindowCloseConfirmed => {
                context.windows().confirm_close(&self.window_id);
            }
            ExplorerMessage::WindowCloseVetoed => {
                context.windows().veto_close(&self.window_id);
            }
            ExplorerMessage::NewFolder => {
                self.last_file_action =
                    Some(format!("New Folder created in {}", self.location.title()));
            }
            ExplorerMessage::ShowHelp => {
                self.last_file_action = Some("Rinka Explorer Help requested".to_owned());
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
            ExplorerMessage::CanvasPointer(event) => self.canvas_pointer = Some(event),
            ExplorerMessage::CanvasFocus(focused) => self.canvas_focused = focused,
            ExplorerMessage::CanvasKey(event) => {
                if event.key == Some(KeyIdentity::BACKSPACE) {
                    self.canvas_echo.pop();
                } else if let Some(text) = &event.text {
                    self.canvas_echo.push_str(text);
                    trim_echo_line(&mut self.canvas_echo);
                }
                self.canvas_last_key = Some(event);
            }
            ExplorerMessage::CanvasIme(event) => match event {
                ImeEvent::Preedit { text, caret } => {
                    self.canvas_preedit = Some((text, caret));
                }
                ImeEvent::Commit { text } => {
                    self.canvas_echo.push_str(&text);
                    trim_echo_line(&mut self.canvas_echo);
                    self.canvas_preedit = None;
                }
                ImeEvent::Cancel => self.canvas_preedit = None,
            },
            ExplorerMessage::RenameFile(file) => {
                self.last_file_action = Some(format!("Rename requested for {}", file_title(file)));
            }
            ExplorerMessage::ConfirmDelete(key) => {
                let Some(record) = file_record_for_key(self, key) else {
                    return;
                };
                // The consumer's iron rule "destructive stays destructive":
                // Delete carries the destructive role and Cancel receives the
                // return-key default, so the safe answer is the easy answer.
                // Button 0 occupies the platform's primary position (the
                // rightmost slot on macOS): Cancel sits there with the
                // return key, and Delete keeps the destructive treatment
                // away from both.
                context.dialogs().alert(
                    Alert::new(
                        format!("Delete “{}”?", record.title),
                        format!(
                            "“{}” will be deleted from {} immediately. This cannot be undone.",
                            record.title,
                            self.location.title()
                        ),
                    )
                    .button(
                        "Cancel",
                        DialogButtonRole::Cancel,
                        ExplorerMessage::DeleteCancelled,
                    )
                    .button(
                        "Delete",
                        DialogButtonRole::Destructive,
                        ExplorerMessage::DeleteConfirmed(key),
                    )
                    .default_button(0),
                );
            }
            ExplorerMessage::DuplicateFile(file) => {
                if !self.duplicated.contains(&file) {
                    self.duplicated.push(file);
                }
                self.last_file_action = Some(format!("Duplicated {}", file_title(file)));
            }
            ExplorerMessage::DeleteFile(file) => {
                if !self.deleted.contains(&file) {
                    self.deleted.push(file);
                }
                if self.selected_file == Some(file) {
                    self.selected_file = None;
                }
                self.last_file_action = Some(format!("Deleted {}", file_title(file)));
            }
            ExplorerMessage::ToggleFavoriteFile(file) => {
                if let Some(index) = self
                    .favorite_files
                    .iter()
                    .position(|favorite| *favorite == file)
                {
                    self.favorite_files.remove(index);
                    self.last_file_action =
                        Some(format!("Removed {} from favorites", file_title(file)));
                } else {
                    self.favorite_files.push(file);
                    self.last_file_action = Some(format!("Favorited {}", file_title(file)));
                }
            }
            ExplorerMessage::OpenFileWith(file, tool) => {
                self.last_file_action = Some(format!("Opened {} in {tool}", file_title(file)));
            }
            ExplorerMessage::CopyPath => {
                let path = self.location.path();
                self.clipboard_note = Some(match context.clipboard().write_text(path) {
                    Ok(()) => format!("Copied {path}"),
                    Err(error) => format!("Copy failed: {error}"),
                });
            }
            ExplorerMessage::PastePath => {
                // The outcome always returns as a message: synchronous
                // platforms deliver before read_text returns and the runtime
                // queues the emission until this update finishes.
                let dispatch = context.dispatch().clone();
                context.clipboard().read_text(move |result| {
                    dispatch.emit(ExplorerMessage::ClipboardRead(result));
                });
            }
            ExplorerMessage::FilesDropped(drop) => {
                let names = drop
                    .paths
                    .iter()
                    .map(|path| {
                        path.file_name().map_or_else(
                            || path.display().to_string(),
                            |name| name.to_string_lossy().into_owned(),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                self.drag_note = Some(format!(
                    "Dropped {} file(s) at ({:.0}, {:.0}): {names}",
                    drop.paths.len(),
                    drop.position.x,
                    drop.position.y,
                ));
            }
            ExplorerMessage::FileExported(result) => {
                self.drag_note = Some(match result {
                    Ok(file_name) => format!("Exported {file_name}"),
                    Err(reason) => format!("Export failed: {reason}"),
                });
            }
            ExplorerMessage::MoveFileToLocation(location, file) => {
                self.drag_note = Some(format!("Moved {file} to {}", location.title()));
            }
            ExplorerMessage::ClipboardRead(result) => {
                self.clipboard_note = Some(match result {
                    Ok(Some(text)) => match location_for_path(text.trim()) {
                        Some(location) => {
                            self.location = location;
                            self.selected_file = (location == Location::RemoteProject
                                && self.scene == Scene::Ready)
                                .then_some(FileKey::Cargo);
                            format!("Went to {}", location.title())
                        }
                        None => format!("Clipboard: {text}"),
                    },
                    Ok(None) => "Clipboard has no text".to_owned(),
                    Err(error) => format!("Clipboard error: {error}"),
                });
            }
            ExplorerMessage::EditorChanged(change) => {
                self.editor.apply_change(&change);
                // The dock's dirty indicator follows real editor edits.
                self.dock_layout.set_dirty(DOCK_TAB_EDITOR, true);
            }
            ExplorerMessage::EditorSelectionChanged(selection) => {
                self.editor.store_selection(selection);
            }
            ExplorerMessage::DeleteConfirmed(key) => {
                if !self.deleted.contains(&key) {
                    self.deleted.push(key);
                }
                if self.selected_file == Some(key) {
                    self.selected_file = None;
                }
                self.last_file_action = Some(format!("Deleted {}", file_title(key)));
            }
            ExplorerMessage::DeleteCancelled => {}
            ExplorerMessage::RequestUpload => {
                context.dialogs().open_panel(
                    OpenPanelDescription {
                        title: Some("Choose files or folders to upload".to_owned()),
                        choose_files: true,
                        choose_directories: true,
                        allows_multiple: true,
                        starting_directory: panel_starting_directory(),
                    },
                    |outcome| match outcome {
                        DialogOutcome::PathsChosen(paths) => {
                            Some(ExplorerMessage::UploadsChosen(paths))
                        }
                        _ => None,
                    },
                );
            }
            ExplorerMessage::UploadsChosen(paths) => self.uploads = paths,
            ExplorerMessage::RequestDownload(key) => {
                let Some(record) = file_record_for_key(self, key) else {
                    return;
                };
                context.dialogs().save_panel(
                    SavePanelDescription {
                        title: Some(format!("Choose where to download “{}”", record.title)),
                        suggested_filename: Some(record.title.to_owned()),
                        starting_directory: panel_starting_directory(),
                    },
                    |outcome| match outcome {
                        DialogOutcome::SavePathChosen(path) => {
                            Some(ExplorerMessage::DownloadTargetChosen(path))
                        }
                        _ => None,
                    },
                );
            }
            ExplorerMessage::DownloadTargetChosen(path) => self.download_target = Some(path),
            ExplorerMessage::EditorSetReadOnly(read_only) => self.editor.set_read_only(read_only),
            ExplorerMessage::EditorJumpEnd => self.editor.jump_to_end(),
            ExplorerMessage::EditorRehighlight => self.editor.rehighlight_all(),
            ExplorerMessage::EditorReload => {
                self.editor.reload();
                self.dock_layout.set_dirty(DOCK_TAB_EDITOR, false);
            }
            ExplorerMessage::Dock(event) => self.apply_dock_event(event, context),
            ExplorerMessage::DockCloseConfirmed(tab) => {
                self.dock_layout.close_tab(&tab);
                self.dock_note = Some(format!("dock: closed {tab}"));
            }
            ExplorerMessage::DockCloseCancelled => {
                self.dock_note = Some("dock: close cancelled".to_owned());
            }
            ExplorerMessage::DockSplitActive(edge) => {
                let Some((group_id, active)) = self
                    .dock_layout
                    .groups()
                    .first()
                    .map(|group| (group.id.clone(), group.active.clone()))
                else {
                    return;
                };
                self.dock_next_group += 1;
                let new_group = format!("group-{}", self.dock_next_group);
                if self
                    .dock_layout
                    .split_with_tab(&group_id, edge, &new_group, &active)
                {
                    self.dock_note = Some(format!("dock: split {group_id} with {active}"));
                } else {
                    self.dock_note = Some("dock: split refused".to_owned());
                }
            }
            ExplorerMessage::DockMarkActiveDirty => {
                let Some(active) = self
                    .dock_layout
                    .groups()
                    .first()
                    .map(|group| group.active.clone())
                else {
                    return;
                };
                let dirty = self.dock_layout.tab(&active).is_some_and(|tab| !tab.dirty);
                self.dock_layout.set_dirty(&active, dirty);
                self.dock_note = Some(format!("dock: {active} dirty={dirty}"));
            }
            ExplorerMessage::DockSaveLayout => {
                let persisted = self.dock_layout.to_persisted();
                self.dock_note = Some(format!("dock: saved layout ({} bytes)", persisted.len()));
                self.dock_saved = Some(persisted);
            }
            ExplorerMessage::DockRestoreLayout => {
                let Some(saved) = &self.dock_saved else {
                    self.dock_note = Some("dock: nothing saved".to_owned());
                    return;
                };
                match DockLayout::from_persisted(saved) {
                    Ok(layout) => {
                        self.dock_layout = layout;
                        self.dock_note = Some("dock: restored layout".to_owned());
                    }
                    Err(reason) => {
                        self.dock_note = Some(format!("dock: restore failed: {reason}"));
                    }
                }
            }
            ExplorerMessage::DockCloseOthers(tab) => {
                let others: Vec<String> = self
                    .dock_layout
                    .group_of_tab(&tab)
                    .map(|group| {
                        group
                            .tabs
                            .iter()
                            .filter(|candidate| candidate.id != tab)
                            .map(|candidate| candidate.id.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for other in others {
                    self.dock_layout.close_tab(&other);
                }
                self.dock_note = Some(format!("dock: closed others of {tab}"));
            }
            ExplorerMessage::DockCloseToTheRight(tab) => {
                let rightward: Vec<String> = self
                    .dock_layout
                    .group_of_tab(&tab)
                    .map(|group| {
                        group
                            .tabs
                            .iter()
                            .skip_while(|candidate| candidate.id != tab)
                            .skip(1)
                            .map(|candidate| candidate.id.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for other in rightward {
                    self.dock_layout.close_tab(&other);
                }
                self.dock_note = Some(format!("dock: closed to the right of {tab}"));
            }
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        explorer_content(self, dispatch)
    }
}

fn announce(action: &'static str) -> impl Fn() {
    move || eprintln!("action={action}")
}

const fn file_title(key: FileKey) -> &'static str {
    match key {
        FileKey::Src => "src",
        FileKey::Lib => "lib.rs",
        FileKey::Main => "main.rs",
        FileKey::Assets => "assets",
        FileKey::AppIcon => "AppIcon.icon",
        FileKey::PreviewAssets => "preview",
        FileKey::Cargo => "Cargo.toml",
        FileKey::Readme => "README.md",
        FileKey::Preview => "design-preview.png",
        FileKey::HiddenEnvironment => ".env",
    }
}

/// Reads the deterministic panel starting directory for probe runs.
fn panel_starting_directory() -> Option<PathBuf> {
    std::env::var_os("RINKA_EXPLORER_PANEL_DIR").map(PathBuf::from)
}

/// Resolves clipboard text back to a known location for paste navigation.
fn location_for_path(path: &str) -> Option<Location> {
    Location::ALL
        .into_iter()
        .find(|location| location.path() == path)
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
        // The live menu bar is declared by the main window's component root
        // (see `explorer_menu_bar`), so its checkmarks and enabled state
        // reconcile with component state; the application-level slot stays
        // empty and the router falls back to the main window's declaration
        // whenever a menu-less window (the activity panel) is focused.
        menu_bar: MenuBar::default(),
        windows,
        // The explorer keeps its historical behavior: closing the last
        // window exits, which is also what the probe suite's native-close
        // finish paths rely on. Declared explicitly rather than through the
        // macOS platform default (which keeps the application running).
        last_window_closed: LastWindowClosedPolicy::Exit,
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
            explorer_toolbar()
        },
        content: WindowContent::component(ExplorerComponent::new(scene)),
    }
}

/// The explorer's native toolbar, shared by the main window and every
/// runtime-opened window.
fn explorer_toolbar() -> Vec<ToolbarItem> {
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
                MenuEntry::item(
                    MenuItem::new("sort-name", "Name", announce("sort-name"))
                        .symbol(Symbol::List)
                        .help("Sort by name")
                        .chord(shortcut("Primary+Shift+N")),
                ),
                MenuEntry::item(
                    MenuItem::new("sort-modified", "Date Modified", announce("sort-modified"))
                        .symbol(Symbol::Refresh)
                        .help("Sort by modification date"),
                ),
                MenuEntry::separator(),
                MenuEntry::item(
                    MenuItem::new("group-kind", "Group by Kind", announce("group-kind"))
                        .symbol(Symbol::Columns)
                        .help("Group files by kind"),
                ),
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
}

/// Builds one runtime-opened explorer window: a full explorer (content,
/// toolbar, menu bar) whose component starts in the opener's scene.
fn secondary_window(scene: Scene, serial: u32) -> WindowSpec {
    WindowSpec {
        id: WindowId::new(format!("explorer-secondary-{serial}")),
        // Superseded on the first render by the component's reconciled
        // window_title declaration.
        title: "Rinka Explorer".to_owned(),
        kind: WindowKind::Main,
        initial_size: Size::new(1120.0, 720.0),
        minimum_size: Size::new(760.0, 520.0),
        toolbar_display: ToolbarDisplay::IconOnly,
        toolbar: explorer_toolbar(),
        content: WindowContent::component(ExplorerComponent::secondary(scene, serial)),
    }
}

fn explorer_content(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    let mut root = mount_pattern(
        UiPattern::NavigationWorkspace {
            sidebar_collapsible: true,
            inspector_collapsible: true,
        },
        [
            sidebar(model, dispatch.clone()),
            directory_content(model, dispatch.clone()),
            inspector(model, dispatch.clone()),
        ],
    )
    .with_key("explorer-workspace")
    .accelerators(explorer_accelerators(model, dispatch.clone()));
    // The menu bar and the window lifecycle surface are declared only where
    // a host realizes them: the AppKit host installs the bar as
    // NSApplication.mainMenu and delivers window events through its window
    // delegate, while the GTK and WinUI hosts currently reject these
    // declarations with typed diagnostics (`reports/app-menu-bar`,
    // `reports/dynamic-window-management`).
    if cfg!(target_os = "macos") {
        let observe = dispatch.clone();
        root = root.on_window_event(move |event| {
            // Only focus transitions become messages: resize and move
            // events arrive continuously during native drags, and this
            // consumer has no state derived from them, so it does not pay
            // an update-render cycle per geometry tick.
            if matches!(event, WindowEvent::Focused | WindowEvent::Resigned) {
                observe.emit(ExplorerMessage::WindowObserved(event));
            }
        });
        if model.scene == Scene::Editor {
            // Close interception is reconciled state: only a window holding
            // a live editor session intercepts its close; every other scene
            // keeps the fully native close path.
            let close_requested = dispatch.clone();
            root = root.on_close_request(move || {
                close_requested.emit(ExplorerMessage::WindowCloseRequested);
            });
        }
        if model.secondary {
            // Runtime-opened windows title themselves from their own scene
            // state; the reconciled declaration retitles the native window
            // live as the scene changes.
            root = root.window_title(format!(
                "Rinka Explorer \u{2014} {}",
                scene_title(model.scene)
            ));
        }
        root.menu_bar(explorer_menu_bar(model, dispatch))
    } else {
        root
    }
}

/// Human-readable scene name used by the secondary windows' live titles.
const fn scene_title(scene: Scene) -> &'static str {
    match scene {
        Scene::Ready => "Ready",
        Scene::Empty => "Empty",
        Scene::Busy => "Busy",
        Scene::Error => "Error",
        Scene::Canvas => "Canvas",
        Scene::Editor => "Editor",
        Scene::Dock => "Dock",
    }
}

fn shortcut(text: &'static str) -> KeyChord {
    text.parse().expect("explorer chords are canonical")
}

/// The explorer's application menu bar, reconciled with component state.
///
/// The scene switchers carry the same chords as the accelerator table below;
/// those chords are menu-owned on macOS (the key monitor defers them to
/// native menu dispatch), so each fires exactly once, through the menu.
/// "Show Hidden Files" deliberately displays no chord: its Primary+Shift+H
/// stays table-owned so it keeps the defer-to-typing policy a native menu
/// key equivalent cannot express.
fn explorer_menu_bar(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> MenuBar {
    let new_window = dispatch.clone();
    let new_folder = dispatch.clone();
    let show_hidden = model.show_hidden;
    let hidden = dispatch.clone();
    let help = dispatch.clone();
    let scene_item = |id: &'static str, label: &'static str, scene: Scene, chord| {
        let switch = dispatch.clone();
        let mut item = MenuItem::new(id, label, move || {
            switch.emit(ExplorerMessage::SetScene(scene));
        })
        .help(format!("Show the {label} scene"))
        .checked(model.scene == scene);
        if let Some(chord) = chord {
            item = item.chord(shortcut(chord));
        }
        MenuBarEntry::item(item)
    };
    MenuBar::new([
        MenuBarMenu::new(
            "file",
            "File",
            [
                MenuBarEntry::item(
                    MenuItem::new("file-new-window", "New Window", move || {
                        new_window.emit(ExplorerMessage::NewWindow);
                    })
                    .help("Open another explorer window")
                    // Primary+N belongs to New Window, the platform's
                    // new-window convention (Overshell's R10 shape); New
                    // Folder keeps a distinct chord below.
                    .chord(shortcut("Primary+N")),
                ),
                MenuBarEntry::item(
                    MenuItem::new("file-new-folder", "New Folder", move || {
                        new_folder.emit(ExplorerMessage::NewFolder);
                    })
                    .help("Create a folder in the current location")
                    // Folders can only be created while the listing is live.
                    .enabled(model.scene == Scene::Ready)
                    .chord(shortcut("Primary+Alt+N")),
                ),
                MenuBarEntry::separator(),
                MenuBarEntry::standard(StandardItem::CloseWindow),
            ],
        ),
        MenuBarMenu::new(
            "edit",
            "Edit",
            [
                MenuBarEntry::standard(StandardItem::Undo),
                MenuBarEntry::standard(StandardItem::Redo),
                MenuBarEntry::separator(),
                MenuBarEntry::standard(StandardItem::Cut),
                MenuBarEntry::standard(StandardItem::Copy),
                MenuBarEntry::standard(StandardItem::Paste),
                MenuBarEntry::separator(),
                MenuBarEntry::standard(StandardItem::SelectAll),
            ],
        ),
        MenuBarMenu::new(
            "view",
            "View",
            [
                scene_item("view-scene-ready", "Ready", Scene::Ready, Some("Primary+1")),
                scene_item("view-scene-empty", "Empty", Scene::Empty, Some("Primary+2")),
                scene_item("view-scene-busy", "Busy", Scene::Busy, None),
                scene_item("view-scene-error", "Error", Scene::Error, Some("Primary+3")),
                scene_item("view-scene-canvas", "Canvas", Scene::Canvas, None),
                MenuBarEntry::separator(),
                MenuBarEntry::item(
                    MenuItem::new("view-show-hidden", "Show Hidden Files", move || {
                        hidden.emit(ExplorerMessage::SetShowHidden(!show_hidden));
                    })
                    .help("Show files whose names begin with a dot")
                    .checked(show_hidden),
                ),
            ],
        ),
        MenuBarMenu::new(
            "window",
            "Window",
            [MenuBarEntry::standard(StandardItem::Minimize)],
        )
        .role(MenuBarMenuRole::Window),
        MenuBarMenu::new(
            "help",
            "Help",
            [MenuBarEntry::item(
                MenuItem::new("help-explorer", "Rinka Explorer Help", move || {
                    help.emit(ExplorerMessage::ShowHelp);
                })
                .help("Show the explorer help"),
            )],
        )
        .role(MenuBarMenuRole::Help),
    ])
}

/// The explorer's keyboard shortcuts, reconciled with the component state.
///
/// The table is declared only where a host delivers it: the AppKit host
/// dispatches through its application key monitor, while the GTK and WinUI
/// hosts currently reject a declared table with a typed diagnostic
/// (`reports/keyboard-shortcuts-and-key-events`).
fn explorer_accelerators(
    model: &ExplorerComponent,
    dispatch: Dispatch<ExplorerMessage>,
) -> Vec<Accelerator> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let ready = dispatch.clone();
    let empty = dispatch.clone();
    let error = dispatch.clone();
    let editor = dispatch.clone();
    let new_window = dispatch.clone();
    let close_now = dispatch.clone();
    let hidden = dispatch.clone();
    let show_hidden = model.show_hidden;
    vec![
        // The three scene chords are also declared on the View menu items,
        // which own them on macOS: the key monitor defers a menu-claimed
        // chord to native menu dispatch, so these entries are shadowed there
        // (their global/withhold flags included) and each chord fires exactly
        // once, through the menu. The entries stay declared as the delivery
        // path for hosts without a realized menu bar.
        Accelerator::new("scene-ready", shortcut("Primary+1"), move || {
            ready.emit(ExplorerMessage::SetScene(Scene::Ready));
        })
        .global(true),
        Accelerator::new("scene-empty", shortcut("Primary+2"), move || {
            empty.emit(ExplorerMessage::SetScene(Scene::Empty));
        }),
        Accelerator::new("scene-error", shortcut("Primary+3"), move || {
            error.emit(ExplorerMessage::SetScene(Scene::Error));
        }),
        // Deliberately without a menu-item chord: the defer-to-typing policy
        // below is owned by the accelerator table, which a native menu key
        // equivalent (always firing over text input) cannot express.
        Accelerator::new("toggle-hidden", shortcut("Primary+Shift+H"), move || {
            hidden.emit(ExplorerMessage::SetShowHidden(!show_hidden));
        }),
        // Same chord the Arrange menu displays on its Name item.
        Accelerator::new("sort-name", shortcut("Primary+Shift+N"), move || {
            dispatch.emit(ExplorerMessage::SetSort(TableSort {
                column_id: "name".to_owned(),
                direction: SortDirection::Ascending,
            }));
        }),
        // Menu-owned like the scene chords (File > New Window claims it);
        // declared as the delivery path for hosts without a realized bar.
        Accelerator::new("new-window", shortcut("Primary+N"), move || {
            new_window.emit(ExplorerMessage::NewWindow);
        }),
        // The editor scene has no menu item, so its chord is table-owned.
        Accelerator::new("scene-editor", shortcut("Primary+4"), move || {
            editor.emit(ExplorerMessage::SetScene(Scene::Editor));
        }),
        // Programmatic close of this window from a component message,
        // bypassing close interception. Deliberately distinct from Cmd+W,
        // which stays the native, interceptable close gesture.
        Accelerator::new("window-close-now", shortcut("Primary+Alt+W"), move || {
            close_now.emit(ExplorerMessage::CloseThisWindow);
        }),
    ]
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
    let move_dispatch = dispatch.clone();
    let mut row = list_row(
        title,
        None,
        Some(symbol),
        model.location == location,
        false,
        title,
        move || dispatch.emit(ExplorerMessage::SelectLocation(location)),
    );
    if drag_interactions_enabled() {
        // Dropping a file row onto a sidebar location moves it there.
        row = row.on_drop_accepting([EXPLORER_FILE_PAYLOAD_TYPE], move |drop| {
            move_dispatch.emit(ExplorerMessage::MoveFileToLocation(
                location,
                drop.payload.id().to_owned(),
            ));
        });
    }
    row.with_key(format!(
        "location-{}",
        title.to_lowercase().replace(' ', "-")
    ))
}

fn directory_content(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    let copy_dispatch = dispatch.clone();
    let paste_dispatch = dispatch.clone();
    let filter_dispatch = dispatch.clone();
    let mut status_children = vec![
        label(scene_summary(model))
            .text_role(TextRole::Secondary)
            .with_key("item-summary"),
        label(model.last_file_action.clone().unwrap_or_default())
            .text_role(TextRole::Secondary)
            .with_key("file-action-note"),
        label(model.drag_note.clone().unwrap_or_default())
            .text_role(TextRole::Secondary)
            .with_key("drag-note"),
    ];
    if let Some(note) = &model.clipboard_note {
        status_children.push(
            label(note.clone())
                .text_role(TextRole::Secondary)
                .with_key("clipboard-note"),
        );
    }
    if let Some(note) = &model.window_note {
        status_children.push(
            label(note.clone())
                .text_role(TextRole::Secondary)
                .with_key("window-note"),
        );
    }
    status_children.push(spacer(true, false).with_key("status-space"));
    status_children.push(
        label(connection_status(model.scene))
            .text_role(TextRole::Secondary)
            .with_key("connection-status"),
    );
    column([
        column([
            label(model.location.title())
                .text_role(TextRole::Title)
                .with_key("directory-title"),
            row([
                label(model.location.path())
                    .text_role(TextRole::Monospace)
                    .selectable(true)
                    .with_key("directory-path"),
                spacer(true, false).with_key("directory-path-space"),
                button("Copy Path", "Copy the current directory path", move || {
                    copy_dispatch.emit(ExplorerMessage::CopyPath);
                })
                .control_size(ControlSize::Small)
                .with_key("copy-path"),
                button("Paste Path", "Go to the path on the clipboard", move || {
                    paste_dispatch.emit(ExplorerMessage::PastePath);
                })
                .control_size(ControlSize::Small)
                .with_key("paste-path"),
            ])
            .align(Align::Center)
            .spacing(Spacing::Related)
            .with_key("directory-path-row"),
            // A live listing filter: the mounted native search field the
            // consumer test harness types into and reads back.
            input(
                model.file_filter.clone(),
                "Filter",
                InputKind::Search,
                "Filter files",
                move |value| filter_dispatch.emit(ExplorerMessage::SetFileFilter(value)),
            )
            .with_key("filter-files"),
        ])
        .spacing(Spacing::Compact)
        .padding(Spacing::Content)
        .with_key("directory-header"),
        separator(Axis::Horizontal).with_key("directory-separator"),
        scene_body(model, dispatch),
        separator(Axis::Horizontal).with_key("status-separator"),
        row(status_children)
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
        Scene::Empty => {
            // The status copy promises "drop files here"; the enclosing
            // column is the region that keeps that promise.
            let mut layout = column([status(
                "This folder is empty",
                "Create a folder or drop files here to begin.",
                StatusTone::Empty,
            )
            .with_key("directory-empty")])
            .justify(Justify::Center);
            if drag_interactions_enabled() {
                layout = layout
                    .on_file_drop(move |drop| dispatch.emit(ExplorerMessage::FilesDropped(drop)));
            }
            layout.with_key("directory-empty-layout")
        }
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
        Scene::Canvas => canvas_pane(model, dispatch),
        Scene::Editor => editor_pane(model, dispatch),
        Scene::Dock => dock_pane(model, dispatch),
    }
}

/// The tabbed-document dock scene: the editor and canvas scene contents plus
/// a notes document hosted as dock tabs, with explicit split, dirty, and
/// persistence commands so every capability is drivable without a drag.
fn dock_pane(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    let contents: Vec<Element> = model
        .dock_layout
        .tabs()
        .into_iter()
        .map(|tab| match tab.id.as_str() {
            DOCK_TAB_EDITOR => editor_pane(model, dispatch.clone()).with_key(DOCK_TAB_EDITOR),
            DOCK_TAB_CANVAS => canvas_pane(model, dispatch.clone()).with_key(DOCK_TAB_CANVAS),
            _ => notes_pane().with_key(tab.id.clone()),
        })
        .collect();
    let handler = dispatch.clone();
    let mut area = dock(
        model.dock_layout.clone(),
        "Documents",
        contents,
        move |event| handler.emit(ExplorerMessage::Dock(event)),
    )
    .with_key("dock-area");
    for tab in model.dock_layout.tabs() {
        let others = dispatch.clone();
        let rightward = dispatch.clone();
        let other_tab = tab.id.clone();
        let right_tab = tab.id.clone();
        area = area.dock_tab_menu(
            tab.id.clone(),
            [
                MenuEntry::item(MenuItem::new("close-others", "Close Others", move || {
                    others.emit(ExplorerMessage::DockCloseOthers(other_tab.clone()));
                })),
                MenuEntry::item(MenuItem::new(
                    "close-right",
                    "Close to the Right",
                    move || {
                        rightward.emit(ExplorerMessage::DockCloseToTheRight(right_tab.clone()));
                    },
                )),
            ],
        );
    }
    let split_right = dispatch.clone();
    let split_down = dispatch.clone();
    let mark_dirty = dispatch.clone();
    let save = dispatch.clone();
    let restore = dispatch.clone();
    column([
        row([
            button(
                "Split Right",
                "Split the active tab to the right",
                move || {
                    split_right.emit(ExplorerMessage::DockSplitActive(DockEdge::Trailing));
                },
            )
            .control_size(ControlSize::Small)
            .with_key("dock-split-right"),
            button("Split Down", "Split the active tab downward", move || {
                split_down.emit(ExplorerMessage::DockSplitActive(DockEdge::Bottom));
            })
            .control_size(ControlSize::Small)
            .with_key("dock-split-down"),
            button(
                "Mark Dirty",
                "Toggle the active tab's unsaved indicator",
                move || mark_dirty.emit(ExplorerMessage::DockMarkActiveDirty),
            )
            .control_size(ControlSize::Small)
            .with_key("dock-mark-dirty"),
            button("Save Layout", "Serialize the dock layout", move || {
                save.emit(ExplorerMessage::DockSaveLayout);
            })
            .control_size(ControlSize::Small)
            .with_key("dock-save-layout"),
            button(
                "Restore Layout",
                "Restore the saved dock layout",
                move || {
                    restore.emit(ExplorerMessage::DockRestoreLayout);
                },
            )
            .control_size(ControlSize::Small)
            .with_key("dock-restore-layout"),
            spacer(true, false).with_key("dock-toolbar-space"),
            label(
                model
                    .dock_note
                    .clone()
                    .unwrap_or_else(|| "dock: ready".to_owned()),
            )
            .text_role(TextRole::Secondary)
            .with_key("dock-note"),
        ])
        .align(Align::Center)
        .padding(Spacing::Content)
        .with_key("dock-toolbar"),
        area,
    ])
    .spacing(Spacing::Joined)
    .with_key("dock-pane")
}

/// The notes document: static native text content standing in for a third
/// open file.
fn notes_pane() -> Element {
    column([
        label("notes.md")
            .text_role(TextRole::Heading)
            .with_key("notes-title"),
        label("- verify tab drag between groups")
            .text_role(TextRole::Monospace)
            .with_key("notes-line-1"),
        label("- verify edge-drop splits")
            .text_role(TextRole::Monospace)
            .with_key("notes-line-2"),
        label("- verify close-last collapses")
            .text_role(TextRole::Monospace)
            .with_key("notes-line-3"),
        spacer(false, true).with_key("notes-space"),
    ])
    .spacing(Spacing::Related)
    .padding(Spacing::Content)
    .with_key("notes-pane")
}

/// The native multi-line editor over a real file: monospace text area with
/// consumer-computed highlight spans, controlled selection, and read-only
/// mode.
fn editor_pane(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    let read_only_dispatch = dispatch.clone();
    let jump_dispatch = dispatch.clone();
    let rehighlight_dispatch = dispatch.clone();
    let reload_dispatch = dispatch.clone();
    let change_dispatch = dispatch.clone();
    let file_name = model.editor.file_name().to_owned();

    let mut area = text_area(
        model.editor.content(),
        format!("Editor for {file_name}"),
        move |change| change_dispatch.emit(ExplorerMessage::EditorChanged(change)),
    )
    .text_role(TextRole::Monospace)
    .read_only(model.editor.read_only())
    .highlight_spans(model.editor.highlight())
    .on_selection_change(move |selection| {
        dispatch.emit(ExplorerMessage::EditorSelectionChanged(selection));
    });
    if let Some(selection) = model.editor.selection() {
        area = area.text_selection(selection);
    }

    column([
        row([
            toggle(
                "Read-only",
                model.editor.read_only(),
                "Reject edits while keeping selection and copying",
                move |value| read_only_dispatch.emit(ExplorerMessage::EditorSetReadOnly(value)),
            )
            .control_size(ControlSize::Small)
            .with_key("editor-readonly"),
            button(
                "Go to End",
                "Move the cursor to the end of the document",
                move || jump_dispatch.emit(ExplorerMessage::EditorJumpEnd),
            )
            .with_key("editor-jump-end"),
            button(
                "Rehighlight",
                "Recompute highlighting for the whole document",
                move || rehighlight_dispatch.emit(ExplorerMessage::EditorRehighlight),
            )
            .with_key("editor-rehighlight"),
            button(
                "Reload",
                "Restore the document from its original text",
                move || reload_dispatch.emit(ExplorerMessage::EditorReload),
            )
            .with_key("editor-reload"),
            spacer(true, false).with_key("editor-toolbar-space"),
            label(model.editor.status_line())
                .text_role(TextRole::Secondary)
                .with_key("editor-status"),
        ])
        .align(Align::Center)
        .padding(Spacing::Content)
        .with_key("editor-toolbar"),
        area.with_key("editor-textarea"),
    ])
    .spacing(Spacing::Joined)
    .with_key("editor-pane")
}

/// Logical cell size of the deterministic canvas grid.
const CANVAS_CELL: f64 = 32.0;
/// Grid columns in the deterministic canvas test pattern.
const CANVAS_GRID_COLUMNS: usize = 8;
/// Grid rows in the deterministic canvas test pattern.
const CANVAS_GRID_ROWS: usize = 5;
/// Outer margin around the deterministic canvas test pattern.
const CANVAS_MARGIN: f64 = 8.0;
/// Full logical extent of the canvas test pattern.
const CANVAS_EXTENT: CanvasSize = CanvasSize::new(
    CANVAS_MARGIN * 2.0 + CANVAS_CELL * CANVAS_GRID_COLUMNS as f64,
    CANVAS_MARGIN * 2.0 + CANVAS_CELL * CANVAS_GRID_ROWS as f64 + 64.0,
);

/// Font size of the canvas text-input echo line.
const ECHO_FONT_SIZE: f64 = 13.0;
/// Approximate advance of one echoed ASCII glyph. The echo surface only
/// needs a stable estimate for its caret rectangle and preedit placement;
/// a real terminal derives exact values from the adapter's
/// `MonospaceMetrics` instead.
const ECHO_ASCII_ADVANCE: f64 = ECHO_FONT_SIZE * 0.6;
/// Approximate advance of one echoed full-width glyph.
const ECHO_WIDE_ADVANCE: f64 = ECHO_ASCII_ADVANCE * 2.0;
/// Prompt drawn before the echoed text.
const ECHO_PROMPT: &str = "> ";
/// Vertical offset of the echo line below the cell grid.
const ECHO_LINE_OFFSET: f64 = 34.0;
/// Widest echo line kept, in characters, so the line stays inside the canvas.
const ECHO_LINE_CAPACITY: usize = 24;

/// Keeps the newest characters once the echo line reaches its capacity.
fn trim_echo_line(echo: &mut String) {
    while echo.chars().count() > ECHO_LINE_CAPACITY {
        echo.remove(0);
    }
}

/// Approximates the advance of echoed text in logical points.
fn echo_advance(text: &str) -> f64 {
    text.chars()
        .map(|character| {
            if character.is_ascii() {
                ECHO_ASCII_ADVANCE
            } else {
                ECHO_WIDE_ADVANCE
            }
        })
        .sum()
}

/// Top-left of the echo line inside the canvas.
fn echo_origin() -> CanvasPoint {
    CanvasPoint::new(
        CANVAS_MARGIN,
        CANVAS_MARGIN + CANVAS_CELL * CANVAS_GRID_ROWS as f64 + ECHO_LINE_OFFSET,
    )
}

/// The caret rectangle declared to the platform for candidate-window
/// anchoring, following the committed text and the preedit caret.
fn canvas_caret_rect(model: &ExplorerComponent) -> CanvasRect {
    let origin = echo_origin();
    let mut x = origin.x + echo_advance(ECHO_PROMPT) + echo_advance(&model.canvas_echo);
    if let Some((preedit, caret)) = &model.canvas_preedit {
        let caret_chars = caret.map_or_else(
            || preedit.chars().count(),
            |span| span.start.min(preedit.chars().count()),
        );
        let before_caret: String = preedit.chars().take(caret_chars).collect();
        x += echo_advance(&before_caret);
    }
    CanvasRect::new(x, origin.y - 2.0, 2.0, ECHO_FONT_SIZE + 6.0)
}

fn canvas_pane(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    let pointer_dispatch = dispatch.clone();
    let focus_dispatch = dispatch.clone();
    let key_dispatch = dispatch.clone();
    column([
        row([
            spacer(true, false).with_key("canvas-leading-space"),
            canvas(
                CANVAS_EXTENT,
                canvas_test_pattern(model),
                "Canvas test pattern: cell grid, color palette, gauge, monospace glyph run, and text-input echo line",
            )
            .accepts_input(true)
            .ime_caret(canvas_caret_rect(model))
            .on_pointer(move |event| pointer_dispatch.emit(ExplorerMessage::CanvasPointer(event)))
            .on_focus_change(move |focused| {
                focus_dispatch.emit(ExplorerMessage::CanvasFocus(focused));
            })
            .on_key(move |event| key_dispatch.emit(ExplorerMessage::CanvasKey(event)))
            .on_ime(move |event| dispatch.emit(ExplorerMessage::CanvasIme(event)))
            .with_key("canvas-surface"),
            spacer(true, false).with_key("canvas-trailing-space"),
        ])
        .align(Align::Center)
        .with_key("canvas-row"),
        row([
            spacer(true, false).with_key("canvas-caption-leading-space"),
            label(canvas_pointer_caption(model.canvas_pointer))
                .text_role(TextRole::Monospace)
                .with_key("canvas-pointer-caption"),
            spacer(true, false).with_key("canvas-caption-trailing-space"),
        ])
        .align(Align::Center)
        .with_key("canvas-caption-row"),
        row([
            spacer(true, false).with_key("canvas-input-caption-leading-space"),
            label(canvas_input_caption(model))
                .text_role(TextRole::Monospace)
                .with_key("canvas-input-caption"),
            spacer(true, false).with_key("canvas-input-caption-trailing-space"),
        ])
        .align(Align::Center)
        .with_key("canvas-input-caption-row"),
    ])
    .justify(Justify::Center)
    .spacing(Spacing::Section)
    .with_key("canvas-pane")
}

/// Mirrors the text-input state into an assertable caption: focus, the echo
/// line, the live preedit, and the last raw key in chord notation.
fn canvas_input_caption(model: &ExplorerComponent) -> String {
    format!(
        "input: focused={} echo={:?} preedit={:?} key={}",
        model.canvas_focused,
        model.canvas_echo,
        model
            .canvas_preedit
            .as_ref()
            .map_or("", |(text, _)| text.as_str()),
        model
            .canvas_last_key
            .as_ref()
            .map_or_else(|| "none".to_owned(), ToString::to_string),
    )
}

fn canvas_pointer_caption(pointer: Option<PointerEvent>) -> String {
    match pointer {
        None => "pointer: none".to_owned(),
        Some(event) => {
            let phase = match event.phase {
                PointerPhase::Down => "down",
                PointerPhase::Up => "up",
                PointerPhase::Move => "move",
                PointerPhase::Drag => "drag",
                PointerPhase::Scroll => "scroll",
            };
            format!(
                "pointer: {phase} @ ({:.1}, {:.1})",
                event.position.x, event.position.y
            )
        }
    }
}

/// Builds the deterministic owned-drawing test pattern: a hairline cell
/// grid, a color palette strip, a ring gauge, a clipped disc, one monospace
/// glyph run, and the text-input echo line. The optional crosshair follows
/// the last pointer event so the round trip is visible on screen.
fn canvas_test_pattern(model: &ExplorerComponent) -> DrawScene {
    let pointer = model.canvas_pointer;
    let mut scene = DrawScene::new();
    let grid_left = CANVAS_MARGIN;
    let grid_top = CANVAS_MARGIN;
    let grid_width = CANVAS_CELL * CANVAS_GRID_COLUMNS as f64;
    let grid_height = CANVAS_CELL * CANVAS_GRID_ROWS as f64;

    // Panel background and subtle checkerboard.
    scene.fill_rect(
        CanvasRect::new(0.0, 0.0, CANVAS_EXTENT.width, CANVAS_EXTENT.height),
        CanvasColor::rgb(0.08, 0.09, 0.12),
    );
    for row in 0..CANVAS_GRID_ROWS {
        for column in 0..CANVAS_GRID_COLUMNS {
            if (row + column) % 2 == 0 {
                continue;
            }
            scene.fill_rect(
                CanvasRect::new(
                    grid_left + column as f64 * CANVAS_CELL,
                    grid_top + row as f64 * CANVAS_CELL,
                    CANVAS_CELL,
                    CANVAS_CELL,
                ),
                CanvasColor::rgb(0.12, 0.14, 0.18),
            );
        }
    }

    // Hairline cell grid: exactly one device pixel per line at any scale.
    let grid_line = CanvasColor::rgb(0.35, 0.38, 0.45);
    for column in 0..=CANVAS_GRID_COLUMNS {
        let x = grid_left + column as f64 * CANVAS_CELL;
        scene.line(
            CanvasPoint::new(x, grid_top),
            CanvasPoint::new(x, grid_top + grid_height),
            LineWidth::Hairline,
            grid_line,
        );
    }
    for row in 0..=CANVAS_GRID_ROWS {
        let y = grid_top + row as f64 * CANVAS_CELL;
        scene.line(
            CanvasPoint::new(grid_left, y),
            CanvasPoint::new(grid_left + grid_width, y),
            LineWidth::Hairline,
            grid_line,
        );
    }

    // Color palette rects inside the first grid row.
    let palette = [
        CanvasColor::rgb(0.86, 0.27, 0.27),
        CanvasColor::rgb(0.92, 0.56, 0.18),
        CanvasColor::rgb(0.93, 0.83, 0.25),
        CanvasColor::rgb(0.30, 0.74, 0.38),
        CanvasColor::rgb(0.26, 0.52, 0.90),
        CanvasColor::rgb(0.61, 0.36, 0.86),
    ];
    for (index, color) in palette.into_iter().enumerate() {
        scene.fill_rect(
            CanvasRect::new(
                grid_left + index as f64 * CANVAS_CELL + 6.0,
                grid_top + 6.0,
                CANVAS_CELL - 12.0,
                CANVAS_CELL - 12.0,
            ),
            color,
        );
    }

    // Ring gauge: full track circle plus a 270-degree progress arc.
    let gauge_center =
        CanvasPoint::new(grid_left + 1.5 * CANVAS_CELL, grid_top + 3.0 * CANVAS_CELL);
    scene.stroke_circle(
        gauge_center,
        CANVAS_CELL * 0.75,
        LineWidth::Points(4.0),
        CanvasColor::rgb(0.22, 0.25, 0.32),
    );
    scene.stroke_arc(
        gauge_center,
        CANVAS_CELL * 0.75,
        -std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
        LineWidth::Points(4.0),
        CanvasColor::rgb(0.92, 0.56, 0.18),
    );
    scene.fill_circle(gauge_center, 3.0, CanvasColor::rgb(0.92, 0.56, 0.18));

    // Clip demonstration: a disc twice the cell size confined to one cell.
    let clip_cell = CanvasRect::new(
        grid_left + 5.0 * CANVAS_CELL,
        grid_top + 2.0 * CANVAS_CELL,
        CANVAS_CELL,
        CANVAS_CELL,
    );
    scene.push_clip(clip_cell);
    scene.fill_circle(
        CanvasPoint::new(clip_cell.origin.x, clip_cell.origin.y),
        CANVAS_CELL,
        CanvasColor::rgba(0.25, 0.78, 0.72, 0.9),
    );
    scene.pop_clip();

    // Monospace glyph run below the grid.
    scene.glyph_run(
        CanvasPoint::new(grid_left, grid_top + grid_height + 12.0),
        "RINKA CANVAS 0123456789",
        13.0,
        CanvasColor::rgb(0.92, 0.94, 0.97),
    );

    // Text-input echo line: typed and committed text after the prompt, the
    // live preedit rendered distinctly (accent color over an underline —
    // the app owns preedit presentation), and the declared caret rectangle
    // drawn where the OS candidate window anchors.
    let accent = CanvasColor::rgb(1.0, 0.76, 0.24);
    let echo_origin = echo_origin();
    scene.glyph_run(
        echo_origin,
        format!("{ECHO_PROMPT}{}", model.canvas_echo),
        ECHO_FONT_SIZE,
        CanvasColor::rgb(0.85, 0.93, 0.85),
    );
    if let Some((preedit, _)) = &model.canvas_preedit {
        let preedit_left =
            echo_origin.x + echo_advance(ECHO_PROMPT) + echo_advance(&model.canvas_echo);
        scene.glyph_run(
            CanvasPoint::new(preedit_left, echo_origin.y),
            preedit.clone(),
            ECHO_FONT_SIZE,
            accent,
        );
        let underline_y = echo_origin.y + ECHO_FONT_SIZE + 4.0;
        scene.line(
            CanvasPoint::new(preedit_left, underline_y),
            CanvasPoint::new(preedit_left + echo_advance(preedit), underline_y),
            LineWidth::Points(1.5),
            accent,
        );
    }
    scene.fill_rect(canvas_caret_rect(model), accent);
    if model.canvas_focused {
        // Focus ring: the visible sign that the canvas holds keyboard focus.
        scene.stroke_rect(
            CanvasRect::new(
                1.0,
                1.0,
                CANVAS_EXTENT.width - 2.0,
                CANVAS_EXTENT.height - 2.0,
            ),
            LineWidth::Points(2.0),
            CanvasColor::rgb(0.35, 0.62, 1.0),
        );
    }

    // Crosshair follows the last pointer event.
    if let Some(event) = pointer {
        let accent = CanvasColor::rgb(1.0, 0.32, 0.32);
        scene.line(
            CanvasPoint::new(event.position.x, 0.0),
            CanvasPoint::new(event.position.x, CANVAS_EXTENT.height),
            LineWidth::Hairline,
            accent,
        );
        scene.line(
            CanvasPoint::new(0.0, event.position.y),
            CanvasPoint::new(CANVAS_EXTENT.width, event.position.y),
            LineWidth::Hairline,
            accent,
        );
        scene.stroke_circle(event.position, 6.0, LineWidth::Points(1.5), accent);
    }
    scene
}

fn file_list(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    let rows = file_rows(file_records(model), model, &dispatch);

    let column = |id: &'static str, title: &'static str| {
        if model.sort.column_id == id {
            TableColumn::new(id, title).sorted(model.sort.direction)
        } else {
            TableColumn::new(id, title).sortable(true)
        }
    };
    let drop_dispatch = dispatch.clone();
    let mut files = list(format!("Files in {}", model.location.title()), rows)
        .table_columns([
            column("name", "Name"),
            column("modified", "Date Modified"),
            column("size", "Size"),
            column("kind", "Kind"),
        ])
        .collection_pattern(CollectionPattern::DataTable)
        .on_sort_change(move |sort| dispatch.emit(ExplorerMessage::SetSort(sort)));
    if drag_interactions_enabled() {
        files =
            files.on_file_drop(move |drop| drop_dispatch.emit(ExplorerMessage::FilesDropped(drop)));
    }
    files.with_key("file-list")
}

/// Builds the display rows for a record set, appending the native row for a
/// duplicated copy directly after its original.
fn file_rows(
    records: Vec<FileRecord>,
    model: &ExplorerComponent,
    dispatch: &Dispatch<ExplorerMessage>,
) -> Vec<Element> {
    let mut rows = Vec::with_capacity(records.len());
    for record in records {
        rows.push(file_row(record, model, dispatch.clone()));
        if model.duplicated.contains(&record.key) {
            rows.push(copy_row(record));
        }
    }
    rows
}

fn copy_row(record: FileRecord) -> Element {
    let title = format!("{} copy", record.title);
    let accessibility_label = format!("{title}, {}, {}", record.kind, record.size);
    list_row(
        title,
        Some(format!("{} · {}", record.kind, record.size)),
        Some(record.symbol),
        false,
        false,
        accessibility_label,
        announce("select-copy"),
    )
    .table_cells([record.modified, record.size, record.kind])
    .with_key(format!("file-{:?}-copy", record.key))
}

fn file_context_menu(
    record: FileRecord,
    model: &ExplorerComponent,
    dispatch: &Dispatch<ExplorerMessage>,
) -> [MenuEntry; 7] {
    let key = record.key;
    let rename = dispatch.clone();
    let duplicate = dispatch.clone();
    let open_editor = dispatch.clone();
    let open_terminal = dispatch.clone();
    let favorite = dispatch.clone();
    let delete = dispatch.clone();
    [
        MenuEntry::item(
            MenuItem::new("rename", "Rename", move || {
                rename.emit(ExplorerMessage::RenameFile(key));
            })
            .help("Rename this item"),
        ),
        MenuEntry::item(
            MenuItem::new("duplicate", "Duplicate", move || {
                duplicate.emit(ExplorerMessage::DuplicateFile(key));
            })
            .enabled(!model.duplicated.contains(&key))
            .help("Create a copy next to this item"),
        ),
        MenuEntry::separator(),
        MenuEntry::submenu(Submenu::new(
            "open-with",
            "Open With",
            [
                MenuEntry::item(
                    MenuItem::new("open-editor", "Editor", move || {
                        open_editor.emit(ExplorerMessage::OpenFileWith(key, "Editor"));
                    })
                    .symbol(Symbol::Code)
                    .help("Open this item in the editor"),
                ),
                MenuEntry::item(
                    MenuItem::new("open-terminal", "Terminal", move || {
                        open_terminal.emit(ExplorerMessage::OpenFileWith(key, "Terminal"));
                    })
                    .symbol(Symbol::Terminal)
                    .help("Open this item in the terminal"),
                ),
            ],
        )),
        MenuEntry::item(
            MenuItem::new("favorite", "Favorite", move || {
                favorite.emit(ExplorerMessage::ToggleFavoriteFile(key));
            })
            .checked(model.favorite_files.contains(&key))
            .help("Keep this item in favorites"),
        ),
        MenuEntry::separator(),
        MenuEntry::item(
            MenuItem::new("delete", "Delete", move || {
                delete.emit(ExplorerMessage::DeleteFile(key));
            })
            .destructive()
            .help("Delete this item"),
        ),
    ]
}

/// Builds the lazily materialized export promise for one file row.
///
/// The exported file is a small generated manifest, not the (fictional)
/// remote bytes; its content is written only when a destination accepts the
/// drop, and the outcome always returns to `update` as a message.
fn file_export_promise(record: FileRecord, dispatch: &Dispatch<ExplorerMessage>) -> FilePromise {
    let file_name = format!("{}.txt", record.title);
    let export_dispatch = dispatch.clone();
    FilePromise::new(file_name.clone(), "public.plain-text", move |path| {
        let content = format!(
            "Generated by Rinka Explorer\nfile: {}\nkind: {}\nsize: {}\n",
            record.title, record.kind, record.size,
        );
        let outcome = std::fs::write(path, content).map_err(|error| error.to_string());
        export_dispatch.emit(ExplorerMessage::FileExported(
            outcome.clone().map(|()| file_name.clone()),
        ));
        outcome
    })
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
    .context_menu(file_context_menu(record, model, &dispatch))
    .with_key(format!("file-{:?}", record.key));

    if drag_interactions_enabled() {
        // Every row moves within the app as a typed payload; non-folder rows
        // additionally export through a file promise, so one drag session
        // serves the sidebar move and the Finder export.
        row = row.drag_payload(DragPayload::new(EXPLORER_FILE_PAYLOAD_TYPE, record.title));
        if record.kind != "Folder" {
            row = row.draggable_file(file_export_promise(record, &dispatch));
        }
    }

    let children = child_file_records(model, record.key);
    if !children.is_empty() {
        let expanded = match record.key {
            FileKey::Src => model.src_expanded,
            FileKey::Assets => model.assets_expanded,
            _ => false,
        };
        let expansion_dispatch = dispatch.clone();
        row = row
            .list_children(file_rows(children, model, &dispatch))
            .expanded(expanded)
            .on_expansion_change(move |value| {
                expansion_dispatch.emit(ExplorerMessage::SetFileExpanded(record.key, value));
            });
    }
    row
}

fn file_records(model: &ExplorerComponent) -> Vec<FileRecord> {
    let mut records: Vec<FileRecord> = vec![
        FileRecord {
            key: FileKey::Src,
            title: file_title(FileKey::Src),
            modified: "Today",
            size: "—",
            kind: "Folder",
            symbol: Symbol::Folder,
        },
        FileRecord {
            key: FileKey::Assets,
            title: file_title(FileKey::Assets),
            modified: "Today",
            size: "18 items",
            kind: "Folder",
            symbol: Symbol::Folder,
        },
        FileRecord {
            key: FileKey::Cargo,
            title: file_title(FileKey::Cargo),
            modified: "Today, 10:42",
            size: "2.4 KB",
            kind: "TOML document",
            symbol: Symbol::Code,
        },
        FileRecord {
            key: FileKey::Readme,
            title: file_title(FileKey::Readme),
            modified: "Yesterday",
            size: "6.8 KB",
            kind: "Markdown document",
            symbol: Symbol::File,
        },
        FileRecord {
            key: FileKey::Preview,
            title: file_title(FileKey::Preview),
            modified: "Today",
            size: "842 KB",
            kind: "PNG image",
            symbol: Symbol::Image,
        },
    ];
    if model.show_hidden {
        records.push(FileRecord {
            key: FileKey::HiddenEnvironment,
            title: file_title(FileKey::HiddenEnvironment),
            modified: "Today, 09:14",
            size: "312 bytes",
            kind: "Environment file",
            symbol: Symbol::File,
        });
    }
    records.retain(|record| !model.deleted.contains(&record.key));
    if !model.file_filter.is_empty() {
        // Case-insensitive title filter on the top-level listing; a
        // matching folder keeps its children, mirroring what the rendered
        // rows (and therefore `file_record_for_key`) consider visible.
        let needle = model.file_filter.to_lowercase();
        records.retain(|record| record.title.to_lowercase().contains(&needle));
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

fn child_file_records(model: &ExplorerComponent, parent: FileKey) -> Vec<FileRecord> {
    let mut children = child_file_catalog(parent);
    children.retain(|record| !model.deleted.contains(&record.key));
    children
}

fn child_file_catalog(parent: FileKey) -> Vec<FileRecord> {
    match parent {
        FileKey::Src => vec![
            FileRecord {
                key: FileKey::Lib,
                title: file_title(FileKey::Lib),
                modified: "Today, 10:38",
                size: "9.1 KB",
                kind: "Rust source",
                symbol: Symbol::Code,
            },
            FileRecord {
                key: FileKey::Main,
                title: file_title(FileKey::Main),
                modified: "Today, 10:41",
                size: "3.7 KB",
                kind: "Rust source",
                symbol: Symbol::Code,
            },
        ],
        FileKey::Assets => vec![
            FileRecord {
                key: FileKey::AppIcon,
                title: file_title(FileKey::AppIcon),
                modified: "Yesterday",
                size: "128 KB",
                kind: "Icon Composer document",
                symbol: Symbol::Image,
            },
            FileRecord {
                key: FileKey::PreviewAssets,
                title: file_title(FileKey::PreviewAssets),
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
            child_file_records(model, record.key)
                .into_iter()
                .find(|child| child.key == key)
        })
    })
}

/// Compacts a path to its final two components for narrow-pane display.
fn compact_path(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    match path.parent().and_then(std::path::Path::file_name) {
        Some(parent) => format!("{}/{name}", parent.to_string_lossy()),
        None => name,
    }
}

fn inspector(model: &ExplorerComponent, dispatch: Dispatch<ExplorerMessage>) -> Element {
    let detail = match model.scene {
        Scene::Ready if model.selected_file.is_some() => {
            let record = file_record_for_key(model, model.selected_file.expect("selection exists"))
                .expect("selected file must remain visible");
            let mut details = vec![
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
            ];
            if let Some(content) = model.preview_content(record.key) {
                details.push(
                    label("Preview")
                        .text_role(TextRole::Secondary)
                        .with_key("inspector-preview-caption"),
                );
                details.push(
                    image(
                        content.clone(),
                        format!("Bitmap preview of {}", record.title),
                    )
                    .with_key("inspector-preview"),
                );
                for (mode, name) in [
                    (ImageScaling::Fit, "fit"),
                    (ImageScaling::Fill, "fill"),
                    (ImageScaling::Actual, "actual"),
                    (ImageScaling::Center, "center"),
                ] {
                    details.push(
                        label(format!("Scaling: {name}"))
                            .text_role(TextRole::Secondary)
                            .with_key(format!("inspector-scaling-{name}-caption")),
                    );
                    details.push(
                        image(
                            model.scaling_probe.clone(),
                            format!("Scaling probe rendered with {name} mapping"),
                        )
                        .image_scaling(mode)
                        .with_key(format!("inspector-scaling-{name}")),
                    );
                }
            }
            if !model.uploads.is_empty() {
                details.push(
                    label(format!("Uploads queued: {}", model.uploads.len()))
                        .text_role(TextRole::Secondary)
                        .with_key("upload-count"),
                );
                details.push(
                    label(compact_path(&model.uploads[0]))
                        .text_role(TextRole::Monospace)
                        .with_key("upload-first-path"),
                );
            }
            if let Some(path) = &model.download_target {
                details.push(
                    label("Download target")
                        .text_role(TextRole::Secondary)
                        .with_key("download-caption"),
                );
                details.push(
                    label(compact_path(path))
                        .text_role(TextRole::Monospace)
                        .with_key("download-path"),
                );
            }
            let delete_dispatch = dispatch.clone();
            let upload_dispatch = dispatch.clone();
            let download_dispatch = dispatch.clone();
            let delete_key = record.key;
            let download_key = record.key;
            details.extend([
                spacer(false, true).with_key("inspector-space"),
                row([
                    button(
                        "Upload Files…",
                        "Upload files to this location",
                        move || {
                            upload_dispatch.emit(ExplorerMessage::RequestUpload);
                        },
                    )
                    .with_key("upload-files"),
                    spacer(true, false).with_key("inspector-transfer-space"),
                    button(
                        "Download…",
                        format!("Download {}", record.title),
                        move || {
                            download_dispatch.emit(ExplorerMessage::RequestDownload(download_key));
                        },
                    )
                    .with_key("download-file"),
                ])
                .align(Align::Center)
                .with_key("inspector-transfers"),
                row([
                    button("Delete…", format!("Delete {}", record.title), move || {
                        delete_dispatch.emit(ExplorerMessage::ConfirmDelete(delete_key));
                    })
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
            ]);
            column(details)
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
        Scene::Canvas => inspector_status(
            "Canvas surface",
            "The content pane owns its drawing.",
            StatusTone::Informational,
        ),
        Scene::Editor => inspector_status(
            "Native editor",
            "The editor pane is a native text view.",
            StatusTone::Informational,
        ),
        Scene::Dock => inspector_status(
            "Documents dock",
            "Tabs and splits are native controls.",
            StatusTone::Informational,
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
            let records = file_records(model);
            let copies = records
                .iter()
                .filter(|record| model.duplicated.contains(&record.key))
                .count();
            let count = records.len() + copies;
            let selected = usize::from(model.selected_file.is_some());
            format!("{count} items · {selected} selected")
        }
        Scene::Empty => "0 items".to_owned(),
        Scene::Busy => "Refreshing…".to_owned(),
        Scene::Error => "No items available".to_owned(),
        Scene::Canvas => "Deterministic canvas test pattern".to_owned(),
        Scene::Editor => format!("Editing {}", model.editor.file_name()),
        Scene::Dock => format!(
            "{} tabs \u{b7} {} groups",
            model.dock_layout.tabs().len(),
            model.dock_layout.groups().len()
        ),
    }
}

const fn connection_status(scene: Scene) -> &'static str {
    match scene {
        Scene::Ready | Scene::Empty | Scene::Busy | Scene::Canvas | Scene::Editor | Scene::Dock => {
            "Connected securely"
        }
        Scene::Error => "Connection interrupted",
    }
}

#[cfg(test)]
mod tests {
    use super::{Component, ExplorerComponent, ExplorerMessage, Location, Scene, application};
    use rinka::{
        DialogButtonRole, DialogDescription, DialogOutcome, Dispatch, PlatformServices, Renderer,
        UpdateContext, WindowContent, WindowRuntime,
    };
    use rinka_headless::{
        CloseRequestOutcome, FakeClipboard, FakeDialogPresenter, HeadlessBackend,
        HeadlessWindowHost,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Builds an update context over the fake clipboard and a recording
    /// dispatch, so `update` is exercised without any platform or runtime.
    fn clipboard_context(
        fake: &FakeClipboard,
    ) -> (
        UpdateContext<ExplorerMessage>,
        Rc<RefCell<Vec<ExplorerMessage>>>,
    ) {
        let received = Rc::new(RefCell::new(Vec::new()));
        let sink = received.clone();
        let dispatch = Dispatch::from_handler(move |message| sink.borrow_mut().push(message));
        (
            UpdateContext::new(dispatch, PlatformServices::new(fake.handle())),
            received,
        )
    }

    /// Applies every recorded follow-up message, as the runtime queue would.
    fn drain_into(
        component: &mut ExplorerComponent,
        context: &UpdateContext<ExplorerMessage>,
        received: &Rc<RefCell<Vec<ExplorerMessage>>>,
    ) {
        loop {
            let message = received.borrow_mut().pop();
            let Some(message) = message else {
                break;
            };
            component.update(message, context);
        }
    }

    fn mounted_explorer() -> (
        WindowRuntime<HeadlessBackend>,
        FakeDialogPresenter,
        rinka::EventBindings,
    ) {
        let presenter = FakeDialogPresenter::new();
        let runtime = WindowRuntime::mount(
            Renderer::new(HeadlessBackend::new()),
            WindowContent::component(ExplorerComponent::new(Scene::Ready)),
            PlatformServices::default().with_dialog_service(presenter.clone()),
        )
        .expect("initial mount");
        let delete_events = runtime.with_renderer(|renderer| {
            let backend = renderer.backend();
            let handle = backend
                .find_by_key("delete-file")
                .expect("delete button is mounted for the selected file");
            backend.events_of(handle).expect("delete button has events")
        });
        (runtime, presenter, delete_events)
    }

    #[test]
    fn copy_path_writes_the_current_location_path_to_the_service() {
        let fake = FakeClipboard::new();
        let (context, _received) = clipboard_context(&fake);
        let mut component = ExplorerComponent::new(Scene::Ready);
        let expected = component.location.path();

        component.update(ExplorerMessage::CopyPath, &context);

        assert_eq!(fake.text().as_deref(), Some(expected));
        assert_eq!(
            component.clipboard_note.as_deref(),
            Some(format!("Copied {expected}").as_str())
        );
    }

    #[test]
    fn paste_path_navigates_to_a_location_read_from_the_clipboard() {
        let fake = FakeClipboard::new();
        fake.handle()
            .write_text(Location::Home.path())
            .expect("preload");
        let (context, received) = clipboard_context(&fake);
        let mut component = ExplorerComponent::new(Scene::Ready);
        assert_eq!(component.location, Location::RemoteProject);

        component.update(ExplorerMessage::PastePath, &context);
        // The read outcome arrives as a dispatched message, never in place.
        assert_eq!(component.location, Location::RemoteProject);
        drain_into(&mut component, &context, &received);

        assert_eq!(component.location, Location::Home);
        assert_eq!(component.clipboard_note.as_deref(), Some("Went to Home"));
    }

    #[test]
    fn paste_of_unrecognized_cjk_multiline_text_is_echoed_verbatim() {
        let fake = FakeClipboard::new();
        fake.handle()
            .write_text("日本語\nline two")
            .expect("preload");
        let (context, received) = clipboard_context(&fake);
        let mut component = ExplorerComponent::new(Scene::Ready);

        component.update(ExplorerMessage::PastePath, &context);
        drain_into(&mut component, &context, &received);

        assert_eq!(component.location, Location::RemoteProject);
        assert_eq!(
            component.clipboard_note.as_deref(),
            Some("Clipboard: 日本語\nline two")
        );
    }

    #[test]
    fn paste_from_an_empty_clipboard_reports_no_text() {
        let fake = FakeClipboard::new();
        let (context, received) = clipboard_context(&fake);
        let mut component = ExplorerComponent::new(Scene::Ready);

        component.update(ExplorerMessage::PastePath, &context);
        drain_into(&mut component, &context, &received);

        assert_eq!(
            component.clipboard_note.as_deref(),
            Some("Clipboard has no text")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_main_window_declares_the_menu_bar_reconciled_with_scene_state() {
        use rinka::MenuBarMenuRole;

        let ready = application(Scene::Ready);
        // The live bar is the main window's declaration; the application
        // slot stays empty so the router falls back to the main window.
        assert!(ready.menu_bar.is_empty());
        let snapshot = ready.windows[0].content.snapshot();
        let bar = snapshot.menu_bar_model().expect("declared menu bar");
        let labels: Vec<&str> = bar.menus.iter().map(|menu| menu.label.as_str()).collect();
        assert_eq!(labels, ["File", "Edit", "View", "Window", "Help"]);
        assert_eq!(bar.menus[3].role, MenuBarMenuRole::Window);
        assert_eq!(bar.menus[4].role, MenuBarMenuRole::Help);
        assert!(bar.find_item("file-new-folder").expect("declared").enabled);
        assert!(bar.find_item("file-new-window").expect("declared").enabled);
        assert!(bar.find_item("view-scene-ready").expect("declared").checked);
        assert!(!bar.find_item("view-scene-empty").expect("declared").checked);

        let error_snapshot = application(Scene::Error).windows[0].content.snapshot();
        let error_bar = error_snapshot.menu_bar_model().expect("declared menu bar");
        assert!(
            !error_bar
                .find_item("file-new-folder")
                .expect("declared")
                .enabled
        );
        assert!(
            error_bar
                .find_item("view-scene-error")
                .expect("declared")
                .checked
        );
    }

    #[test]
    fn dropped_files_reach_the_status_note_with_names_and_position() {
        let fake = FakeClipboard::new();
        let (context, _received) = clipboard_context(&fake);
        let mut component = ExplorerComponent::new(Scene::Ready);

        component.update(
            ExplorerMessage::FilesDropped(rinka::FileDrop {
                paths: vec![
                    std::path::PathBuf::from("/tmp/report.pdf"),
                    std::path::PathBuf::from("/tmp/photo.png"),
                ],
                position: rinka::DropPosition::new(140.4, 60.6),
            }),
            &context,
        );

        assert_eq!(
            component.drag_note.as_deref(),
            Some("Dropped 2 file(s) at (140, 61): report.pdf, photo.png")
        );
    }

    #[test]
    fn a_file_promise_materializes_lazily_and_reports_the_export() {
        let fake = FakeClipboard::new();
        let (context, received) = clipboard_context(&fake);
        let mut component = ExplorerComponent::new(Scene::Ready);
        let dispatch = {
            let sink = received.clone();
            rinka::Dispatch::from_handler(move |message| sink.borrow_mut().push(message))
        };
        let record = super::file_records(&component)
            .into_iter()
            .find(|record| record.key == super::FileKey::Readme)
            .expect("README.md is listed");
        let promise = super::file_export_promise(record, &dispatch);
        let directory =
            std::env::temp_dir().join(format!("rinka-explorer-export-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp export directory");
        let destination = directory.join(promise.file_name());

        // Nothing materializes before a destination accepts the drop.
        assert!(!destination.exists());
        promise.write_to(&destination).expect("export succeeds");

        let content = std::fs::read_to_string(&destination).expect("exported manifest");
        assert!(content.contains("file: README.md"));
        drain_into(&mut component, &context, &received);
        assert_eq!(
            component.drag_note.as_deref(),
            Some("Exported README.md.txt")
        );
        std::fs::remove_dir_all(&directory).expect("temp export cleanup");
    }

    #[test]
    fn a_row_dropped_onto_a_sidebar_location_records_the_move() {
        let fake = FakeClipboard::new();
        let (context, _received) = clipboard_context(&fake);
        let mut component = ExplorerComponent::new(Scene::Ready);

        component.update(
            ExplorerMessage::MoveFileToLocation(Location::Documents, "README.md".to_owned()),
            &context,
        );

        assert_eq!(
            component.drag_note.as_deref(),
            Some("Moved README.md to Documents")
        );
    }

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

    #[test]
    fn the_delete_confirm_keeps_destructive_off_the_return_key_default() {
        let (_runtime, presenter, delete_events) = mounted_explorer();
        delete_events.emit_activate();

        assert_eq!(presenter.presented_count(), 1);
        let Some(DialogDescription::Alert(alert)) = presenter.description(0) else {
            panic!("expected a confirmation alert");
        };
        assert_eq!(alert.title, "Delete \u{201c}Cargo.toml\u{201d}?");
        assert_eq!(alert.buttons[0].label, "Cancel");
        assert_eq!(alert.buttons[0].role, DialogButtonRole::Cancel);
        assert_eq!(alert.buttons[1].label, "Delete");
        assert_eq!(alert.buttons[1].role, DialogButtonRole::Destructive);
        // Cancel owns the return key; the destructive button never does.
        assert_eq!(alert.default_button, Some(0));
    }

    #[test]
    fn confirming_the_delete_removes_the_file_and_clears_the_selection() {
        let (runtime, presenter, delete_events) = mounted_explorer();
        delete_events.emit_activate();

        assert!(presenter.deliver(0, DialogOutcome::ButtonChosen(1)));
        runtime.with_renderer(|renderer| {
            let backend = renderer.backend();
            assert!(backend.find_by_key("file-Cargo").is_none());
            assert!(backend.find_by_key("inspector-state").is_some());
        });
        assert!(runtime.take_error().is_none());
    }

    #[test]
    fn the_dock_scene_round_trips_a_dirty_close_through_the_dialog() {
        use rinka::DockEvent;
        let presenter = FakeDialogPresenter::new();
        let runtime = WindowRuntime::mount(
            Renderer::new(HeadlessBackend::new()),
            WindowContent::component(ExplorerComponent::new(Scene::Dock)),
            PlatformServices::default().with_dialog_service(presenter.clone()),
        )
        .expect("initial mount");
        let (dock, events) = runtime.with_renderer(|renderer| {
            let backend = renderer.backend();
            let dock = backend
                .find_by_key("dock-area")
                .expect("the dock scene mounts the dock");
            (dock, backend.events_of(dock).expect("dock has events"))
        });
        runtime.with_renderer(|renderer| {
            let layout = renderer
                .backend()
                .dock_layout_of(dock)
                .expect("dock layout realized");
            assert_eq!(layout.tab_ids(), ["editor", "canvas", "notes"]);
        });

        // The dirty notes tab closes only through the dialog round trip.
        assert!(events.emit_dock(DockEvent::CloseTab {
            group: "documents".to_owned(),
            tab: "notes".to_owned(),
        }));
        assert_eq!(presenter.presented_count(), 1);
        let Some(DialogDescription::Alert(alert)) = presenter.description(0) else {
            panic!("expected a close confirmation");
        };
        assert_eq!(alert.buttons[0].role, DialogButtonRole::Cancel);
        assert_eq!(alert.default_button, Some(0));
        assert!(presenter.deliver(0, DialogOutcome::ButtonChosen(1)));
        runtime.with_renderer(|renderer| {
            let layout = renderer
                .backend()
                .dock_layout_of(dock)
                .expect("dock layout realized");
            assert_eq!(layout.tab_ids(), ["editor", "canvas"]);
        });
        assert!(runtime.take_error().is_none());
    }

    #[test]
    fn a_new_window_message_opens_a_second_explorer_through_the_window_service() {
        let host = HeadlessWindowHost::new();
        host.open(super::main_window(Scene::Ready))
            .expect("open the main window");
        let fake = FakeClipboard::new();
        let received = Rc::new(RefCell::new(Vec::new()));
        let sink = received.clone();
        let dispatch = Dispatch::from_handler(move |message| sink.borrow_mut().push(message));
        let services = PlatformServices::new(fake.handle()).with_window_service(host.service());
        let context = UpdateContext::new(dispatch, services);
        let mut component = ExplorerComponent::new(Scene::Ready);

        component.update(ExplorerMessage::NewWindow, &context);

        let ids = host.open_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids[1].as_str().starts_with("explorer-secondary-"));
        // The new window took focus, and the opener recorded the action.
        assert_eq!(host.focused(), Some(ids[1].clone()));
        assert!(
            component
                .last_file_action
                .as_deref()
                .is_some_and(|note| note.starts_with("Opened window explorer-secondary-"))
        );
        // On the host that realizes the declaration, the secondary titles
        // itself from its own scene state from the first render.
        if cfg!(target_os = "macos") {
            assert_eq!(
                host.title_of(&ids[1]).as_deref(),
                Some("Rinka Explorer \u{2014} Ready")
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_editor_window_vetoes_its_close_once_then_confirms_through_the_sheet() {
        let presenter = FakeDialogPresenter::new();
        let dialog_presenter = presenter.clone();
        let host = HeadlessWindowHost::new().with_services(move || {
            PlatformServices::default().with_dialog_service(dialog_presenter.clone())
        });
        host.open(super::main_window(Scene::Editor))
            .expect("open the editor window");
        let id = rinka::WindowId::new("explorer-main");

        // The user's close gesture is deferred behind the confirm sheet.
        assert_eq!(
            host.request_close(&id).expect("request close"),
            CloseRequestOutcome::Deferred
        );
        assert_eq!(presenter.presented_count(), 1);
        let Some(DialogDescription::Alert(alert)) = presenter.description(0) else {
            panic!("expected the close confirmation alert");
        };
        assert_eq!(alert.buttons[0].label, "Cancel");
        assert_eq!(alert.buttons[0].role, DialogButtonRole::Cancel);
        assert_eq!(alert.buttons[1].label, "Close");
        assert_eq!(alert.buttons[1].role, DialogButtonRole::Destructive);
        assert_eq!(alert.default_button, Some(0));

        // Cancel vetoes: the window survives its own close request.
        assert!(presenter.deliver(0, DialogOutcome::ButtonChosen(0)));
        assert!(host.is_open(&id));
        assert!(host.pending_close_ids().is_empty());

        // A second gesture confirmed through the sheet closes the window.
        assert_eq!(
            host.request_close(&id).expect("request close"),
            CloseRequestOutcome::Deferred
        );
        assert!(presenter.deliver(1, DialogOutcome::ButtonChosen(1)));
        assert!(!host.is_open(&id));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_ready_window_keeps_the_fully_native_close_path() {
        let host = HeadlessWindowHost::new();
        host.open(super::main_window(Scene::Ready))
            .expect("open the main window");

        // No editor session, no interception: the gesture closes natively.
        assert_eq!(
            host.request_close(&rinka::WindowId::new("explorer-main"))
                .expect("request close"),
            CloseRequestOutcome::ClosedImmediately
        );
        assert!(host.open_ids().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_secondary_window_declares_its_title_from_scene_state() {
        let mut component = ExplorerComponent::secondary(Scene::Ready, 900);
        let recording = Dispatch::from_handler(|_message: ExplorerMessage| {});
        assert_eq!(
            component.view(recording.clone()).window_title_model(),
            Some("Rinka Explorer \u{2014} Ready")
        );
        // The declared title is a pure function of the scene state.
        let fake = FakeClipboard::new();
        let context = UpdateContext::new(recording.clone(), PlatformServices::new(fake.handle()));
        component.update(ExplorerMessage::SetScene(Scene::Editor), &context);
        let view = component.view(recording);
        assert_eq!(
            view.window_title_model(),
            Some("Rinka Explorer \u{2014} Editor")
        );
        // The editor scene also declares close interception; other scenes
        // (asserted above through the main window) do not.
        assert!(view.declares_close_request());
        // The main window keeps its launch title.
        let main = ExplorerComponent::new(Scene::Ready);
        let main_view = main.view(Dispatch::from_handler(|_message: ExplorerMessage| {}));
        assert_eq!(main_view.window_title_model(), None);
        assert!(!main_view.declares_close_request());
        assert!(main_view.declares_window_events());
    }

    #[test]
    fn cancelling_the_delete_keeps_the_file() {
        let (runtime, presenter, delete_events) = mounted_explorer();
        delete_events.emit_activate();

        assert!(presenter.deliver(0, DialogOutcome::ButtonChosen(0)));
        runtime.with_renderer(|renderer| {
            let backend = renderer.backend();
            assert!(backend.find_by_key("file-Cargo").is_some());
        });
        assert!(runtime.take_error().is_none());
    }
}
