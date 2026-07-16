//! Main-thread AppKit implementation.

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{MainThreadMarker, NSObject};
use rinka_core::{
    Align, ApplicationSpec, Axis, ButtonMaterial, ButtonRole, CollectionPattern, ControlSize,
    Element, ElementKind, EventBindings, InputKind, Justify, ListRowRole, MountedNode,
    NativeBackend, PanelBehavior, PatternRegion, PropertyPatch, Props, Renderer, SortDirection,
    Spacing, StatusTone, Symbol, TableColumn, TableSort, TextRole, ToolbarAction, ToolbarDisplay,
    ToolbarGroupDisplay, ToolbarItem, ToolbarItemKind, ToolbarMenuEntry, ToolbarPlacement,
    UiPattern, WindowKind, WindowRuntime, WindowSpec,
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
include!("platform/collection_delegate.rs");
include!("platform/backend.rs");
include!("platform/collection_mount.rs");
include!("platform/layout_primitives.rs");
include!("platform/stack_layout.rs");
include!("platform/reconciliation.rs");
include!("platform/native_metrics.rs");
