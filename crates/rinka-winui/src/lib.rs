//! Native WinUI 3 projection of Rinka's declarative application model.
//!
//! The adapter keeps the common component runtime authoritative and translates
//! its retained mounted tree into native WinUI controls. Windows App SDK
//! runtime staging belongs to the executable package that calls [`run`].

use rinka_core::{ApplicationSpec, PanelBehavior, WindowKind};
use std::error::Error;
use std::fmt;

#[cfg(target_os = "windows")]
mod platform;

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
    /// A panel requested behavior that the WinUI host cannot represent.
    UnsupportedPanelBehavior {
        /// Stable panel identity.
        window_id: String,
        /// Human-readable unsupported field.
        field: &'static str,
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
            Self::UnsupportedPanelBehavior { window_id, field } => write!(
                formatter,
                "panel '{window_id}' requests unsupported behavior '{field}'"
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
    }
    match main_windows {
        0 => Err(WinUiDiagnostic::MissingMainWindow),
        1 => Ok(()),
        _ => Err(WinUiDiagnostic::MultipleMainWindows),
    }
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
        ApplicationSpec, PanelBehavior, Size, ToolbarDisplay, WindowContent, WindowId, WindowKind,
        WindowSpec, label,
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
}
