//! Native WinUI 3 projection of Rinka's declarative application model.
//!
//! The adapter keeps the common component runtime authoritative and translates
//! its retained mounted tree into native WinUI controls. Windows App SDK
//! runtime staging belongs to the executable package that calls [`run`].

use rinka_core::{ApplicationSpec, Element, ElementKind, PanelBehavior, WindowKind};
use std::error::Error;
use std::fmt;

/// Identifies this platform adapter in diagnostics.
pub const PLATFORM_NAME: &str = "Windows WinUI 3";

pub mod accelerator_mapping;

#[cfg(target_os = "windows")]
mod platform;

/// Builds this host's service registry.
///
/// The WinUI host does not implement the clipboard service yet
/// (`reports/clipboard-access`); the registry carries the typed rejection so
/// component update logic observes an honest [`rinka_core::ClipboardError`]
/// instead of a stub success.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn platform_services() -> rinka_core::PlatformServices {
    rinka_core::PlatformServices::new(rinka_core::Clipboard::unsupported(PLATFORM_NAME))
}

/// A diagnostic returned before or while starting the WinUI host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WinUiDiagnostic {
    /// The adapter was called on a non-Windows operating system.
    UnsupportedPlatform,
    /// The application did not declare a main window.
    MissingMainWindow,
    /// The application declared more than one main window.
    MultipleMainWindows,
    /// The declared top-level semantic kind is not implemented by this host.
    UnsupportedWindowKind {
        /// Stable window identity.
        window_id: String,
        /// Rejected semantic kind.
        kind: WindowKind,
    },
    /// The declared content uses a semantic element this host does not
    /// realize yet.
    UnsupportedElement {
        /// Stable window identity.
        window_id: String,
        /// Rejected semantic element kind.
        element: ElementKind,
    },
    /// A panel requested behavior that the WinUI host cannot represent.
    UnsupportedPanelBehavior {
        /// Stable panel identity.
        window_id: String,
        /// Human-readable unsupported field.
        field: &'static str,
    },
    /// A declared element capability that the WinUI host does not implement.
    UnsupportedElementCapability {
        /// Element that requested the capability.
        kind: ElementKind,
        /// Stable capability identifier.
        capability: &'static str,
    },
    /// A window declared an accelerator table this host does not deliver yet.
    ///
    /// The KeyboardAccelerator integration is tracked in
    /// `reports/keyboard-shortcuts-and-key-events`; rejecting the table keeps
    /// the contract honest instead of silently dropping declared chords.
    UnsupportedAccelerators {
        /// Stable window identity.
        window_id: String,
    },
    /// The application or a window declared a menu bar this host does not
    /// realize yet.
    ///
    /// The WinUI `MenuBar` control integration is tracked in
    /// `reports/app-menu-bar`; rejecting the declaration keeps the contract
    /// honest instead of silently dropping declared menus.
    UnsupportedMenuBar {
        /// `"application"` for the application-level declaration, otherwise
        /// the declaring window's stable identity.
        scope: String,
    },
    /// Window content declared a semantic capability this host does not
    /// realize yet.
    UnsupportedContentCapability {
        /// Stable window identity.
        window_id: String,
        /// Stable capability identifier.
        capability: &'static str,
    },
    /// Common content was structurally invalid at initial projection time.
    Projection(String),
    /// The Windows App SDK host returned a native error.
    Native(String),
}

impl fmt::Display for WinUiDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter.write_str("WinUI 3 requires Windows"),
            Self::MissingMainWindow => formatter.write_str("application has no main window"),
            Self::MultipleMainWindows => {
                formatter.write_str("application has more than one main window")
            }
            Self::UnsupportedWindowKind { window_id, kind } => {
                write!(
                    formatter,
                    "window '{window_id}' uses unsupported kind {kind:?}"
                )
            }
            Self::UnsupportedElement { window_id, element } => {
                write!(
                    formatter,
                    "window '{window_id}' declares content element {element:?}, which the WinUI host does not realize yet"
                )
            }
            Self::UnsupportedPanelBehavior { window_id, field } => write!(
                formatter,
                "panel '{window_id}' requests unsupported behavior '{field}'"
            ),
            Self::UnsupportedElementCapability { kind, capability } => write!(
                formatter,
                "WinUI host does not implement {capability} for {kind:?}"
            ),
            Self::UnsupportedAccelerators { window_id } => write!(
                formatter,
                "window '{window_id}' declares an accelerator table the WinUI host does not deliver yet"
            ),
            Self::UnsupportedMenuBar { scope } => write!(
                formatter,
                "'{scope}' declares a menu bar the WinUI host does not realize yet"
            ),
            Self::UnsupportedContentCapability {
                window_id,
                capability,
            } => write!(
                formatter,
                "window '{window_id}' content requests unsupported capability '{capability}'"
            ),
            Self::Projection(message) => write!(formatter, "common projection failed: {message}"),
            Self::Native(message) => write!(formatter, "WinUI 3 host failed: {message}"),
        }
    }
}

