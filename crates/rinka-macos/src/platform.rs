//! Main-thread AppKit implementation.

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{MainThreadMarker, NSObject};
use rinka_core::{
    AcceleratorOutcome, AcceleratorRouter, Align, ApplicationSpec, Axis, ButtonMaterial,
    ButtonRole, CanvasColor, CanvasPoint, CanvasRect, CanvasSize, CanvasVector, CollectionPattern,
    ControlSize, DrawCommand, DrawScene, Element, ElementKind, EventBindings, ImageContent,
    ImageScaling, InputKind, Justify, KeyChord, KeyIdentity, KeyRoutingContext, LineWidth,
    ListRowRole, MenuEntry, MenuItem, Modifiers, MonospaceMetrics, MountedNode, NativeBackend,
    PanelBehavior, PatternRegion, PointerButton, PointerEvent, PointerModifiers, PointerPhase,
    PrimaryModifier, PropertyPatch, Props, Renderer, SortDirection, Spacing, StatusTone, Symbol,
    TableColumn, TableSort, TextRole, ToolbarAction, ToolbarDisplay, ToolbarGroupDisplay,
    ToolbarItem, ToolbarItemKind, ToolbarPlacement, UiPattern, WindowId, WindowKind, WindowRuntime,
    WindowSpec,
};
use std::cell::{Cell, RefCell};
use std::error::Error;
use std::ffi::{CStr, c_char};
use std::fmt;
use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

mod application;
pub use application::run;

include!("platform/native_runtime.rs");
include!("platform/canvas_surface.rs");
include!("platform/key_dispatch.rs");
include!("platform/collection_delegate.rs");
include!("platform/backend.rs");
include!("platform/collection_mount.rs");
include!("platform/layout_primitives.rs");
include!("platform/stack_layout.rs");
include!("platform/reconciliation.rs");
include!("platform/native_metrics.rs");
include!("platform/image_display.rs");
