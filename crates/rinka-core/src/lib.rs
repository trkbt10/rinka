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
mod semantics;
mod toolbar;
mod validation;
mod window;

pub use backend::{NativeBackend, PropertyPatch};
pub use element::{
    Element, Key, button, column, input, label, list, list_row, progress, row, scroll, separator,
    spacer, split, status, toggle, workspace,
};
pub use event::{ActivateHandler, EventBindings, InputHandler, SortHandler, ToggleHandler};
pub use projection::{ProjectedHandle, WindowProjection};
pub use reconcile::{MountedNode, RenderError, RenderStats, Renderer};
pub use runtime::{AppRuntime, Component, Dispatch, WindowRuntime};
pub use semantics::{
    Align, Axis, ButtonMaterial, ButtonRole, ControlSize, ElementKind, InputKind, Justify,
    ListRowRole, ListStyle, Props, SortDirection, Spacing, SplitRole, StatusTone, Symbol,
    TableColumn, TableSort, TextRole,
};
pub use toolbar::{
    ToolbarAction, ToolbarChoice, ToolbarDisplay, ToolbarGroupDisplay, ToolbarItem,
    ToolbarItemKind, ToolbarMenuEntry, ToolbarPlacement,
};
pub use validation::TreeError;
pub use window::{
    ApplicationSpec, PanelBehavior, RenderContext, Size, WindowContent, WindowId, WindowKind,
    WindowSpec,
};
