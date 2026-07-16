//! Platform-neutral declarative UI contracts.
//!
//! The core stores descriptions and native handles separately. It never draws
//! controls and never imports an operating-system toolkit.

mod accelerator;
mod backend;
mod canvas;
mod chord;
mod clipboard;
mod element;
mod event;
mod menu;
mod pattern;
mod projection;
mod reconcile;
mod runtime;
mod semantics;
mod services;
mod text_editing;
mod toolbar;
mod validation;
mod window;

pub use accelerator::{
    Accelerator, AcceleratorBindings, AcceleratorDescription, AcceleratorOutcome,
    AcceleratorRouter, AcceleratorScope, KeyRoutingContext,
};
pub use backend::{NativeBackend, PropertyPatch};
pub use canvas::{
    CanvasColor, CanvasPoint, CanvasRect, CanvasSize, CanvasVector, DrawCommand, DrawScene,
    LineWidth, MonospaceMetrics, PointerButton, PointerEvent, PointerModifiers, PointerPhase,
};
pub use chord::{
    ChordParseError, KeyChord, KeyIdentity, Modifiers, PrimaryModifier, ResolvedModifiers,
};
pub use clipboard::{Clipboard, ClipboardError, ClipboardFlavor, ClipboardService};
pub use element::{
    Element, Key, button, canvas, column, image, input, label, list, list_row, mount_pattern,
    progress, row, scroll, separator, spacer, status, text_area, toggle,
};
pub use event::{
    ActivateHandler, EventBindings, InputHandler, PointerHandler, SelectionChangeHandler,
    SortHandler, TextChangeHandler, ToggleHandler,
};
pub use menu::{ContextMenu, MenuEntry, MenuItem, MenuItemRole, Submenu};
pub use pattern::{PatternRegion, UiPattern};
pub use projection::{ProjectedHandle, WindowProjection};
pub use reconcile::{MountedNode, RenderError, RenderStats, Renderer};
pub use runtime::{AppRuntime, Component, Dispatch, UpdateContext, WindowRuntime};
pub use semantics::{
    Align, Axis, ButtonMaterial, ButtonRole, CollectionPattern, ControlSize, ElementKind,
    ImageContent, ImageScaling, InputKind, Justify, ListRowRole, Props, SortDirection, Spacing,
    StatusTone, Symbol, TableColumn, TableSort, TextRole,
};
pub use services::PlatformServices;
pub use text_editing::{
    HighlightRole, HighlightSpan, HighlightSpans, TextChange, TextContent, TextEdit, TextRange,
    TextRevision, TextSelection, TextSyncAction, char_range_to_byte_range,
};
pub use toolbar::{
    ToolbarAction, ToolbarChoice, ToolbarDisplay, ToolbarGroupDisplay, ToolbarItem,
    ToolbarItemKind, ToolbarPlacement,
};
pub use validation::TreeError;
pub use window::{
    ApplicationSpec, PanelBehavior, RenderContext, Size, WindowContent, WindowId, WindowKind,
    WindowSpec,
};