impl Error for WinUiDiagnostic {}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AuxiliaryVisibility {
    pub(crate) sidebar_open: bool,
    pub(crate) inspector_open: bool,
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn resolve_workspace_visibility(
    sidebar_collapsible: bool,
    requested_sidebar_open: bool,
    inspector_collapsible: bool,
    requested_inspector_open: bool,
    _window_width: f64,
) -> AuxiliaryVisibility {
    AuxiliaryVisibility {
        sidebar_open: if sidebar_collapsible {
            requested_sidebar_open
        } else {
            true
        },
        inspector_open: if inspector_collapsible {
            requested_inspector_open
        } else {
            true
        },
    }
}

fn validate_application(application: &ApplicationSpec) -> Result<(), WinUiDiagnostic> {
    if !application.menu_bar.is_empty() {
        return Err(WinUiDiagnostic::UnsupportedMenuBar {
            scope: "application".to_owned(),
        });
    }
    let mut main_windows = 0_usize;
    for window in &application.windows {
        match window.kind {
            WindowKind::Main => main_windows += 1,
            WindowKind::Panel(behavior) => validate_panel(window.id.as_str(), behavior)?,
            WindowKind::Preferences => {
                return Err(WinUiDiagnostic::UnsupportedWindowKind {
                    window_id: window.id.as_str().to_owned(),
                    kind: window.kind,
                });
            }
        }
        let snapshot = window.content.snapshot();
        if !snapshot.accelerator_table().is_empty() {
            return Err(WinUiDiagnostic::UnsupportedAccelerators {
                window_id: window.id.as_str().to_owned(),
            });
        }
        if snapshot.menu_bar_model().is_some() {
            return Err(WinUiDiagnostic::UnsupportedMenuBar {
                scope: window.id.as_str().to_owned(),
            });
        }
        validate_content(window.id.as_str(), &snapshot)?;
    }
    match main_windows {
        0 => Err(WinUiDiagnostic::MissingMainWindow),
        1 => Ok(()),
        _ => Err(WinUiDiagnostic::MultipleMainWindows),
    }
}

/// Rejects declared content this host cannot realize, before projection.
///
/// The walk sees each window's initial component view; content that only a
/// later state transition declares is outside this startup validation.
/// The owned-drawing canvas, the bitmap image element, and element context
/// menus (the WinUI `MenuFlyout` realization is recorded in
/// reports/context-menus) are rejected with typed diagnostics and never
/// replaced with a visually unrelated control.
fn validate_content(window_id: &str, element: &Element) -> Result<(), WinUiDiagnostic> {
    if element.kind() == ElementKind::Canvas {
        return Err(WinUiDiagnostic::UnsupportedElementCapability {
            kind: ElementKind::Canvas,
            capability: "owned-drawing canvas surface",
        });
    }
    if element.kind() == ElementKind::Image {
        // The WinUI host has no bitmap image realization yet (Image over a
        // WriteableBitmap is the planned mapping); per the AGENTS contract
        // it rejects the tree instead of substituting an unrelated control.
        return Err(WinUiDiagnostic::UnsupportedElement {
            window_id: window_id.to_owned(),
            element: ElementKind::Image,
        });
    }
    if element.context_menu_model().is_some() {
        return Err(WinUiDiagnostic::UnsupportedContentCapability {
            window_id: window_id.to_owned(),
            capability: "context menu",
        });
    }
    if element.kind() == ElementKind::TextArea {
        // The WinUI host has no multi-line editable text realization yet
        // (RichEditBox driven by the controlled-text protocol is the planned
        // mapping); it rejects the tree instead of substituting a TextBox.
        return Err(WinUiDiagnostic::UnsupportedElement {
            window_id: window_id.to_owned(),
            element: ElementKind::TextArea,
        });
    }
    if element.kind() == ElementKind::Dock {
        // The WinUI host has no tabbed-document dock realization yet. The
        // intended native mapping — TabView per group (CanReorderTabs, drag
        // between TabViews, TabDroppedOutside) over sized Grid splits — is
        // recorded in reports/document-tabs-and-splits; it rejects the tree
        // instead of substituting an unrelated control.
        return Err(WinUiDiagnostic::UnsupportedElement {
            window_id: window_id.to_owned(),
            element: ElementKind::Dock,
        });
    }
    if element.file_promise_model().is_some()
        || element.drag_payload_model().is_some()
        || element.drop_target_model().is_some()
    {
        // The WinUI realization (CanDrag/AllowDrop with DataPackage) does
        // not exist yet; the typed rejection and its follow-up are recorded
        // in reports/drag-and-drop.
        return Err(WinUiDiagnostic::UnsupportedContentCapability {
            window_id: window_id.to_owned(),
            capability: "drag and drop",
        });
    }
    for child in element.children() {
        validate_content(window_id, child)?;
    }
    Ok(())
}

fn validate_panel(window_id: &str, behavior: PanelBehavior) -> Result<(), WinUiDiagnostic> {
    if behavior.hides_when_inactive {
        return Err(WinUiDiagnostic::UnsupportedPanelBehavior {
            window_id: window_id.to_owned(),
            field: "hides_when_inactive",
        });
    }
    if !behavior.accepts_keyboard {
        return Err(WinUiDiagnostic::UnsupportedPanelBehavior {
            window_id: window_id.to_owned(),
            field: "accepts_keyboard=false",
        });
    }
    Ok(())
}

/// Runs a declarative application in one Windows App SDK application loop.
#[cfg(target_os = "windows")]
pub fn run(application: ApplicationSpec) -> Result<(), WinUiDiagnostic> {
    validate_application(&application)?;
    platform::run(application)
}

/// Reports that WinUI 3 cannot run on the current operating system.
#[cfg(not(target_os = "windows"))]
pub fn run(application: ApplicationSpec) -> Result<(), WinUiDiagnostic> {
    validate_application(&application)?;
    Err(WinUiDiagnostic::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::{WinUiDiagnostic, resolve_workspace_visibility, validate_application};
    use rinka_core::{
        Accelerator, ApplicationSpec, PanelBehavior, Size, ToolbarDisplay, WindowContent, WindowId,
        WindowKind, WindowSpec, column, label,
    };

    fn window(id: &str, kind: WindowKind) -> WindowSpec {
        WindowSpec {
            id: WindowId::new(id),
            title: id.to_owned(),
            kind,
            initial_size: Size::new(640.0, 480.0),
            minimum_size: Size::new(320.0, 240.0),
            toolbar: Vec::new(),
            toolbar_display: ToolbarDisplay::Automatic,
            content: WindowContent::from(label(id)),
        }
    }

    fn application(windows: Vec<WindowSpec>) -> ApplicationSpec {
        ApplicationSpec {
            id: "dev.rinka.test".to_owned(),
            name: "Test".to_owned(),
            menu_bar: rinka_core::MenuBar::default(),
            windows,
        }
    }

    #[test]
    fn accepts_main_window_with_current_activity_panel_behavior() {
        let value = application(vec![
            window("main", WindowKind::Main),
            window(
                "activity",
                WindowKind::Panel(PanelBehavior {
                    floating: true,
                    hides_when_inactive: false,
                    accepts_keyboard: true,
                }),
            ),
        ]);
        assert_eq!(validate_application(&value), Ok(()));
    }

    #[test]
    fn rejects_top_level_kinds_and_panel_behavior_explicitly() {
        let preferences = application(vec![
            window("main", WindowKind::Main),
            window("settings", WindowKind::Preferences),
        ]);
        assert!(matches!(
            validate_application(&preferences),
            Err(WinUiDiagnostic::UnsupportedWindowKind { .. })
        ));

        let hidden_panel = application(vec![
            window("main", WindowKind::Main),
            window(
                "activity",
                WindowKind::Panel(PanelBehavior {
                    floating: true,
                    hides_when_inactive: true,
                    accepts_keyboard: true,
                }),
            ),
        ]);
        assert_eq!(
            validate_application(&hidden_panel),
            Err(WinUiDiagnostic::UnsupportedPanelBehavior {
                window_id: "activity".to_owned(),
                field: "hides_when_inactive",
            })
        );
    }

    #[test]
    fn canvas_content_is_a_typed_unsupported_capability() {
        let mut with_canvas = window("main", WindowKind::Main);
        with_canvas.content = WindowContent::from(rinka_core::column([rinka_core::canvas(
            rinka_core::CanvasSize::new(32.0, 32.0),
            rinka_core::DrawScene::new(),
            "Level meter",
        )]));
        let value = application(vec![with_canvas]);
        assert_eq!(
            validate_application(&value),
            Err(WinUiDiagnostic::UnsupportedElementCapability {
                kind: rinka_core::ElementKind::Canvas,
                capability: "owned-drawing canvas surface",
            })
        );
    }

    #[test]
    fn text_input_canvas_content_is_the_same_typed_unsupported_capability() {
        // An input-accepting canvas (the terminal text-input host of
        // reports/canvas-text-input) is rejected through the canvas
        // capability diagnostic: the WinUI host implements neither the
        // canvas nor its TSF text-input plumbing, and it must reject the
        // tree instead of silently dropping the input surface.
        let mut with_terminal = window("main", WindowKind::Main);
        with_terminal.content = WindowContent::from(rinka_core::column([rinka_core::canvas(
            rinka_core::CanvasSize::new(320.0, 240.0),
            rinka_core::DrawScene::new(),
            "Terminal",
        )
        .accepts_input(true)
        .ime_caret(rinka_core::CanvasRect::new(0.0, 0.0, 2.0, 16.0))
        .on_key(|_| {})
        .on_ime(|_| {})]));
        let value = application(vec![with_terminal]);
        assert_eq!(
            validate_application(&value),
            Err(WinUiDiagnostic::UnsupportedElementCapability {
                kind: rinka_core::ElementKind::Canvas,
                capability: "owned-drawing canvas surface",
            })
        );
    }

    #[test]
    fn bitmap_image_content_is_a_typed_diagnostic() {
        let content = rinka_core::ImageContent::from_rgba8(2, 2, 8, vec![0_u8; 32], 1);
        let mut preview = window("main", WindowKind::Main);
        preview.content =
            WindowContent::from(rinka_core::column([rinka_core::image(content, "Preview")]));
        let value = application(vec![preview]);

        assert_eq!(
            validate_application(&value),
            Err(super::WinUiDiagnostic::UnsupportedElement {
                window_id: "main".to_owned(),
                element: rinka_core::ElementKind::Image,
            })
        );
    }

    #[test]
    fn multi_line_text_area_content_is_a_typed_diagnostic() {
        let content =
            rinka_core::TextContent::new("fn main() {}\n", rinka_core::TextRevision::new(1));
        let mut editor = window("main", WindowKind::Main);
        editor.content =
            WindowContent::from(column([rinka_core::text_area(content, "Editor", |_| {})]));
        let value = application(vec![editor]);

        assert_eq!(
            validate_application(&value),
            Err(WinUiDiagnostic::UnsupportedElement {
                window_id: "main".to_owned(),
                element: rinka_core::ElementKind::TextArea,
            })
        );
    }

    #[test]
    fn tabbed_document_dock_content_is_a_typed_diagnostic() {
        use rinka_core::{DockGroup, DockLayout, DockTab};
        let mut with_dock = window("main", WindowKind::Main);
        with_dock.content = WindowContent::from(rinka_core::dock(
            DockLayout::single_group(DockGroup::new(
                "documents",
                [DockTab::new("readme", "README.md")],
                "readme",
            )),
            "Documents",
            [label("readme contents").with_key("readme")],
            |_| {},
        ));
        assert_eq!(
            validate_application(&application(vec![with_dock])),
            Err(WinUiDiagnostic::UnsupportedElement {
                window_id: "main".to_owned(),
                element: rinka_core::ElementKind::Dock,
            })
        );
    }

    #[test]
    fn declared_accelerator_tables_are_a_typed_diagnostic() {
        let mut shortcut_window = window("main", WindowKind::Main);
        shortcut_window.content = WindowContent::from(
            column([label("main").with_key("title")])
                .with_key("root")
                .accelerators([Accelerator::new(
                    "save",
                    "Primary+S".parse().expect("test chord"),
                    || {},
                )]),
        );

        assert_eq!(
            validate_application(&application(vec![shortcut_window])),
            Err(WinUiDiagnostic::UnsupportedAccelerators {
                window_id: "main".to_owned(),
            })
        );
    }

    #[test]
    fn declared_menu_bars_are_a_typed_diagnostic() {
        use rinka_core::{MenuBar, MenuBarEntry, MenuBarMenu, MenuItem};

        let bar = || {
            MenuBar::new([MenuBarMenu::new(
                "file",
                "File",
                [MenuBarEntry::item(MenuItem::new("open", "Open", || {}))],
            )])
        };

        let mut menu_application = application(vec![window("main", WindowKind::Main)]);
        menu_application.menu_bar = bar();
        assert_eq!(
            validate_application(&menu_application),
            Err(WinUiDiagnostic::UnsupportedMenuBar {
                scope: "application".to_owned(),
            })
        );

        let mut menu_window = window("main", WindowKind::Main);
        menu_window.content = WindowContent::from(
            column([label("main").with_key("title")])
                .with_key("root")
                .menu_bar(bar()),
        );
        assert_eq!(
            validate_application(&application(vec![menu_window])),
            Err(WinUiDiagnostic::UnsupportedMenuBar {
                scope: "main".to_owned(),
            })
        );
    }

    #[test]
    fn content_context_menus_are_a_typed_diagnostic() {
        let mut with_menu = window("main", WindowKind::Main);
        with_menu.content =
            WindowContent::from(label("File").context_menu([rinka_core::MenuEntry::item(
                rinka_core::MenuItem::new("open", "Open", || {}),
            )]));
        assert_eq!(
            validate_application(&application(vec![with_menu])),
            Err(WinUiDiagnostic::UnsupportedContentCapability {
                window_id: "main".to_owned(),
                capability: "context menu",
            })
        );
    }

    #[test]
    fn drag_and_drop_declarations_are_a_typed_diagnostic() {
        let mut with_drop_target = window("main", WindowKind::Main);
        with_drop_target.content =
            WindowContent::from(column([label("drop zone")]).on_file_drop(|_| {}));
        assert_eq!(
            validate_application(&application(vec![with_drop_target])),
            Err(WinUiDiagnostic::UnsupportedContentCapability {
                window_id: "main".to_owned(),
                capability: "drag and drop",
            })
        );

        let mut with_drag_source = window("main", WindowKind::Main);
        with_drag_source.content = WindowContent::from(rinka_core::list(
            "Files",
            [
                rinka_core::list_row("readme", None, None, false, false, "readme", || {})
                    .drag_payload(rinka_core::DragPayload::new("demo.file", "readme")),
            ],
        ));
        assert_eq!(
            validate_application(&application(vec![with_drag_source])),
            Err(WinUiDiagnostic::UnsupportedContentCapability {
                window_id: "main".to_owned(),
                capability: "drag and drop",
            })
        );
    }

    #[test]
    fn workspace_visibility_only_hides_regions_declared_collapsible() {
        let fixed = resolve_workspace_visibility(false, false, false, false, 520.0);
        assert!(fixed.sidebar_open);
        assert!(fixed.inspector_open);

        let narrow = resolve_workspace_visibility(true, false, true, false, 760.0);
        assert!(!narrow.sidebar_open);
        assert!(!narrow.inspector_open);

        let narrow_requested = resolve_workspace_visibility(true, true, true, true, 760.0);
        assert!(narrow_requested.sidebar_open);
        assert!(narrow_requested.inspector_open);

        let wide = resolve_workspace_visibility(true, true, true, true, 1120.0);
        assert!(wide.sidebar_open);
        assert!(wide.inspector_open);
    }

    #[test]
    fn the_clipboard_capability_is_a_typed_unsupported_diagnostic() {
        use rinka_core::ClipboardError;
        use std::cell::RefCell;
        use std::rc::Rc;

        let services = super::platform_services();
        assert_eq!(
            services.clipboard().write_text("never stored"),
            Err(ClipboardError::Unsupported {
                platform: super::PLATFORM_NAME,
            })
        );
        let delivered = Rc::new(RefCell::new(None));
        let sink = delivered.clone();
        services
            .clipboard()
            .read_text(move |result| *sink.borrow_mut() = Some(result));
        assert_eq!(
            delivered.borrow_mut().take(),
            Some(Err(ClipboardError::Unsupported {
                platform: super::PLATFORM_NAME,
            }))
        );
    }
}
