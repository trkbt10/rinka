//! Windows Server 2025 native host built from Win32 and Common Controls v6.

use rinka_core::{ButtonMaterial, Element, ElementKind, Props};
use std::error::Error;
use std::fmt;

/// Identifies this platform adapter in diagnostics.
pub const PLATFORM_NAME: &str = "Windows Win32 + Common Controls v6";

/// A typed rejection or native-operation failure from the Windows adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WindowsDiagnostic {
    /// A declared semantic capability has no equivalent native Windows control.
    UnsupportedCapability {
        /// Element that requested the capability.
        element: ElementKind,
        /// Stable capability identifier.
        capability: &'static str,
    },
    /// A declared toolbar capability has no equivalent native Windows control.
    UnsupportedToolbarCapability {
        /// Stable capability identifier.
        capability: &'static str,
    },
    /// A Win32 operation returned an operating-system error.
    NativeOperation {
        /// Operation that failed.
        operation: &'static str,
        /// `GetLastError` value captured at the failure site.
        code: u32,
    },
    /// A Windows resource or element relationship was internally inconsistent.
    InvalidNativeState {
        /// Explanation suitable for a build or verification log.
        reason: String,
    },
}

impl fmt::Display for WindowsDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedCapability {
                element,
                capability,
            } => write!(
                formatter,
                "Windows adapter does not implement {capability} for {element:?}"
            ),
            Self::UnsupportedToolbarCapability { capability } => write!(
                formatter,
                "Windows adapter does not implement {capability} for the toolbar"
            ),
            Self::NativeOperation { operation, code } => {
                write!(
                    formatter,
                    "Win32 operation {operation} failed with code {code}"
                )
            }
            Self::InvalidNativeState { reason } => formatter.write_str(reason),
        }
    }
}

impl Error for WindowsDiagnostic {}

/// Validates one element against native Windows semantic capabilities.
pub fn validate_element(element: &Element) -> Result<(), WindowsDiagnostic> {
    if !element.accelerator_table().is_empty() {
        // The Win32 contract probe has no accelerator-table delivery
        // (reports/keyboard-shortcuts-and-key-events); the declared table is
        // rejected instead of silently dropped.
        return Err(WindowsDiagnostic::UnsupportedCapability {
            element: element.kind(),
            capability: "declared accelerator table",
        });
    }
    if let Props::Button {
        material: ButtonMaterial::Glass,
        ..
    } = element.props()
    {
        return Err(WindowsDiagnostic::UnsupportedCapability {
            element: ElementKind::Button,
            capability: "glass button material",
        });
    }
    if element.kind() == ElementKind::Canvas {
        return Err(WindowsDiagnostic::UnsupportedCapability {
            element: ElementKind::Canvas,
            capability: "owned-drawing canvas surface",
        });
    }
    if element.kind() == ElementKind::Image {
        // The Win32 contract probe has no bitmap image realization yet; per
        // the AGENTS contract it rejects the capability instead of
        // substituting an unrelated control.
        return Err(WindowsDiagnostic::UnsupportedCapability {
            element: ElementKind::Image,
            capability: "bitmap image element",
        });
    }
    if element.context_menu_model().is_some() {
        // The classic Win32 probe has no context-menu realization yet; the
        // typed rejection and its follow-up are recorded in
        // reports/context-menus.
        return Err(WindowsDiagnostic::UnsupportedCapability {
            element: element.kind(),
            capability: "context menu",
        });
    }
    Ok(())
}

#[cfg(target_os = "windows")]
mod platform;

#[cfg(target_os = "windows")]
pub use platform::{WindowsHandle, run};

#[cfg(not(target_os = "windows"))]
/// Reports a programming error when invoked on another operating system.
pub fn run(_application: rinka_core::ApplicationSpec) -> Result<(), WindowsDiagnostic> {
    Err(WindowsDiagnostic::UnsupportedCapability {
        element: ElementKind::Pattern,
        capability: "Windows desktop process",
    })
}

#[cfg(test)]
mod tests {
    use super::{WindowsDiagnostic, validate_element};
    use rinka_core::{ButtonMaterial, ElementKind, button};

    #[test]
    fn glass_material_is_a_typed_diagnostic() {
        let element = button("Action", "Action", || {}).button_material(ButtonMaterial::Glass);
        assert_eq!(
            validate_element(&element),
            Err(WindowsDiagnostic::UnsupportedCapability {
                element: ElementKind::Button,
                capability: "glass button material",
            })
        );
    }

    #[test]
    fn ordinary_native_button_is_supported() {
        assert_eq!(validate_element(&button("Action", "Action", || {})), Ok(()));
    }

    #[test]
    fn canvas_is_a_typed_unsupported_capability() {
        let element = rinka_core::canvas(
            rinka_core::CanvasSize::new(32.0, 32.0),
            rinka_core::DrawScene::new(),
            "Level meter",
        );
        assert_eq!(
            validate_element(&element),
            Err(WindowsDiagnostic::UnsupportedCapability {
                element: ElementKind::Canvas,
                capability: "owned-drawing canvas surface",
            })
        );
    }

    #[test]
    fn bitmap_image_content_is_a_typed_diagnostic() {
        let content = rinka_core::ImageContent::from_rgba8(2, 2, 8, vec![0_u8; 32], 1);
        let element = rinka_core::image(content, "Preview");
        assert_eq!(
            validate_element(&element),
            Err(WindowsDiagnostic::UnsupportedCapability {
                element: ElementKind::Image,
                capability: "bitmap image element",
            })
        );
    }

    #[test]
    fn declared_accelerator_tables_are_a_typed_diagnostic() {
        use rinka_core::{Accelerator, column, label};
        let element = column([label("main").with_key("title")])
            .with_key("root")
            .accelerators([Accelerator::new(
                "save",
                "Primary+S".parse().expect("test chord"),
                || {},
            )]);
        assert_eq!(
            validate_element(&element),
            Err(WindowsDiagnostic::UnsupportedCapability {
                element: ElementKind::Stack,
                capability: "declared accelerator table",
            })
        );
    }

    #[test]
    fn a_context_menu_is_a_typed_diagnostic() {
        let element =
            button("Action", "Action", || {}).context_menu([rinka_core::MenuEntry::item(
                rinka_core::MenuItem::new("open", "Open", || {}),
            )]);
        assert_eq!(
            validate_element(&element),
            Err(WindowsDiagnostic::UnsupportedCapability {
                element: ElementKind::Button,
                capability: "context menu",
            })
        );
    }
}
