use crate::{WinUiDiagnostic, resolve_workspace_visibility};
use rinka_core::{
    Align, ApplicationSpec, Axis, ButtonRole, CollectionPattern, ControlSize, ElementKind,
    EventBindings, InputKind, Justify, ListRowRole, MountedNode, ProjectedHandle, Props,
    SortDirection, Spacing, StatusTone, Symbol as CommonSymbol, TableColumn, TableSort, TextRole,
    ToolbarAction, ToolbarItemKind, ToolbarMenuEntry, UiPattern, WindowKind, WindowProjection,
    WindowSpec,
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

include!("platform/window_host.rs");
include!("platform/toolbar.rs");
include!("platform/elements.rs");
include!("platform/table_status.rs");
