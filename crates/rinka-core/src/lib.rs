//! Platform-neutral declarative UI contracts.
//!
//! The core stores descriptions and native handles separately. It never draws
//! controls and never imports an operating-system toolkit.

mod backend;
mod element;
mod event;
mod projection;
mod reconcile;
mod runtime;
mod window;

pub use backend::{NativeBackend, PropertyPatch};
pub use element::{
    Align, Axis, ButtonMaterial, ButtonRole, ControlSize, Element, ElementKind, InputKind, Justify,
    Key, ListRowRole, ListStyle, Props, SortDirection, Spacing, SplitRole, StatusTone, Symbol,
    TableColumn, TableSort, TextRole, button, column, input, label, list, list_row, progress, row,
    scroll, separator, spacer, split, status, toggle, workspace,
};
pub use event::{ActivateHandler, EventBindings, InputHandler, SortHandler, ToggleHandler};
pub use projection::{ProjectedHandle, WindowProjection};
pub use reconcile::{MountedNode, RenderError, RenderStats, Renderer, TreeError};
pub use runtime::{AppRuntime, Component, Dispatch, WindowRuntime};
pub use window::{
    ApplicationSpec, PanelBehavior, RenderContext, Size, ToolbarAction, ToolbarChoice,
    ToolbarDisplay, ToolbarGroupDisplay, ToolbarItem, ToolbarItemKind, ToolbarMenuEntry,
    ToolbarPlacement, WindowContent, WindowId, WindowKind, WindowSpec,
};
