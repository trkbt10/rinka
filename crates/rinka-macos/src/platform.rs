//! Main-thread AppKit implementation.

use objc2::rc::{Retained, Weak as ObjcWeak, autoreleasepool};
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{ClassType, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{MainThreadMarker, NSNotFound, NSObject, NSRange};
use rinka_core::{
    AcceleratorOutcome, AcceleratorRouter, Align, ApplicationSpec, Axis, ButtonMaterial,
    ButtonRole, CanvasColor, CanvasPoint, CanvasRect, CanvasSize, CanvasVector, CollectionPattern,
    ContextMenu, ControlSize, DialogButtonRole, DialogDescription, DialogOutcome, DialogRequest,
    DialogResponder, DialogService, DockEdge, DockEvent, DockGroup, DockLayout, DockNode, DockTab,
    DockTabMenus, DragPayload, DrawCommand, DrawScene, DropPosition, DropTarget, Element,
    ElementKind, EventBindings, FileDrop, HighlightRole, HighlightSpan, HighlightSpans,
    ImageContent, ImageScaling, ImeEvent, InputKind, Justify, KeyChord, KeyEvent, KeyIdentity,
    KeyRoutingContext, LastWindowClosedPolicy, LineWidth, ListRowRole, MenuBar, MenuBarActivation,
    MenuBarBindings, MenuBarEntry, MenuBarMenuRole, MenuBarRouter, MenuBarUpdate, MenuEntry,
    MenuItem, Modifiers, MonospaceMetrics, MountedNode, NativeBackend, PanelBehavior,
    PatternRegion, PayloadDrop, PointerButton, PointerEvent, PointerModifiers, PointerPhase,
    PreeditCaret, PrimaryModifier, PropertyPatch, Props, Renderer, Size as LogicalSize,
    SortDirection, Spacing, StandardItem, StatusTone, Symbol, TableColumn, TableSort, TextChange,
    TextContent, TextEdit, TextRange, TextRevision, TextRole, TextSelection, TextSyncAction,
    ToolbarAction, ToolbarDisplay, ToolbarGroupDisplay, ToolbarItem, ToolbarItemKind,
    ToolbarPlacement, UiPattern, WindowBindings, WindowError, WindowEvent, WindowId, WindowKind,
    WindowPosition, WindowRuntime, WindowService, WindowSpec,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::error::Error;
use std::ffi::{CStr, c_char};
use std::fmt;
use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

mod application;
pub use application::run;
pub use application::{AppKitTestHost, SettleObservation, window_server_session_available};

include!("platform/native_runtime.rs");
include!("platform/text_editing.rs");
include!("platform/canvas_surface.rs");
include!("platform/key_dispatch.rs");
include!("platform/menu_bar_host.rs");
include!("platform/pasteboard.rs");
include!("platform/drag_drop.rs");
include!("platform/dock_area.rs");
include!("platform/collection_delegate.rs");
include!("platform/backend.rs");
include!("platform/collection_mount.rs");
include!("platform/layout_primitives.rs");
include!("platform/stack_layout.rs");
include!("platform/reconciliation.rs");
include!("platform/native_metrics.rs");
include!("platform/image_display.rs");
