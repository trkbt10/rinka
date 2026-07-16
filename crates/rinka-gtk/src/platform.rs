//! GTK 4 and libadwaita implementation.

use adw::prelude::*;
use gtk::{gio, glib};
use rinka_core::{
    Align, ApplicationSpec, Axis, ButtonMaterial, ButtonRole, CollectionPattern, ControlSize,
    Element, ElementKind, EventBindings, InputKind, Justify, ListRowRole, MountedNode,
    NativeBackend, PanelBehavior, PatternRegion, PropertyPatch, Props, Renderer, SortDirection,
    Spacing, StatusTone, Symbol, TableColumn, TableSort, TextRole, ToolbarAction, ToolbarDisplay,
    ToolbarGroupDisplay, ToolbarItem, ToolbarItemKind, ToolbarMenuEntry, ToolbarPlacement,
    UiPattern, WindowKind, WindowRuntime, WindowSpec,
};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::error::Error;
use std::fmt;
use std::rc::Rc;

include!("platform/model.rs");
include!("platform/backend.rs");
include!("platform/mount.rs");
include!("platform/window.rs");
