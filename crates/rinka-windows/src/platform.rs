//! Win32 retained-object implementation.

use crate::{WindowsDiagnostic, validate_element};
use rinka_core::{
    Align, ApplicationSpec, Axis, ButtonRole, ControlSize, Element, ElementKind, EventBindings,
    InputKind, Justify, ListRowRole, ListStyle, NativeBackend, PanelBehavior, PropertyPatch, Props,
    Renderer, SortDirection, Spacing, SplitRole, StatusTone, Symbol, TableSort, TextRole,
    ToolbarAction, ToolbarDisplay, ToolbarItem, ToolbarItemKind, ToolbarMenuEntry, WindowKind,
    WindowRuntime, WindowSpec,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::fmt;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use windows::Win32::Foundation::HWND as WindowsHwnd;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CAccPropServices, IAccPropServices, Name_Property_GUID, PROPID_ACC_NAME,
};
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    GetThreadDpiAwarenessContext, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{CHILDID_SELF, OBJID_CLIENT};
use windows::core::PCWSTR;
use windows_sys::Win32::Foundation::{
    ERROR_SUCCESS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, CreateSolidBrush, DeleteObject, FillRect, HBRUSH, HDC, HFONT, HGDIOBJ,
    SetBkColor, SetTextColor, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::Controls::{
    ICC_BAR_CLASSES, ICC_LISTVIEW_CLASSES, ICC_PROGRESS_CLASS, ICC_TREEVIEW_CLASSES,
    INITCOMMONCONTROLSEX, InitCommonControlsEx, SetWindowTheme, TTF_IDISHWND, TTF_SUBCLASS,
    TTM_ADDTOOLW, TTS_ALWAYSTIP, TTTOOLINFOW,
};
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForSystem, SystemParametersInfoForDpi,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetActiveWindow, VK_RETURN, VK_SPACE,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClassNameW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, HMENU, IDC_ARROW,
    IDI_APPLICATION, IsDialogMessageW, IsWindow, LoadCursorW, LoadIconW, MF_GRAYED, MF_SEPARATOR,
    MF_STRING, MINMAXINFO, MSG, MoveWindow, NONCLIENTMETRICSW, PostQuitMessage, RegisterClassExW,
    SPI_GETNONCLIENTMETRICS, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SW_SHOWNORMAL, SetParent,
    SetWindowLongPtrW, SetWindowTextW, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WM_ACTIVATEAPP, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DPICHANGED,
    WM_GETMINMAXINFO, WM_KEYUP, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NCCREATE, WM_NCDESTROY,
    WM_SETFONT, WM_SIZE, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_GROUP, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_THICKFRAME,
    WS_VISIBLE, WS_VSCROLL,
};

const CLASS_NAME: &str = "Rinka.Window.Server2025";
const STATIC_CLASS: &str = "STATIC";
const BUTTON_CLASS: &str = "BUTTON";
const EDIT_CLASS: &str = "EDIT";
const LIST_VIEW_CLASS: &str = "SysListView32";
const TREE_VIEW_CLASS: &str = "SysTreeView32";
const PROGRESS_CLASS: &str = "msctls_progress32";
const TOOLTIP_CLASS: &str = "tooltips_class32";

const BS_PUSHBUTTON: u32 = 0;
const BS_DEFPUSHBUTTON: u32 = 1;
const BS_AUTOCHECKBOX: u32 = 3;
const BS_AUTORADIOBUTTON: u32 = 9;
const BS_FLAT: u32 = 0x8000;
const BS_PUSHLIKE: u32 = 0x1000;
const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_PASSWORD: u32 = 0x0020;
const ES_SEARCH: u32 = 0x0080;
const ES_READONLY: u32 = 0x0800;
const SS_LEFT: u32 = 0;
const SS_CENTER: u32 = 1;
const SS_ETCHEDHORZ: u32 = 0x10;
const SS_ETCHEDVERT: u32 = 0x11;
const SS_NOTIFY: u32 = 0x0100;
const LVS_REPORT: u32 = 0x0001;
const LVS_LIST: u32 = 0x0003;
const LVS_SINGLESEL: u32 = 0x0004;
const LVS_SHOWSELALWAYS: u32 = 0x0008;
const LVS_NOSORTHEADER: u32 = 0x8000;
const LVS_EX_FULLROWSELECT: usize = 0x20;
const LVS_EX_DOUBLEBUFFER: usize = 0x10000;
const TVS_HASBUTTONS: u32 = 0x0001;
const TVS_HASLINES: u32 = 0x0002;
const TVS_LINESATROOT: u32 = 0x0004;
const TVS_SHOWSELALWAYS: u32 = 0x0020;
const TVS_FULLROWSELECT: u32 = 0x1000;
const PBS_SMOOTH: u32 = 0x01;
const LVM_FIRST: u32 = 0x1000;
const LVM_DELETEALLITEMS: u32 = LVM_FIRST + 9;
const LVM_GETNEXTITEM: u32 = LVM_FIRST + 12;
const LVM_DELETECOLUMN: u32 = LVM_FIRST + 28;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = LVM_FIRST + 54;
const LVM_INSERTCOLUMNW: u32 = LVM_FIRST + 97;
const LVM_INSERTITEMW: u32 = LVM_FIRST + 77;
const LVM_GETITEMW: u32 = LVM_FIRST + 75;
const LVM_SETITEMTEXTW: u32 = LVM_FIRST + 116;
const LVM_SETCOLUMNWIDTH: u32 = LVM_FIRST + 30;
const TV_FIRST: u32 = 0x1100;
const TVM_DELETEITEM: u32 = TV_FIRST + 1;
const TVM_INSERTITEMW: u32 = TV_FIRST + 50;
const TVM_EXPAND: u32 = TV_FIRST + 2;
const TVM_GETNEXTITEM: u32 = TV_FIRST + 10;
const TVM_GETITEMW: u32 = TV_FIRST + 62;
const TVE_EXPAND: usize = 0x0002;
const TVGN_CARET: usize = 0x0009;
const LVNI_SELECTED: isize = 0x0002;
const TVI_ROOT: isize = -0x10000;
const TVI_LAST: isize = -0x0fffe;
const PBM_SETPOS: u32 = 0x0402;
const BM_SETCHECK: u32 = 0x00f1;
const BM_GETCHECK: u32 = 0x00f0;
const BST_CHECKED: usize = 1;
const EM_SETCUEBANNER: u32 = 0x1501;
const LVCF_FMT: u32 = 0x0001;
const LVCF_WIDTH: u32 = 0x0002;
const LVCF_TEXT: u32 = 0x0004;
const LVCF_SUBITEM: u32 = 0x0008;
const LVIF_TEXT: u32 = 0x0001;
const LVIF_PARAM: u32 = 0x0004;
const LVIF_STATE: u32 = 0x0008;
const LVIF_INDENT: u32 = 0x0010;
const LVIS_SELECTED: u32 = 0x0002;
const TVIF_TEXT: u32 = 0x0001;
const TVIF_PARAM: u32 = 0x0004;
const TVIF_STATE: u32 = 0x0008;
const TVIF_CHILDREN: u32 = 0x0040;
const TVIS_SELECTED: u32 = 0x0002;
const TVIS_BOLD: u32 = 0x0010;
const EN_CHANGE: u16 = 0x0300;
const BN_CLICKED: u16 = 0;
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const WM_CTLCOLORBTN: u32 = 0x0135;
const WM_CTLCOLOREDIT: u32 = 0x0133;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const WM_ERASE_BACKGROUND: u32 = 0x0014;
const DARK_BACKGROUND: u32 = 0x0020_2020;
const DARK_TEXT: u32 = 0x00f0_f0f0;
const WM_NOTIFY_MESSAGE: u32 = 0x004e;
const TOOLBAR_HEIGHT: i32 = 52;
const CONTROL_ID_FIRST: usize = 100;
const LVN_ITEMCHANGED: i32 = -101;
const LVN_COLUMNCLICK: i32 = -108;
const TVN_SELCHANGEDW: i32 = -451;
const TVN_ITEMEXPANDEDW: i32 = -455;

std::thread_local! {
    static TOP_LEVEL_WINDOWS: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
    static MESSAGE_LOOP_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static ACCESSIBILITY_SERVICE: RefCell<Option<IAccPropServices>> = const { RefCell::new(None) };
}

#[repr(C)]
struct NmHdr {
    hwnd_from: HWND,
    id_from: usize,
    code: u32,
}

#[repr(C)]
struct NmListView {
    header: NmHdr,
    item: i32,
    sub_item: i32,
    new_state: u32,
    old_state: u32,
    changed: u32,
    point_x: i32,
    point_y: i32,
    l_param: LPARAM,
}

#[repr(C)]
struct NmTreeViewW {
    header: NmHdr,
    action: u32,
    item_old: TvItemExW,
    item_new: TvItemExW,
    point_x: i32,
    point_y: i32,
}

#[repr(C)]
struct LvItemW {
    mask: u32,
    i_item: i32,
    i_sub_item: i32,
    state: u32,
    state_mask: u32,
    psz_text: *mut u16,
    cch_text_max: i32,
    i_image: i32,
    l_param: LPARAM,
    i_indent: i32,
    i_group_id: i32,
    c_columns: u32,
    pu_columns: *mut u32,
    pi_col_fmt: *mut i32,
    i_group: i32,
}

#[repr(C)]
struct LvColumnW {
    mask: u32,
    fmt: i32,
    cx: i32,
    psz_text: *mut u16,
    cch_text_max: i32,
    i_sub_item: i32,
    i_image: i32,
    i_order: i32,
    cx_min: i32,
    cx_default: i32,
    cx_ideal: i32,
}

#[repr(C)]
struct TvItemExW {
    mask: u32,
    item: isize,
    state: u32,
    state_mask: u32,
    psz_text: *mut u16,
    cch_text_max: i32,
    i_image: i32,
    i_selected_image: i32,
    c_children: i32,
    l_param: LPARAM,
    i_integral: i32,
    state_ex: u32,
    hwnd: HWND,
    i_expanded_image: i32,
    i_reserved: i32,
}

#[repr(C)]
struct TvInsertStructW {
    parent: isize,
    insert_after: isize,
    item: TvItemExW,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostKind {
    Root,
    Element(ElementKind),
}

#[derive(Clone)]
struct RowState {
    owner: Cell<HWND>,
    list_index: Cell<i32>,
    tree_item: Cell<isize>,
}

struct HandleInner {
    hwnd: HWND,
    kind: HostKind,
    props: RefCell<Option<Props>>,
    children: RefCell<Vec<WindowsHandle>>,
    semantic_font: RefCell<Option<Rc<NativeFont>>>,
    events: EventBindings,
    row: Option<RowState>,
    is_root: bool,
    subclassed: Cell<bool>,
    list_rebuilding: Cell<bool>,
    dark: bool,
    background_brush: HBRUSH,
}

struct NativeFont(HFONT);

impl Drop for NativeFont {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the font is created by CreateFontIndirectW and owned by this wrapper.
            unsafe {
                let _ = DeleteObject(self.0 as HGDIOBJ);
            }
        }
    }
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        ACCESSIBILITY_SERVICE.with(|service| {
            let _ = service.borrow_mut().take();
        });
        // SAFETY: this guard is created only after successful CoInitializeEx on this thread.
        unsafe {
            CoUninitialize();
        }
    }
}

struct PendingTopLevel(HWND);

impl Drop for PendingTopLevel {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this HWND has not yet been handed to the message-loop registry.
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}

impl Drop for HandleInner {
    fn drop(&mut self) {
        // SAFETY: the subclass identity is removed while its reference data still points
        // to this allocation, and `IsWindow` prevents destruction of an expired HWND.
        unsafe {
            if self.subclassed.get() {
                let _ = RemoveWindowSubclass(self.hwnd, Some(element_subclass), 1);
            }
            if !self.background_brush.is_null() {
                let _ = DeleteObject(self.background_brush as HGDIOBJ);
            }
            if !self.is_root && IsWindow(self.hwnd) != 0 {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// Retained Windows identity associated with one declarative element.
#[derive(Clone)]
pub struct WindowsHandle(Rc<HandleInner>);

impl WindowsHandle {
    fn new(
        hwnd: HWND,
        kind: HostKind,
        props: Option<Props>,
        events: EventBindings,
        is_root: bool,
        dark: bool,
    ) -> Self {
        let row = (kind == HostKind::Element(ElementKind::ListRow)).then(|| RowState {
            owner: Cell::new(null_mut()),
            list_index: Cell::new(-1),
            tree_item: Cell::new(0),
        });
        // SAFETY: the brush is retained until the last handle clone is dropped.
        let background_brush = if dark {
            unsafe { CreateSolidBrush(DARK_BACKGROUND) }
        } else {
            null_mut()
        };
        let handle = Self(Rc::new(HandleInner {
            hwnd,
            kind,
            props: RefCell::new(props),
            children: RefCell::new(Vec::new()),
            semantic_font: RefCell::new(None),
            events,
            row,
            is_root,
            subclassed: Cell::new(false),
            list_rebuilding: Cell::new(false),
            dark,
            background_brush,
        }));
        if !hwnd.is_null() {
            // SAFETY: `Rc::as_ptr` is stable until the subclass is removed in `Drop`.
            unsafe {
                if SetWindowSubclass(
                    hwnd,
                    Some(element_subclass),
                    1,
                    Rc::as_ptr(&handle.0) as usize,
                ) != 0
                {
                    handle.0.subclassed.set(true);
                }
            }
        }
        handle
    }

    /// Returns the raw HWND identity for diagnostics and native integration.
    pub fn hwnd(&self) -> isize {
        self.0.hwnd as isize
    }

    /// Returns the registered native window class name.
    pub fn native_class_name(&self) -> String {
        class_name(self.0.hwnd)
    }
}

impl fmt::Debug for WindowsHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsHandle")
            .field("hwnd", &self.hwnd())
            .field("kind", &self.0.kind)
            .field("native_class", &self.native_class_name())
            .finish()
    }
}

/// Retained Win32 backend for one top-level window.
pub struct WindowsBackend {
    root: WindowsHandle,
    font: Rc<NativeFont>,
    dpi: Cell<u32>,
    dark: bool,
    sidebar_visible: Cell<bool>,
    inspector_visible: Cell<bool>,
}

impl WindowsBackend {
    fn new(root: HWND, dpi: u32, font: Rc<NativeFont>, dark: bool) -> Self {
        Self {
            root: WindowsHandle::new(
                root,
                HostKind::Root,
                None,
                EventBindings::default(),
                true,
                dark,
            ),
            font,
            dpi: Cell::new(dpi),
            dark,
            sidebar_visible: Cell::new(true),
            inspector_visible: Cell::new(true),
        }
    }

    fn set_dpi(&mut self, dpi: u32, font: Rc<NativeFont>) {
        self.dpi.set(dpi.max(96));
        self.font = font;
        if apply_semantic_font_tree(&self.root, self.dpi.get(), self.font.0).is_err() {
            clear_semantic_fonts(&self.root, self.font.0);
        }
    }

    fn layout_root(&self) {
        let mut rect = RECT::default();
        // SAFETY: root is a live child HWND while the window runtime owns this backend.
        unsafe {
            let _ = GetClientRect(self.root.0.hwnd, &mut rect);
        }
        if let Some(child) = self.root.0.children.borrow().first() {
            self.layout_handle(child, 0, 0, rect.right - rect.left, rect.bottom - rect.top);
        }
    }

    fn layout_handle(&self, handle: &WindowsHandle, x: i32, y: i32, width: i32, height: i32) {
        move_window(handle.0.hwnd, x, y, width.max(0), height.max(0), true);
        let props = handle.0.props.borrow().clone();
        let children = handle.0.children.borrow().clone();
        match props {
            Some(Props::Stack {
                axis,
                spacing,
                padding,
                align,
                justify,
            }) => self.layout_stack(
                &children, width, height, axis, spacing, padding, align, justify,
            ),
            Some(Props::Scroll { .. }) => {
                if let Some(child) = children.first() {
                    self.layout_handle(child, 0, 0, width, height);
                }
            }
            Some(Props::Split { role, .. }) => {
                self.layout_split(&children, width, height, role);
            }
            Some(Props::Workspace { .. }) => self.layout_workspace(&children, width, height),
            Some(Props::List {
                style: ListStyle::Table,
                ..
            }) => self.size_table_columns(handle, width),
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_stack(
        &self,
        children: &[WindowsHandle],
        width: i32,
        height: i32,
        axis: Axis,
        spacing: Spacing,
        padding: Option<Spacing>,
        align: Align,
        justify: Justify,
    ) {
        let gap = self.metric(spacing);
        let inset = padding.map_or(0, |value| self.metric(value));
        let available_width = (width - inset * 2).max(0);
        let available_height = (height - inset * 2).max(0);
        let main_extent = if axis == Axis::Horizontal {
            available_width
        } else {
            available_height
        };
        let gap_total = gap * i32::try_from(children.len().saturating_sub(1)).unwrap_or(i32::MAX);
        let fixed = children
            .iter()
            .filter(|child| !is_flexible(child, axis))
            .map(|child| self.desired(child, axis))
            .sum::<i32>();
        let flexible_count = children
            .iter()
            .filter(|child| is_flexible(child, axis))
            .count();
        let remaining = (main_extent - fixed - gap_total).max(0);
        let flexible_extent = if flexible_count == 0 {
            0
        } else {
            remaining / i32::try_from(flexible_count).unwrap_or(1)
        };
        let occupied =
            fixed + gap_total + flexible_extent * i32::try_from(flexible_count).unwrap_or_default();
        let mut cursor = inset
            + match justify {
                Justify::Start => 0,
                Justify::Center => (main_extent - occupied).max(0) / 2,
                Justify::End => (main_extent - occupied).max(0),
            };
        for child in children {
            let primary = if is_flexible(child, axis) {
                flexible_extent
            } else {
                self.desired(child, axis).min(main_extent)
            };
            let cross_available = if axis == Axis::Horizontal {
                available_height
            } else {
                available_width
            };
            let desired_cross = self.desired(
                child,
                if axis == Axis::Horizontal {
                    Axis::Vertical
                } else {
                    Axis::Horizontal
                },
            );
            let cross = if align == Align::Stretch || structural(child) {
                cross_available
            } else {
                desired_cross.min(cross_available)
            };
            let cross_origin = inset
                + match align {
                    Align::Start | Align::Stretch => 0,
                    Align::Center => (cross_available - cross).max(0) / 2,
                    Align::End => (cross_available - cross).max(0),
                };
            if axis == Axis::Horizontal {
                self.layout_handle(child, cursor, cross_origin, primary, cross);
            } else {
                self.layout_handle(child, cross_origin, cursor, cross, primary);
            }
            cursor += primary + gap;
        }
    }

    fn layout_split(&self, children: &[WindowsHandle], width: i32, height: i32, role: SplitRole) {
        let secondary = match role {
            SplitRole::Navigation => self.scale(240),
            SplitRole::Utility => self.scale(288),
        }
        .min((width / 2).max(0));
        if let Some(first) = children.first() {
            self.layout_handle(first, 0, 0, secondary, height);
        }
        if let Some(second) = children.get(1) {
            self.layout_handle(second, secondary + 1, 0, width - secondary - 1, height);
        }
    }

    fn layout_workspace(&self, children: &[WindowsHandle], width: i32, height: i32) {
        let mut sidebar = if self.sidebar_visible.get() {
            self.scale(236)
        } else {
            0
        };
        let mut inspector = if self.inspector_visible.get() {
            self.scale(284)
        } else {
            0
        };
        if width < self.scale(900) {
            sidebar = sidebar.min(self.scale(190));
            inspector = inspector.min(self.scale(220));
        }
        let minimum_content = self.scale(320);
        let side_total = (width - minimum_content).max(0);
        if sidebar + inspector > side_total && sidebar + inspector > 0 {
            let original = sidebar + inspector;
            sidebar = sidebar * side_total / original;
            inspector = side_total - sidebar;
        }
        let content_width = (width - sidebar - inspector).max(0);
        if let Some(value) = children.first() {
            show(value.0.hwnd, sidebar > 0);
            if sidebar > 0 {
                self.layout_handle(value, 0, 0, sidebar, height);
            }
        }
        if let Some(value) = children.get(1) {
            self.layout_handle(value, sidebar, 0, content_width, height);
        }
        if let Some(value) = children.get(2) {
            show(value.0.hwnd, inspector > 0);
            if inspector > 0 {
                self.layout_handle(value, sidebar + content_width, 0, inspector, height);
            }
        }
    }

    fn desired(&self, handle: &WindowsHandle, axis: Axis) -> i32 {
        match handle.0.props.borrow().as_ref() {
            Some(Props::Label { role, text, .. }) => match axis {
                Axis::Horizontal => {
                    self.scale((text.chars().count() as i32 * 7 + 12).clamp(40, 440))
                }
                Axis::Vertical => self.scale(match role {
                    TextRole::Title => 32,
                    TextRole::Heading => 26,
                    _ => 22,
                }),
            },
            Some(Props::Button { label, size, .. }) => match axis {
                Axis::Horizontal => {
                    self.scale((label.chars().count() as i32 * 8 + 28).clamp(72, 220))
                }
                Axis::Vertical => self.control_height(*size),
            },
            Some(Props::Input { .. }) => match axis {
                Axis::Horizontal => self.scale(180),
                Axis::Vertical => self.scale(30),
            },
            Some(Props::Toggle { label, size, .. }) => match axis {
                Axis::Horizontal => {
                    self.scale((label.chars().count() as i32 * 8 + 28).clamp(100, 260))
                }
                Axis::Vertical => self.control_height(*size),
            },
            Some(Props::Progress { .. }) => match axis {
                Axis::Horizontal => self.scale(220),
                Axis::Vertical => self.scale(18),
            },
            Some(Props::Separator { .. }) => self.scale(1),
            Some(Props::Spacer {
                horizontal,
                vertical,
            }) => match axis {
                Axis::Horizontal if *horizontal => 0,
                Axis::Vertical if *vertical => 0,
                _ => 1,
            },
            Some(Props::Stack {
                axis: stack_axis,
                spacing,
                padding,
                ..
            }) if *stack_axis == axis => {
                let child_sum = handle
                    .0
                    .children
                    .borrow()
                    .iter()
                    .map(|child| self.desired(child, axis))
                    .sum::<i32>();
                let count = handle.0.children.borrow().len();
                child_sum
                    + self.metric(*spacing)
                        * i32::try_from(count.saturating_sub(1)).unwrap_or_default()
                    + padding.map_or(0, |value| self.metric(value) * 2)
            }
            Some(Props::Stack { padding, .. }) => {
                handle
                    .0
                    .children
                    .borrow()
                    .iter()
                    .map(|child| self.desired(child, axis))
                    .max()
                    .unwrap_or(0)
                    + padding.map_or(0, |value| self.metric(value) * 2)
            }
            Some(Props::Status { title, message, .. }) => match axis {
                Axis::Horizontal => self.scale(
                    (title.chars().count().max(message.chars().count()) as i32 * 7 + 40)
                        .clamp(180, 560),
                ),
                Axis::Vertical => self.scale(86),
            },
            _ => match axis {
                Axis::Horizontal => self.scale(160),
                Axis::Vertical => self.scale(32),
            },
        }
    }

    fn metric(&self, spacing: Spacing) -> i32 {
        self.scale(match spacing {
            Spacing::Joined => 0,
            Spacing::Compact => 4,
            Spacing::Related => 8,
            Spacing::Section => 16,
            Spacing::Content => 20,
        })
    }

    fn control_height(&self, size: ControlSize) -> i32 {
        self.scale(match size {
            ControlSize::Mini => 22,
            ControlSize::Small => 26,
            ControlSize::Regular => 30,
            ControlSize::Large => 36,
            ControlSize::ExtraLarge => 42,
        })
    }

    fn scale(&self, value: i32) -> i32 {
        value.saturating_mul(self.dpi.get() as i32) / 96
    }

    fn size_table_columns(&self, handle: &WindowsHandle, width: i32) {
        let Some(Props::List { columns, .. }) = handle.0.props.borrow().clone() else {
            return;
        };
        if columns.is_empty() {
            return;
        }
        let remaining = width.max(self.scale(400));
        let first = (remaining * 40 / 100).max(self.scale(180));
        // SAFETY: the messages target a live SysListView32 HWND and use integer widths.
        unsafe {
            for index in 0..columns.len() {
                let column_width = if index == 0 {
                    first
                } else {
                    ((remaining - first) / i32::try_from(columns.len() - 1).unwrap_or(1))
                        .max(self.scale(100))
                };
                let _ = send_message(
                    handle.0.hwnd,
                    LVM_SETCOLUMNWIDTH,
                    index,
                    column_width as isize,
                );
            }
        }
    }

    fn rebuild_list(&self, list: &WindowsHandle) {
        let props = list.0.props.borrow().clone();
        let Some(Props::List { style, columns, .. }) = props else {
            return;
        };
        let rows = list.0.children.borrow().clone();
        list.0.list_rebuilding.set(true);
        // SAFETY: all message structures remain alive for the synchronous SendMessage calls.
        unsafe {
            if style == ListStyle::Source {
                let _ = send_message(list.0.hwnd, TVM_DELETEITEM, 0, TVI_ROOT);
                for row in &rows {
                    self.insert_tree_row(list.0.hwnd, row, TVI_ROOT);
                }
            } else {
                let _ = send_message(list.0.hwnd, LVM_DELETEALLITEMS, 0, 0);
                if style == ListStyle::Table {
                    for index in 0..32 {
                        if send_message(list.0.hwnd, LVM_DELETECOLUMN, 0, 0) == 0 {
                            break;
                        }
                        if index == 31 {
                            break;
                        }
                    }
                    for (index, column) in columns.iter().enumerate() {
                        let mut title = wide(&column.title);
                        let mut native = LvColumnW {
                            mask: LVCF_FMT | LVCF_WIDTH | LVCF_TEXT | LVCF_SUBITEM,
                            fmt: 0,
                            cx: self.scale(if index == 0 { 260 } else { 140 }),
                            psz_text: title.as_mut_ptr(),
                            cch_text_max: i32::try_from(title.len()).unwrap_or(i32::MAX),
                            i_sub_item: i32::try_from(index).unwrap_or(i32::MAX),
                            i_image: 0,
                            i_order: 0,
                            cx_min: 0,
                            cx_default: 0,
                            cx_ideal: 0,
                        };
                        let _ = send_message(
                            list.0.hwnd,
                            LVM_INSERTCOLUMNW,
                            index,
                            (&raw mut native) as isize,
                        );
                    }
                }
                let mut next = 0;
                for row in &rows {
                    self.insert_list_row(list.0.hwnd, row, style, &mut next, 0);
                }
            }
        }
        list.0.list_rebuilding.set(false);
    }

    unsafe fn insert_tree_row(&self, owner: HWND, row: &WindowsHandle, parent: isize) {
        let Some(Props::ListRow {
            title,
            role,
            expanded,
            selected,
            ..
        }) = row.0.props.borrow().clone()
        else {
            return;
        };
        let mut text = wide(&title);
        let mut insertion = TvInsertStructW {
            parent,
            insert_after: TVI_LAST,
            item: TvItemExW {
                mask: TVIF_TEXT | TVIF_PARAM | TVIF_STATE | TVIF_CHILDREN,
                item: 0,
                state: if role == ListRowRole::Section {
                    TVIS_BOLD
                } else {
                    0
                } | if selected { TVIS_SELECTED } else { 0 },
                state_mask: TVIS_BOLD | TVIS_SELECTED,
                psz_text: text.as_mut_ptr(),
                cch_text_max: i32::try_from(text.len()).unwrap_or(i32::MAX),
                i_image: 0,
                i_selected_image: 0,
                c_children: i32::from(!row.0.children.borrow().is_empty()),
                l_param: Rc::as_ptr(&row.0) as LPARAM,
                i_integral: 0,
                state_ex: 0,
                hwnd: null_mut(),
                i_expanded_image: 0,
                i_reserved: 0,
            },
        };
        // SAFETY: insertion points at a fully initialized structure for a live tree view.
        let native_item =
            unsafe { send_message(owner, TVM_INSERTITEMW, 0, (&raw mut insertion) as isize) };
        if let Some(state) = &row.0.row {
            state.owner.set(owner);
            state.tree_item.set(native_item);
        }
        for child in row.0.children.borrow().iter() {
            // SAFETY: child insertion occurs synchronously under the native parent item.
            unsafe { self.insert_tree_row(owner, child, native_item) };
        }
        if expanded {
            // SAFETY: the item was returned by this tree control.
            unsafe {
                let _ = send_message(owner, TVM_EXPAND, TVE_EXPAND, native_item);
            }
        }
    }

    unsafe fn insert_list_row(
        &self,
        owner: HWND,
        row: &WindowsHandle,
        style: ListStyle,
        next: &mut i32,
        depth: i32,
    ) {
        let Some(Props::ListRow {
            title,
            cells,
            expanded,
            selected,
            ..
        }) = row.0.props.borrow().clone()
        else {
            return;
        };
        let index = *next;
        *next += 1;
        let has_children = !row.0.children.borrow().is_empty();
        let display_title = if has_children {
            format!("{} {title}", if expanded { "▾" } else { "▸" })
        } else {
            title
        };
        let mut title = wide(&display_title);
        let mut native = LvItemW {
            mask: LVIF_TEXT | LVIF_PARAM | LVIF_STATE | LVIF_INDENT,
            i_item: index,
            i_sub_item: 0,
            state: if selected { LVIS_SELECTED } else { 0 },
            state_mask: LVIS_SELECTED,
            psz_text: title.as_mut_ptr(),
            cch_text_max: i32::try_from(title.len()).unwrap_or(i32::MAX),
            i_image: 0,
            l_param: Rc::as_ptr(&row.0) as LPARAM,
            i_indent: depth,
            i_group_id: 0,
            c_columns: 0,
            pu_columns: null_mut(),
            pi_col_fmt: null_mut(),
            i_group: 0,
        };
        // SAFETY: item points at initialized storage for a live list view.
        unsafe {
            let _ = send_message(owner, LVM_INSERTITEMW, 0, (&raw mut native) as isize);
        }
        if style == ListStyle::Table {
            for (offset, cell) in cells.iter().enumerate() {
                let mut value = wide(cell);
                native.i_sub_item = i32::try_from(offset + 1).unwrap_or(i32::MAX);
                native.psz_text = value.as_mut_ptr();
                // SAFETY: each subitem update is synchronous and `value` remains alive.
                unsafe {
                    let _ = send_message(
                        owner,
                        LVM_SETITEMTEXTW,
                        index as usize,
                        (&raw mut native) as isize,
                    );
                }
            }
        }
        if let Some(state) = &row.0.row {
            state.owner.set(owner);
            state.list_index.set(index);
        }
        if expanded {
            for child in row.0.children.borrow().iter() {
                // SAFETY: list rows are inserted synchronously into the same owner control.
                unsafe { self.insert_list_row(owner, child, style, next, depth + 1) };
            }
        }
    }
}

impl NativeBackend for WindowsBackend {
    type Handle = WindowsHandle;
    type Error = WindowsDiagnostic;

    fn root(&self) -> Self::Handle {
        self.root.clone()
    }

    fn validate(&self, element: &Element) -> Result<(), Self::Error> {
        validate_element(element)
    }

    fn create(
        &mut self,
        element: &Element,
        events: EventBindings,
    ) -> Result<Self::Handle, Self::Error> {
        let (class, text, style, extended) = native_description(element);
        let hwnd = create_window(class, &text, style, extended, self.root.0.hwnd, null_mut())?;
        set_native_font(hwnd, self.font.0);
        apply_native_theme(hwnd, self.dark);
        let handle = WindowsHandle::new(
            hwnd,
            HostKind::Element(element.kind()),
            Some(element.props().clone()),
            events,
            false,
            self.dark,
        );
        apply_semantic_font(&handle, self.dpi.get(), self.font.0)?;
        apply_initial_properties(&handle);
        if let Some(name) = accessibility_name(element) {
            set_accessible_name(hwnd, name)?;
        }
        if let Props::List { style, .. } = element.props()
            && *style != ListStyle::Source
        {
            // SAFETY: the message configures a live SysListView32 instance.
            unsafe {
                let _ = send_message(
                    hwnd,
                    LVM_SETEXTENDEDLISTVIEWSTYLE,
                    0,
                    (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
                );
            }
        }
        Ok(handle)
    }

    fn apply(&mut self, handle: &Self::Handle, patch: &PropertyPatch) -> Result<(), Self::Error> {
        let props = props_from_patch(patch);
        let accessible_name = accessibility_name_for_props(&props).map(str::to_owned);
        *handle.0.props.borrow_mut() = Some(props);
        apply_semantic_font(handle, self.dpi.get(), self.font.0)?;
        apply_patch_to_native(handle, patch);
        if let Some(name) = accessible_name {
            set_accessible_name(handle.0.hwnd, &name)?;
        }
        if handle.0.kind == HostKind::Element(ElementKind::List) {
            self.rebuild_list(handle);
        }
        if handle.0.kind == HostKind::Element(ElementKind::ListRow) {
            let owner = handle
                .0
                .row
                .as_ref()
                .map_or(null_mut(), |state| state.owner.get());
            if !owner.is_null()
                && let Some(list) = find_ancestor_list(&self.root, owner)
            {
                self.rebuild_list(&list);
            }
        }
        self.layout_root();
        Ok(())
    }

    fn insert_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        // SAFETY: both HWND values are live children on this UI thread.
        unsafe {
            let _ = SetParent(child.0.hwnd, parent.0.hwnd);
        }
        let mut children = parent.0.children.borrow_mut();
        let insertion = index.min(children.len());
        children.insert(insertion, child.clone());
        drop(children);
        if parent.0.kind == HostKind::Element(ElementKind::List) {
            self.rebuild_list(parent);
        }
        self.layout_root();
        Ok(())
    }

    fn remove_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        let mut children = parent.0.children.borrow_mut();
        if children
            .get(index)
            .is_some_and(|value| Rc::ptr_eq(&value.0, &child.0))
        {
            children.remove(index);
        } else if let Some(position) = children
            .iter()
            .position(|value| Rc::ptr_eq(&value.0, &child.0))
        {
            children.remove(position);
        }
        drop(children);
        if parent.0.kind == HostKind::Element(ElementKind::List) {
            self.rebuild_list(parent);
        }
        self.layout_root();
        Ok(())
    }

    fn move_child(
        &mut self,
        parent: &Self::Handle,
        _child: &Self::Handle,
        from: usize,
        to: usize,
    ) -> Result<(), Self::Error> {
        let mut children = parent.0.children.borrow_mut();
        if from < children.len() {
            let child = children.remove(from);
            let destination = to.min(children.len());
            children.insert(destination, child);
        }
        drop(children);
        if parent.0.kind == HostKind::Element(ElementKind::List) {
            self.rebuild_list(parent);
        }
        self.layout_root();
        Ok(())
    }
}

enum Command {
    Activate(EventBindings),
    Input {
        hwnd: HWND,
        events: EventBindings,
    },
    Select {
        value: String,
        events: EventBindings,
    },
    Menu {
        hwnd: HWND,
        entries: Vec<MenuCommand>,
    },
    ToggleSidebar {
        hwnd: HWND,
    },
    ToggleInspector {
        hwnd: HWND,
    },
}

enum MenuCommand {
    Action {
        label: String,
        enabled: bool,
        events: EventBindings,
    },
    Separator,
}

struct ToolbarControl {
    hwnd: HWND,
    width: i32,
    right_aligned: bool,
    essential: bool,
    symbol_only: bool,
}

struct HostWindow {
    hwnd: HWND,
    root: HWND,
    runtime: WindowRuntime<WindowsBackend>,
    commands: HashMap<usize, Command>,
    toolbar: Vec<ToolbarControl>,
    tooltip: HWND,
    tooltip_texts: Vec<Box<[u16]>>,
    next_id: usize,
    dpi: u32,
    minimum_width: i32,
    minimum_height: i32,
    window_style: u32,
    window_extended_style: u32,
    dark: bool,
    font: Rc<NativeFont>,
    symbol_font: Rc<NativeFont>,
    panel_behavior: Option<PanelBehavior>,
    background_brush: HBRUSH,
}

impl Drop for HostWindow {
    fn drop(&mut self) {
        if !self.tooltip.is_null() {
            // SAFETY: the tooltip is owned only by this host window.
            unsafe {
                let _ = DestroyWindow(self.tooltip);
            }
        }
        if !self.background_brush.is_null() {
            // SAFETY: the brush is owned only by this host and no paint can run after teardown.
            unsafe {
                let _ = DeleteObject(self.background_brush as HGDIOBJ);
            }
        }
    }
}

impl HostWindow {
    fn toolbar_height(&self) -> i32 {
        if self.toolbar.is_empty() {
            0
        } else {
            scale(TOOLBAR_HEIGHT, self.dpi)
        }
    }

    fn command(&mut self, id: usize, notification: u16) {
        let Some(command) = self.commands.get(&id) else {
            return;
        };
        match command {
            Command::Activate(events) if notification == BN_CLICKED => events.emit_activate(),
            Command::Input { hwnd, events } if notification == EN_CHANGE => {
                events.emit_input(window_text(*hwnd));
            }
            Command::Select { value, events } if notification == BN_CLICKED => {
                events.emit_input(value.clone());
            }
            Command::Menu { hwnd, entries } if notification == BN_CLICKED => {
                show_command_menu(self.hwnd, *hwnd, entries);
            }
            Command::ToggleSidebar { hwnd } if notification == BN_CLICKED => {
                let checked = button_checked(*hwnd);
                self.runtime.with_renderer_mut(|renderer| {
                    renderer.backend_mut().sidebar_visible.set(checked);
                    renderer.backend().layout_root();
                });
            }
            Command::ToggleInspector { hwnd } if notification == BN_CLICKED => {
                let checked = button_checked(*hwnd);
                self.runtime.with_renderer_mut(|renderer| {
                    renderer.backend_mut().inspector_visible.set(checked);
                    renderer.backend().layout_root();
                });
            }
            _ => {}
        }
    }

    fn relayout(&mut self, width: i32, height: i32) {
        let toolbar_height = self.toolbar_height();
        move_window(
            self.root,
            0,
            toolbar_height,
            width,
            (height - toolbar_height).max(0),
            true,
        );
        let padding = scale(10, self.dpi);
        let gap = scale(6, self.dpi);
        let control_height = scale(32, self.dpi);
        let mut left = padding;
        let mut right = width - padding;
        for essential in [true, false] {
            for control in self
                .toolbar
                .iter_mut()
                .filter(|value| value.right_aligned && value.essential == essential)
            {
                let control_width = scale(control.width, self.dpi);
                right -= control_width;
                let visible = right > width / 2;
                show(control.hwnd, visible);
                if visible {
                    move_window(
                        control.hwnd,
                        right,
                        scale(10, self.dpi),
                        control_width,
                        control_height,
                        true,
                    );
                }
                right -= gap;
            }
        }
        for control in self.toolbar.iter_mut().filter(|value| !value.right_aligned) {
            let control_width = scale(control.width, self.dpi);
            let visible = left + control_width < right;
            show(control.hwnd, visible);
            if visible {
                move_window(
                    control.hwnd,
                    left,
                    scale(10, self.dpi),
                    control_width,
                    control_height,
                    true,
                );
                left += control_width + gap;
            }
        }
        self.runtime.with_renderer_mut(|renderer| {
            renderer.backend().layout_root();
        });
    }

    fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi.max(96);
        let Ok(font) = system_message_font(self.dpi) else {
            self.runtime
                .with_renderer_mut(|renderer| renderer.backend_mut().dpi.set(self.dpi));
            return;
        };
        let Ok(symbol_font) = system_symbol_font(self.dpi) else {
            self.runtime
                .with_renderer_mut(|renderer| renderer.backend_mut().dpi.set(self.dpi));
            return;
        };
        for control in &self.toolbar {
            set_native_font(
                control.hwnd,
                if control.symbol_only {
                    symbol_font.0
                } else {
                    font.0
                },
            );
        }
        self.runtime.with_renderer_mut(|renderer| {
            renderer.backend_mut().set_dpi(self.dpi, font.clone());
        });
        self.font = font;
        self.symbol_font = symbol_font;
    }
}

/// Runs the native Windows application until its last top-level window closes.
pub fn run(application: ApplicationSpec) -> Result<(), WindowsDiagnostic> {
    let _apartment = initialize_native_process()?;
    let instance = module_instance()?;
    register_window_class(instance)?;
    let mut primary_windows = Vec::new();
    let mut panels = Vec::new();
    for window in application.windows {
        if matches!(window.kind, WindowKind::Panel(_)) {
            panels.push(window);
        } else {
            primary_windows.push(window);
        }
    }
    let mut main_window: HWND = null_mut();
    let mut created_windows = Vec::new();
    for window in primary_windows.into_iter().chain(panels) {
        let owner = match window.kind {
            WindowKind::Panel(PanelBehavior { floating: true, .. }) => main_window,
            _ => null_mut(),
        };
        let is_first_main = main_window.is_null() && matches!(window.kind, WindowKind::Main);
        let hwnd = match create_host_window(instance, owner, window) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                for created in created_windows.into_iter().rev() {
                    // SAFETY: these HWND values were created on this thread and are still live.
                    unsafe {
                        let _ = DestroyWindow(created);
                    }
                }
                return Err(error);
            }
        };
        created_windows.push(hwnd);
        if is_first_main {
            main_window = hwnd;
        }
    }
    if created_windows.is_empty() {
        return Err(WindowsDiagnostic::InvalidNativeState {
            reason: "Windows application requires at least one top-level window".to_owned(),
        });
    }
    MESSAGE_LOOP_ACTIVE.with(|active| active.set(true));
    // SAFETY: the message loop owns all created HWND values on the current thread.
    unsafe {
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, null_mut(), 0, 0);
            if result == -1 {
                MESSAGE_LOOP_ACTIVE.with(|active| active.set(false));
                return Err(last_error("GetMessageW"));
            }
            if result == 0 {
                break;
            }
            let active_window = GetActiveWindow();
            if !active_window.is_null() && IsDialogMessageW(active_window, &message) != 0 {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    MESSAGE_LOOP_ACTIVE.with(|active| active.set(false));
    Ok(())
}

fn initialize_native_process() -> Result<ComApartment, WindowsDiagnostic> {
    // SAFETY: this executes before any HWND is created on the process UI thread.
    unsafe {
        if let Err(error) =
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            && !AreDpiAwarenessContextsEqual(
                GetThreadDpiAwarenessContext(),
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            )
            .as_bool()
        {
            return Err(WindowsDiagnostic::InvalidNativeState {
                reason: format!("PerMonitorV2 DPI initialization failed: {error}"),
            });
        }
        let controls = INITCOMMONCONTROLSEX {
            dwSize: u32::try_from(size_of::<INITCOMMONCONTROLSEX>()).unwrap_or(u32::MAX),
            dwICC: ICC_BAR_CLASSES
                | ICC_LISTVIEW_CLASSES
                | ICC_PROGRESS_CLASS
                | ICC_TREEVIEW_CLASSES,
        };
        if InitCommonControlsEx(&controls) == 0 {
            return Err(last_error("InitCommonControlsEx"));
        }
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|error| WindowsDiagnostic::InvalidNativeState {
                reason: format!("COM apartment initialization failed: {error}"),
            })?;
    }
    Ok(ComApartment)
}

fn register_window_class(instance: HINSTANCE) -> Result<(), WindowsDiagnostic> {
    let class_name = wide(CLASS_NAME);
    // SAFETY: the class structure references local UTF-16 storage only for the synchronous call.
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(u32::MAX),
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: LoadIconW(null_mut(), IDI_APPLICATION),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (6usize) as HBRUSH,
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: LoadIconW(null_mut(), IDI_APPLICATION),
        };
        if RegisterClassExW(&class) == 0 {
            let code = GetLastError();
            if code != 1410 {
                return Err(WindowsDiagnostic::NativeOperation {
                    operation: "RegisterClassExW",
                    code,
                });
            }
        }
    }
    Ok(())
}

fn create_host_window(
    instance: HINSTANCE,
    owner: HWND,
    spec: WindowSpec,
) -> Result<HWND, WindowsDiagnostic> {
    let dark = dark_appearance();
    let class_name = wide(CLASS_NAME);
    let title = wide(&spec.title);
    let panel_behavior = match spec.kind {
        WindowKind::Panel(behavior) => Some(behavior),
        WindowKind::Main | WindowKind::Preferences => None,
    };
    let (extended, style) = match panel_behavior {
        None => (0, WS_OVERLAPPEDWINDOW),
        Some(behavior) => (
            WS_EX_TOOLWINDOW
                | if behavior.accepts_keyboard {
                    0
                } else {
                    WS_EX_NOACTIVATE
                },
            WS_CAPTION | WS_SYSMENU | WS_THICKFRAME,
        ),
    };
    let workspace_panes = match spec.content.snapshot().props() {
        Props::Workspace {
            sidebar_collapsible,
            inspector_collapsible,
        } => Some((*sidebar_collapsible, *inspector_collapsible)),
        _ => None,
    };
    let has_toolbar = !spec.toolbar.is_empty()
        || workspace_panes.is_some_and(|(sidebar, inspector)| sidebar || inspector);
    let creation_dpi = unsafe { GetDpiForSystem() }.max(96);
    let window_style = style | WS_CLIPCHILDREN;
    let initial_width = scale(spec.initial_size.width.round() as i32, creation_dpi);
    let initial_height = scale(spec.initial_size.height.round() as i32, creation_dpi)
        + if has_toolbar {
            scale(TOOLBAR_HEIGHT, creation_dpi)
        } else {
            0
        };
    let (initial_outer_width, initial_outer_height) = outer_size_for_content(
        initial_width,
        initial_height,
        window_style,
        extended,
        creation_dpi,
    )?;
    // SAFETY: the registered class and UTF-16 strings remain valid through the call.
    let hwnd = unsafe {
        CreateWindowExW(
            extended,
            class_name.as_ptr(),
            title.as_ptr(),
            window_style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            initial_outer_width,
            initial_outer_height,
            owner,
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        return Err(last_error("CreateWindowExW(top-level)"));
    }
    let mut pending_window = PendingTopLevel(hwnd);
    set_dark_title_bar(hwnd, dark);
    let dpi = dpi_for_window(hwnd);
    let font = system_message_font(dpi)?;
    let symbol_font = system_symbol_font(dpi)?;
    let root = create_window(
        STATIC_CLASS,
        "Rinka content root",
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        WS_EX_CONTROLPARENT,
        hwnd,
        null_mut(),
    )?;
    set_native_font(root, font.0);
    apply_native_theme(root, dark);
    let backend = WindowsBackend::new(root, dpi, font.clone(), dark);
    let runtime = WindowRuntime::mount(Renderer::new(backend), spec.content).map_err(|error| {
        WindowsDiagnostic::InvalidNativeState {
            reason: format!("initial Windows render failed: {error}"),
        }
    })?;
    let mut host = Box::new(HostWindow {
        hwnd,
        root,
        runtime,
        commands: HashMap::new(),
        toolbar: Vec::new(),
        tooltip: null_mut(),
        tooltip_texts: Vec::new(),
        next_id: CONTROL_ID_FIRST,
        dpi,
        minimum_width: spec.minimum_size.width.round() as i32,
        minimum_height: spec.minimum_size.height.round() as i32,
        window_style,
        window_extended_style: extended,
        dark,
        font,
        symbol_font,
        panel_behavior,
        // SAFETY: this brush remains owned by HostWindow until WM_NCDESTROY.
        background_brush: if dark {
            unsafe { CreateSolidBrush(DARK_BACKGROUND) }
        } else {
            null_mut()
        },
    });
    build_toolbar(
        &mut host,
        &spec.toolbar,
        spec.toolbar_display,
        workspace_panes,
    )?;
    // SAFETY: `host` remains allocated until WM_NCDESTROY reclaims the pointer.
    unsafe {
        let host_pointer = Box::into_raw(host);
        let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, host_pointer as isize);
        TOP_LEVEL_WINDOWS.with(|windows| windows.borrow_mut().push(hwnd));
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        (*host_pointer).relayout(rect.right - rect.left, rect.bottom - rect.top);
        ShowWindow(
            hwnd,
            if panel_behavior.is_some_and(|behavior| !behavior.accepts_keyboard) {
                SW_SHOWNOACTIVATE
            } else {
                SW_SHOWNORMAL
            },
        );
        let _ = UpdateWindow(hwnd);
    }
    pending_window.0 = null_mut();
    Ok(hwnd)
}

fn build_toolbar(
    host: &mut HostWindow,
    items: &[ToolbarItem],
    display: ToolbarDisplay,
    workspace_panes: Option<(bool, bool)>,
) -> Result<(), WindowsDiagnostic> {
    for item in items {
        let right_aligned = matches!(item.placement, rinka_core::ToolbarPlacement::Trailing);
        match &item.kind {
            ToolbarItemKind::Action {
                symbol,
                on_activate,
            } => {
                let events = EventBindings::activate(on_activate.clone());
                let presentation = toolbar_presentation(display, *symbol, &item.label);
                add_toolbar_button(
                    host,
                    &presentation,
                    &item.label,
                    &item.help,
                    item.enabled,
                    right_aligned,
                    events,
                )?;
            }
            ToolbarItemKind::ActionGroup { actions } => {
                for action in actions {
                    add_action_button(host, action, item.enabled, display, right_aligned)?;
                }
            }
            ToolbarItemKind::SelectionGroup {
                choices,
                selected_id,
                on_select,
            } => {
                for (choice_index, choice) in choices.iter().enumerate() {
                    let id = host.next_id;
                    host.next_id += 1;
                    let presentation = toolbar_presentation(display, choice.symbol, &choice.label);
                    let hwnd = toolbar_control(
                        host.hwnd,
                        BUTTON_CLASS,
                        &presentation.text,
                        WS_CHILD
                            | WS_VISIBLE
                            | WS_TABSTOP
                            | BS_AUTORADIOBUTTON
                            | BS_PUSHLIKE
                            | BS_FLAT
                            | if choice_index == 0 { WS_GROUP } else { 0 },
                        id,
                        if presentation.symbol_only {
                            host.symbol_font.0
                        } else {
                            host.font.0
                        },
                        host.dark,
                    )?;
                    set_accessible_name(hwnd, &choice.label)?;
                    add_toolbar_tooltip(host, hwnd, &choice.label)?;
                    set_enabled(hwnd, item.enabled && choice.enabled);
                    if choice.id == *selected_id {
                        // SAFETY: the button is live and accepts BM_SETCHECK.
                        unsafe {
                            let _ = send_message(hwnd, BM_SETCHECK, BST_CHECKED, 0);
                        }
                    }
                    host.commands.insert(
                        id,
                        Command::Select {
                            value: choice.id.clone(),
                            events: EventBindings::input(on_select.clone()),
                        },
                    );
                    host.toolbar.push(ToolbarControl {
                        hwnd,
                        width: presentation.width,
                        right_aligned,
                        essential: false,
                        symbol_only: presentation.symbol_only,
                    });
                }
            }
            ToolbarItemKind::Menu {
                symbol, entries, ..
            } => {
                let id = host.next_id;
                host.next_id += 1;
                let mut presentation = toolbar_presentation(display, *symbol, &item.label);
                if !presentation.symbol_only {
                    presentation.text.push_str(" ▾");
                    presentation.width = (presentation.width + 14).min(180);
                }
                let hwnd = toolbar_control(
                    host.hwnd,
                    BUTTON_CLASS,
                    &presentation.text,
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
                    id,
                    if presentation.symbol_only {
                        host.symbol_font.0
                    } else {
                        host.font.0
                    },
                    host.dark,
                )?;
                set_accessible_name(hwnd, &item.label)?;
                add_toolbar_tooltip(host, hwnd, &item.help)?;
                set_enabled(hwnd, item.enabled);
                let mut commands = Vec::new();
                for entry in entries {
                    match entry {
                        ToolbarMenuEntry::Action(action) => commands.push(MenuCommand::Action {
                            label: action.label.clone(),
                            enabled: action.enabled,
                            events: EventBindings::activate(action.on_activate.clone()),
                        }),
                        ToolbarMenuEntry::Separator => commands.push(MenuCommand::Separator),
                    }
                }
                host.commands.insert(
                    id,
                    Command::Menu {
                        hwnd,
                        entries: commands,
                    },
                );
                host.toolbar.push(ToolbarControl {
                    hwnd,
                    width: presentation.width,
                    right_aligned,
                    essential: false,
                    symbol_only: presentation.symbol_only,
                });
            }
            ToolbarItemKind::Search {
                value,
                placeholder,
                accessibility_label,
                on_input,
                ..
            } => {
                let id = host.next_id;
                host.next_id += 1;
                let hwnd = toolbar_control(
                    host.hwnd,
                    EDIT_CLASS,
                    value,
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_SEARCH,
                    id,
                    host.font.0,
                    host.dark,
                )?;
                set_cue_banner(hwnd, placeholder);
                set_accessible_name(hwnd, accessibility_label)?;
                add_toolbar_tooltip(host, hwnd, &item.help)?;
                set_enabled(hwnd, item.enabled);
                host.commands.insert(
                    id,
                    Command::Input {
                        hwnd,
                        events: EventBindings::input(on_input.clone()),
                    },
                );
                host.toolbar.push(ToolbarControl {
                    hwnd,
                    width: 190,
                    right_aligned: true,
                    essential: true,
                    symbol_only: false,
                });
            }
        }
    }
    if let Some((sidebar_collapsible, inspector_collapsible)) = workspace_panes {
        if sidebar_collapsible {
            add_pane_toggle(host, "Navigation pane", true, true)?;
        }
        if inspector_collapsible {
            add_pane_toggle(host, "Details pane", false, true)?;
        }
    }
    Ok(())
}

struct ToolbarPresentation {
    text: String,
    width: i32,
    symbol_only: bool,
}

fn toolbar_presentation(
    display: ToolbarDisplay,
    symbol: Symbol,
    label: &str,
) -> ToolbarPresentation {
    let symbol_only = display == ToolbarDisplay::IconOnly;
    ToolbarPresentation {
        text: if symbol_only {
            symbol_glyph(symbol).to_string()
        } else {
            label.to_owned()
        },
        width: if symbol_only {
            40
        } else {
            (label.chars().count() as i32 * 8 + 30).clamp(64, 160)
        },
        symbol_only,
    }
}

fn symbol_glyph(symbol: Symbol) -> char {
    match symbol {
        Symbol::Back => '\u{e72b}',
        Symbol::Forward => '\u{e72a}',
        Symbol::Add => '\u{e710}',
        Symbol::Refresh => '\u{e72c}',
        Symbol::Search => '\u{e721}',
        Symbol::Home => '\u{e80f}',
        Symbol::Folder => '\u{e8b7}',
        Symbol::File => '\u{e8a5}',
        Symbol::Code => '\u{e8a5}',
        Symbol::Image => '\u{e8b9}',
        Symbol::Terminal => '\u{e756}',
        Symbol::Settings => '\u{e713}',
        Symbol::More => '\u{e712}',
        Symbol::Grid => '\u{e8a9}',
        Symbol::List => '\u{e8fd}',
        Symbol::Columns => '\u{e89f}',
        Symbol::Gallery => '\u{e7aa}',
        Symbol::Sort => '\u{e8cb}',
        Symbol::Share => '\u{e72d}',
        Symbol::Tag => '\u{e8ec}',
        Symbol::Disclosure => '\u{e76c}',
        Symbol::Warning => '\u{e7ba}',
    }
}

fn add_action_button(
    host: &mut HostWindow,
    action: &ToolbarAction,
    item_enabled: bool,
    display: ToolbarDisplay,
    right_aligned: bool,
) -> Result<(), WindowsDiagnostic> {
    let presentation = toolbar_presentation(display, action.symbol, &action.label);
    add_toolbar_button(
        host,
        &presentation,
        &action.label,
        &action.help,
        item_enabled && action.enabled,
        right_aligned,
        EventBindings::activate(action.on_activate.clone()),
    )
}

fn add_toolbar_button(
    host: &mut HostWindow,
    presentation: &ToolbarPresentation,
    accessibility_label: &str,
    tooltip: &str,
    enabled: bool,
    right_aligned: bool,
    events: EventBindings,
) -> Result<(), WindowsDiagnostic> {
    let id = host.next_id;
    host.next_id += 1;
    let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON;
    let hwnd = toolbar_control(
        host.hwnd,
        BUTTON_CLASS,
        &presentation.text,
        style,
        id,
        if presentation.symbol_only {
            host.symbol_font.0
        } else {
            host.font.0
        },
        host.dark,
    )?;
    set_accessible_name(hwnd, accessibility_label)?;
    add_toolbar_tooltip(host, hwnd, tooltip)?;
    set_enabled(hwnd, enabled);
    host.commands.insert(id, Command::Activate(events));
    host.toolbar.push(ToolbarControl {
        hwnd,
        width: presentation.width,
        right_aligned,
        essential: false,
        symbol_only: presentation.symbol_only,
    });
    Ok(())
}

fn add_pane_toggle(
    host: &mut HostWindow,
    label: &str,
    sidebar: bool,
    right_aligned: bool,
) -> Result<(), WindowsDiagnostic> {
    let id = host.next_id;
    host.next_id += 1;
    let text = if sidebar { '\u{e8a0}' } else { '\u{e90d}' }.to_string();
    let hwnd = toolbar_control(
        host.hwnd,
        BUTTON_CLASS,
        &text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX | BS_PUSHLIKE,
        id,
        host.symbol_font.0,
        host.dark,
    )?;
    set_accessible_name(hwnd, label)?;
    add_toolbar_tooltip(host, hwnd, label)?;
    // SAFETY: the button accepts BM_SETCHECK and is initialized visible.
    unsafe {
        let _ = send_message(hwnd, BM_SETCHECK, BST_CHECKED, 0);
    }
    host.commands.insert(
        id,
        if sidebar {
            Command::ToggleSidebar { hwnd }
        } else {
            Command::ToggleInspector { hwnd }
        },
    );
    host.toolbar.push(ToolbarControl {
        hwnd,
        width: 40,
        right_aligned,
        essential: true,
        symbol_only: true,
    });
    Ok(())
}

fn add_toolbar_tooltip(
    host: &mut HostWindow,
    control: HWND,
    text: &str,
) -> Result<(), WindowsDiagnostic> {
    if text.trim().is_empty() {
        return Ok(());
    }
    if host.tooltip.is_null() {
        host.tooltip = create_window(
            TOOLTIP_CLASS,
            "",
            WS_POPUP | TTS_ALWAYSTIP,
            WS_EX_TOPMOST,
            host.hwnd,
            null_mut(),
        )?;
        apply_native_theme(host.tooltip, host.dark);
    }
    let mut text = wide(text).into_boxed_slice();
    let mut tool = TTTOOLINFOW {
        // `TTM_ADDTOOLW` consumes the version-2 prefix. Windows Server 2025 rejects the
        // newer allocation size that includes `lpReserved`, so derive the SDK's
        // `TTTOOLINFO_V2_SIZE` boundary from the field layout instead of hard-coding it.
        cbSize: u32::try_from(std::mem::offset_of!(TTTOOLINFOW, lpReserved)).unwrap_or(u32::MAX),
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: host.hwnd,
        uId: control as usize,
        rect: RECT::default(),
        hinst: null_mut(),
        lpszText: text.as_mut_ptr(),
        lParam: 0,
        lpReserved: null_mut(),
    };
    // SAFETY: the tooltip and control are live on the UI thread; the retained text allocation
    // remains stable until the host destroys the tooltip.
    let added = unsafe { send_message(host.tooltip, TTM_ADDTOOLW, 0, (&raw mut tool) as isize) };
    if added == 0 {
        return Err(WindowsDiagnostic::InvalidNativeState {
            reason: "native toolbar tooltip registration failed".to_owned(),
        });
    }
    host.tooltip_texts.push(text);
    Ok(())
}

fn show_command_menu(owner: HWND, button: HWND, entries: &[MenuCommand]) {
    // SAFETY: the menu exists only for this synchronous popup interaction.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        for (index, entry) in entries.iter().enumerate() {
            match entry {
                MenuCommand::Action { label, enabled, .. } => {
                    let label = wide(label);
                    let flags = MF_STRING | if *enabled { 0 } else { MF_GRAYED };
                    let _ = AppendMenuW(menu, flags, index + 1, label.as_ptr());
                }
                MenuCommand::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, null());
                }
            }
        }
        let mut rect = RECT::default();
        let _ = GetWindowRect(button, &mut rect);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            rect.left,
            rect.bottom,
            0,
            owner,
            null(),
        );
        let _ = DestroyMenu(menu);
        if selected > 0
            && let Some(MenuCommand::Action {
                enabled: true,
                events,
                ..
            }) = entries.get(selected as usize - 1)
        {
            events.emit_activate();
        }
    }
}

fn toolbar_control(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    id: usize,
    font: HFONT,
    dark: bool,
) -> Result<HWND, WindowsDiagnostic> {
    let hwnd = create_window(class, text, style, 0, parent, id as HMENU)?;
    set_native_font(hwnd, font);
    apply_native_theme(hwnd, dark);
    Ok(hwnd)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            // SAFETY: DefWindowProc handles initial creation until HostWindow is installed.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_CREATE => 0,
        WM_SIZE => {
            if let Some(host) = host_window(hwnd) {
                let width = low_word(lparam as usize) as i32;
                let height = high_word(lparam as usize) as i32;
                host.relayout(width, height);
            }
            0
        }
        WM_ACTIVATEAPP => {
            set_inactive_panels_visible(wparam != 0);
            // SAFETY: activation bookkeeping is complete; the default procedure retains
            // standard top-level activation behavior.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_DPICHANGED => {
            if let Some(host) = host_window(hwnd) {
                host.set_dpi(high_word(wparam) as u32);
                let suggested = lparam as *const RECT;
                if !suggested.is_null() {
                    // SAFETY: WM_DPICHANGED supplies a valid RECT for this synchronous call.
                    let rect = unsafe { &*suggested };
                    move_window(
                        hwnd,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        true,
                    );
                }
            }
            0
        }
        WM_GETMINMAXINFO => {
            if let Some(host) = host_window(hwnd) {
                let info = lparam as *mut MINMAXINFO;
                if !info.is_null() {
                    let content_width = scale(host.minimum_width, host.dpi);
                    let content_height =
                        scale(host.minimum_height, host.dpi) + host.toolbar_height();
                    if let Ok((outer_width, outer_height)) = outer_size_for_content(
                        content_width,
                        content_height,
                        host.window_style,
                        host.window_extended_style,
                        host.dpi,
                    ) {
                        // SAFETY: WM_GETMINMAXINFO supplies writable MINMAXINFO storage.
                        unsafe {
                            (*info).ptMinTrackSize.x = outer_width;
                            (*info).ptMinTrackSize.y = outer_height;
                        }
                    }
                }
            }
            0
        }
        WM_COMMAND => {
            if let Some(host) = host_window(hwnd) {
                host.command(low_word(wparam) as usize, high_word(wparam));
            }
            0
        }
        WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            if let Some(host) = host_window(hwnd)
                && host.dark
            {
                return configure_dark_device_context(wparam as HDC, host.background_brush);
            }
            // SAFETY: unhandled color messages use the registered default procedure.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_ERASE_BACKGROUND => {
            if let Some(host) = host_window(hwnd)
                && host.dark
            {
                paint_dark_background(hwnd, wparam as HDC, host.background_brush);
                return 1;
            }
            // SAFETY: unhandled erase messages use the registered default procedure.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_DESTROY => {
            let is_managed_window = host_window(hwnd).is_some();
            let last_window = is_managed_window
                && TOP_LEVEL_WINDOWS.with(|windows| {
                    let mut windows = windows.borrow_mut();
                    windows.retain(|candidate| *candidate != hwnd);
                    windows.is_empty()
                });
            if last_window && MESSAGE_LOOP_ACTIVE.with(Cell::get) {
                // SAFETY: the application terminates only after its final managed window closes.
                unsafe { PostQuitMessage(0) };
            }
            0
        }
        WM_NCDESTROY => {
            // SAFETY: the pointer was allocated by Box::into_raw exactly once.
            unsafe {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HostWindow;
                if !pointer.is_null() {
                    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(pointer));
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        _ => {
            // SAFETY: unhandled messages are delegated to the registered window procedure.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

fn set_inactive_panels_visible(application_active: bool) {
    let windows = TOP_LEVEL_WINDOWS.with(|windows| windows.borrow().clone());
    for hwnd in windows {
        let Some(host) = host_window(hwnd) else {
            continue;
        };
        let Some(behavior) = host.panel_behavior else {
            continue;
        };
        if !behavior.hides_when_inactive {
            continue;
        }
        let command = if application_active {
            if behavior.accepts_keyboard {
                SW_SHOW
            } else {
                SW_SHOWNOACTIVATE
            }
        } else {
            SW_HIDE
        };
        // SAFETY: the registry contains live top-level HWND values on the current UI thread.
        unsafe {
            ShowWindow(hwnd, command);
        }
    }
}

fn host_window(hwnd: HWND) -> Option<&'static mut HostWindow> {
    // SAFETY: the pointer is owned by the window between installation and WM_NCDESTROY.
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HostWindow;
        pointer.as_mut()
    }
}

unsafe extern "system" fn element_subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    let inner = reference_data as *const HandleInner;
    if inner.is_null() {
        // SAFETY: Windows owns the default subclass procedure.
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    }
    // For controls whose state changes in the default procedure, run the default
    // handler first and then emit through the stable Rust event slot.
    // SAFETY: the native subclass API permits calling DefSubclassProc once per message.
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    // SAFETY: reference_data points at the live Rc allocation until subclass removal.
    let handle = unsafe { &*inner };
    if handle.dark {
        match message {
            WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
                return configure_dark_device_context(wparam as HDC, handle.background_brush);
            }
            WM_ERASE_BACKGROUND => {
                paint_dark_background(hwnd, wparam as HDC, handle.background_brush);
                return 1;
            }
            _ => {}
        }
    }
    if message == WM_NOTIFY_MESSAGE {
        emit_list_notification(handle, lparam);
    }
    if message == WM_COMMAND {
        emit_control_command(handle, wparam, lparam);
    }
    match (handle.kind, message) {
        (HostKind::Element(ElementKind::ListRow), WM_LBUTTONUP) => {
            handle.events.emit_activate();
        }
        (HostKind::Element(ElementKind::ListRow), WM_KEYUP)
            if wparam == VK_SPACE as usize || wparam == VK_RETURN as usize =>
        {
            handle.events.emit_activate();
        }
        (HostKind::Element(ElementKind::List), WM_KEYUP) if wparam == VK_RETURN as usize => {
            activate_selected_list_row(handle);
        }
        (HostKind::Element(ElementKind::List), WM_LBUTTONDBLCLK) => {
            toggle_selected_list_row(handle);
        }
        (HostKind::Element(ElementKind::List), WM_KEYUP) if wparam == VK_SPACE as usize => {
            toggle_selected_list_row(handle);
        }
        _ => {}
    }
    result
}

fn emit_control_command(parent: &HandleInner, wparam: WPARAM, lparam: LPARAM) {
    let source = lparam as HWND;
    if source.is_null() {
        return;
    }
    let child = parent
        .children
        .borrow()
        .iter()
        .find(|child| child.0.hwnd == source)
        .cloned();
    let Some(child) = child else {
        return;
    };
    let notification = high_word(wparam);
    match child.0.kind {
        HostKind::Element(ElementKind::Button) if notification == BN_CLICKED => {
            child.0.events.emit_activate();
        }
        HostKind::Element(ElementKind::Toggle) if notification == BN_CLICKED => {
            child.0.events.emit_toggle(button_checked(source));
        }
        HostKind::Element(ElementKind::Input) if notification == EN_CHANGE => {
            child.0.events.emit_input(window_text(source));
        }
        _ => {}
    }
}

fn emit_list_notification(parent: &HandleInner, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    // SAFETY: every WM_NOTIFY structure begins with NMHDR for this synchronous call.
    let header = unsafe { &*(lparam as *const NmHdr) };
    let list = parent
        .children
        .borrow()
        .iter()
        .find(|child| child.0.hwnd == header.hwnd_from)
        .cloned();
    let Some(list) = list else {
        return;
    };
    if list.0.list_rebuilding.get() {
        return;
    }
    if header.code as i32 == TVN_SELCHANGEDW {
        // SAFETY: the notification code identifies NMTREEVIEWW storage.
        let notification = unsafe { &*(lparam as *const NmTreeViewW) };
        let row = notification.item_new.l_param as *const HandleInner;
        // SAFETY: inserted tree items retain their declarative row allocation.
        if let Some(row) = unsafe { row.as_ref() } {
            row.events.emit_activate();
        }
        return;
    }
    if header.code as i32 == TVN_ITEMEXPANDEDW {
        // SAFETY: the notification code identifies NMTREEVIEWW storage.
        let notification = unsafe { &*(lparam as *const NmTreeViewW) };
        let row = notification.item_new.l_param as *const HandleInner;
        // SAFETY: every inserted tree item retains an Rc pointer until it is deleted.
        if let Some(row) = unsafe { row.as_ref() } {
            row.events
                .emit_toggle(notification.action == TVE_EXPAND as u32);
        }
        return;
    }
    if header.code as i32 == LVN_ITEMCHANGED {
        // SAFETY: the notification code identifies NMLISTVIEW storage.
        let notification = unsafe { &*(lparam as *const NmListView) };
        let became_selected = notification.changed & LVIF_STATE != 0
            && notification.new_state & LVIS_SELECTED != 0
            && notification.old_state & LVIS_SELECTED == 0;
        if became_selected {
            let row = notification.l_param as *const HandleInner;
            // SAFETY: inserted list items retain their declarative row allocation.
            if let Some(row) = unsafe { row.as_ref() } {
                row.events.emit_activate();
            }
        }
        return;
    }
    if header.code as i32 != LVN_COLUMNCLICK {
        return;
    }
    // SAFETY: the notification code identifies NMLISTVIEW storage.
    let notification = unsafe { &*(lparam as *const NmListView) };
    let Some(Props::List {
        style: ListStyle::Table,
        columns,
        ..
    }) = list.0.props.borrow().clone()
    else {
        return;
    };
    let Ok(index) = usize::try_from(notification.sub_item) else {
        return;
    };
    let Some(column) = columns.get(index) else {
        return;
    };
    if !column.sortable {
        return;
    }
    let direction = match column.sort_direction {
        Some(SortDirection::Ascending) => SortDirection::Descending,
        Some(SortDirection::Descending) | None => SortDirection::Ascending,
    };
    list.0.events.emit_sort(TableSort {
        column_id: column.id.clone(),
        direction,
    });
}

fn activate_selected_list_row(handle: &HandleInner) {
    if let Some(row) = selected_list_row(handle) {
        row.events.emit_activate();
    }
}

fn toggle_selected_list_row(handle: &HandleInner) {
    if matches!(
        handle.props.borrow().as_ref(),
        Some(Props::List {
            style: ListStyle::Source,
            ..
        })
    ) {
        return;
    }
    let Some(row) = selected_list_row(handle) else {
        return;
    };
    let Some(Props::ListRow { expanded, .. }) = row.props.borrow().clone() else {
        return;
    };
    if !row.children.borrow().is_empty() {
        row.events.emit_toggle(!expanded);
    }
}

fn selected_list_row(handle: &HandleInner) -> Option<&HandleInner> {
    let Some(Props::List { style, .. }) = handle.props.borrow().clone() else {
        return None;
    };
    // SAFETY: the query messages synchronously populate initialized item structures.
    unsafe {
        let row_pointer = if style == ListStyle::Source {
            let selected = send_message(handle.hwnd, TVM_GETNEXTITEM, TVGN_CARET, 0);
            if selected == 0 {
                return None;
            }
            let mut item = TvItemExW {
                mask: TVIF_PARAM,
                item: selected,
                state: 0,
                state_mask: 0,
                psz_text: null_mut(),
                cch_text_max: 0,
                i_image: 0,
                i_selected_image: 0,
                c_children: 0,
                l_param: 0,
                i_integral: 0,
                state_ex: 0,
                hwnd: null_mut(),
                i_expanded_image: 0,
                i_reserved: 0,
            };
            if send_message(handle.hwnd, TVM_GETITEMW, 0, (&raw mut item) as isize) == 0 {
                return None;
            }
            item.l_param as *const HandleInner
        } else {
            let index = send_message(handle.hwnd, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED);
            if index < 0 {
                return None;
            }
            let mut item = LvItemW {
                mask: LVIF_PARAM,
                i_item: index as i32,
                i_sub_item: 0,
                state: 0,
                state_mask: 0,
                psz_text: null_mut(),
                cch_text_max: 0,
                i_image: 0,
                l_param: 0,
                i_indent: 0,
                i_group_id: 0,
                c_columns: 0,
                pu_columns: null_mut(),
                pi_col_fmt: null_mut(),
                i_group: 0,
            };
            if send_message(handle.hwnd, LVM_GETITEMW, 0, (&raw mut item) as isize) == 0 {
                return None;
            }
            item.l_param as *const HandleInner
        };
        row_pointer.as_ref()
    }
}

fn native_description(element: &Element) -> (&'static str, String, u32, u32) {
    match element.props() {
        Props::Label {
            text,
            selectable: true,
            ..
        } => (
            EDIT_CLASS,
            text.clone(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL | ES_READONLY,
            0,
        ),
        Props::Label { text, .. } => (
            STATIC_CLASS,
            text.clone(),
            WS_CHILD | WS_VISIBLE | SS_NOTIFY | SS_LEFT,
            0,
        ),
        Props::Button { label, role, .. } => (
            BUTTON_CLASS,
            label.clone(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | if *role == ButtonRole::Primary {
                    BS_DEFPUSHBUTTON
                } else {
                    BS_PUSHBUTTON
                },
            0,
        ),
        Props::Input { value, kind, .. } => (
            EDIT_CLASS,
            value.clone(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | ES_AUTOHSCROLL
                | if *kind == InputKind::Secure {
                    ES_PASSWORD
                } else {
                    0
                },
            WS_EX_CLIENTEDGE,
        ),
        Props::Toggle { label, .. } => (
            BUTTON_CLASS,
            label.clone(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX,
            0,
        ),
        Props::Progress { .. } => (
            PROGRESS_CLASS,
            String::new(),
            WS_CHILD | WS_VISIBLE | PBS_SMOOTH,
            0,
        ),
        Props::Separator { axis } => (
            STATIC_CLASS,
            String::new(),
            WS_CHILD
                | WS_VISIBLE
                | if *axis == Axis::Horizontal {
                    SS_ETCHEDHORZ
                } else {
                    SS_ETCHEDVERT
                },
            0,
        ),
        Props::Spacer { .. }
        | Props::Stack { .. }
        | Props::Scroll { .. }
        | Props::Split { .. }
        | Props::Workspace { .. } => (
            STATIC_CLASS,
            String::new(),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            WS_EX_CONTROLPARENT,
        ),
        Props::List { style, .. } if *style == ListStyle::Source => (
            TREE_VIEW_CLASS,
            String::new(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | TVS_HASBUTTONS
                | TVS_HASLINES
                | TVS_LINESATROOT
                | TVS_SHOWSELALWAYS
                | TVS_FULLROWSELECT,
            WS_EX_CLIENTEDGE,
        ),
        Props::List { style, .. } => (
            LIST_VIEW_CLASS,
            String::new(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | LVS_SINGLESEL
                | LVS_SHOWSELALWAYS
                | if *style == ListStyle::Table {
                    LVS_REPORT
                } else {
                    LVS_LIST | LVS_NOSORTHEADER
                },
            WS_EX_CLIENTEDGE,
        ),
        Props::ListRow {
            accessibility_label,
            ..
        } => (
            STATIC_CLASS,
            accessibility_label.clone(),
            WS_CHILD | SS_NOTIFY,
            0,
        ),
        Props::Status {
            title,
            message,
            tone,
        } => (
            STATIC_CLASS,
            format!("{}\r\n{}", status_prefix(*tone, title), message),
            WS_CHILD | WS_VISIBLE | SS_CENTER | SS_NOTIFY,
            WS_EX_CONTROLPARENT,
        ),
    }
}

fn status_prefix(tone: StatusTone, title: &str) -> String {
    match tone {
        StatusTone::Error => format!("⚠ {title}"),
        StatusTone::Busy => format!("… {title}"),
        StatusTone::Empty | StatusTone::Informational => title.to_owned(),
    }
}

fn apply_initial_properties(handle: &WindowsHandle) {
    let Some(props) = handle.0.props.borrow().clone() else {
        return;
    };
    match &props {
        Props::Input { placeholder, .. } => set_cue_banner(handle.0.hwnd, placeholder),
        Props::Toggle { value, .. } => set_button_checked(handle.0.hwnd, *value),
        Props::Progress { fraction, .. } => set_progress(handle.0.hwnd, *fraction),
        _ => {}
    }
    match props {
        Props::Button { enabled, .. }
        | Props::Input { enabled, .. }
        | Props::Toggle { enabled, .. } => set_enabled(handle.0.hwnd, enabled),
        _ => {}
    }
}

fn accessibility_name(element: &Element) -> Option<&str> {
    accessibility_name_for_props(element.props())
}

fn accessibility_name_for_props(props: &Props) -> Option<&str> {
    match props {
        Props::Label { text, .. } => Some(text),
        Props::Button {
            accessibility_label,
            ..
        }
        | Props::Input {
            accessibility_label,
            ..
        }
        | Props::Toggle {
            accessibility_label,
            ..
        }
        | Props::Progress {
            accessibility_label,
            ..
        }
        | Props::List {
            accessibility_label,
            ..
        }
        | Props::ListRow {
            accessibility_label,
            ..
        } => Some(accessibility_label),
        Props::Status { title, .. } => Some(title),
        _ => None,
    }
}

fn set_accessible_name(hwnd: HWND, name: &str) -> Result<(), WindowsDiagnostic> {
    let name = wide(name);
    ACCESSIBILITY_SERVICE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            // SAFETY: COM is initialized on the UI thread before native elements are created.
            let service =
                unsafe { CoCreateInstance(&CAccPropServices, None, CLSCTX_INPROC_SERVER) }
                    .map_err(|error| WindowsDiagnostic::InvalidNativeState {
                        reason: format!(
                            "accessibility annotation service creation failed: {error}"
                        ),
                    })?;
            *slot = Some(service);
        }
        let service = slot
            .as_ref()
            .ok_or_else(|| WindowsDiagnostic::InvalidNativeState {
                reason: "accessibility annotation service was not retained".to_owned(),
            })?;
        // SAFETY: the retained service synchronously copies the string and remains alive until
        // every HWND has been destroyed and the UI thread leaves its COM apartment.
        unsafe {
            service.SetHwndPropStr(
                WindowsHwnd(hwnd),
                OBJID_CLIENT.0 as u32,
                CHILDID_SELF,
                Name_Property_GUID,
                PCWSTR(name.as_ptr()),
            )
        }
        .map_err(|error| WindowsDiagnostic::InvalidNativeState {
            reason: format!("UI Automation name annotation failed: {error}"),
        })?;
        // SAFETY: the same retained service also owns the Active Accessibility name used by
        // native Win32 assistive clients and UIA's legacy bridge.
        unsafe {
            service.SetHwndPropStr(
                WindowsHwnd(hwnd),
                OBJID_CLIENT.0 as u32,
                CHILDID_SELF,
                PROPID_ACC_NAME,
                PCWSTR(name.as_ptr()),
            )
        }
        .map_err(|error| WindowsDiagnostic::InvalidNativeState {
            reason: format!("Active Accessibility name annotation failed: {error}"),
        })?;
        Ok(())
    })
}

fn apply_patch_to_native(handle: &WindowsHandle, patch: &PropertyPatch) {
    match patch {
        PropertyPatch::Label { text, .. } => set_window_text(handle.0.hwnd, text),
        PropertyPatch::Button { label, enabled, .. } => {
            set_window_text(handle.0.hwnd, label);
            set_enabled(handle.0.hwnd, *enabled);
        }
        PropertyPatch::Input {
            value,
            placeholder,
            enabled,
            ..
        } => {
            if window_text(handle.0.hwnd) != *value {
                set_window_text(handle.0.hwnd, value);
            }
            set_cue_banner(handle.0.hwnd, placeholder);
            set_enabled(handle.0.hwnd, *enabled);
        }
        PropertyPatch::Toggle {
            label,
            value,
            enabled,
            ..
        } => {
            set_window_text(handle.0.hwnd, label);
            set_button_checked(handle.0.hwnd, *value);
            set_enabled(handle.0.hwnd, *enabled);
        }
        PropertyPatch::Progress { fraction, .. } => set_progress(handle.0.hwnd, *fraction),
        PropertyPatch::Status {
            title,
            message,
            tone,
        } => set_window_text(
            handle.0.hwnd,
            &format!("{}\r\n{}", status_prefix(*tone, title), message),
        ),
        _ => {}
    }
}

fn props_from_patch(patch: &PropertyPatch) -> Props {
    match patch {
        PropertyPatch::Label {
            text,
            role,
            selectable,
        } => Props::Label {
            text: text.clone(),
            role: *role,
            selectable: *selectable,
        },
        PropertyPatch::Button {
            label,
            role,
            size,
            material,
            enabled,
            tooltip,
            accessibility_label,
        } => Props::Button {
            label: label.clone(),
            role: *role,
            size: *size,
            material: *material,
            enabled: *enabled,
            tooltip: tooltip.clone(),
            accessibility_label: accessibility_label.clone(),
        },
        PropertyPatch::Input {
            value,
            placeholder,
            kind,
            enabled,
            accessibility_label,
        } => Props::Input {
            value: value.clone(),
            placeholder: placeholder.clone(),
            kind: *kind,
            enabled: *enabled,
            accessibility_label: accessibility_label.clone(),
        },
        PropertyPatch::Toggle {
            label,
            value,
            size,
            enabled,
            accessibility_label,
        } => Props::Toggle {
            label: label.clone(),
            value: *value,
            size: *size,
            enabled: *enabled,
            accessibility_label: accessibility_label.clone(),
        },
        PropertyPatch::Progress {
            fraction,
            accessibility_label,
        } => Props::Progress {
            fraction: *fraction,
            accessibility_label: accessibility_label.clone(),
        },
        PropertyPatch::Separator { axis } => Props::Separator { axis: *axis },
        PropertyPatch::Stack {
            axis,
            spacing,
            padding,
            align,
            justify,
        } => Props::Stack {
            axis: *axis,
            spacing: *spacing,
            padding: *padding,
            align: *align,
            justify: *justify,
        },
        PropertyPatch::Spacer {
            horizontal,
            vertical,
        } => Props::Spacer {
            horizontal: *horizontal,
            vertical: *vertical,
        },
        PropertyPatch::Scroll { axis } => Props::Scroll { axis: *axis },
        PropertyPatch::Split { role, collapsible } => Props::Split {
            role: *role,
            collapsible: *collapsible,
        },
        PropertyPatch::Workspace {
            sidebar_collapsible,
            inspector_collapsible,
        } => Props::Workspace {
            sidebar_collapsible: *sidebar_collapsible,
            inspector_collapsible: *inspector_collapsible,
        },
        PropertyPatch::List {
            accessibility_label,
            style,
            columns,
        } => Props::List {
            accessibility_label: accessibility_label.clone(),
            style: *style,
            columns: columns.clone(),
        },
        PropertyPatch::ListRow {
            title,
            subtitle,
            cells,
            role,
            expanded,
            symbol,
            selected,
            disclosure,
            accessibility_label,
        } => Props::ListRow {
            title: title.clone(),
            subtitle: subtitle.clone(),
            cells: cells.clone(),
            role: *role,
            expanded: *expanded,
            symbol: *symbol,
            selected: *selected,
            disclosure: *disclosure,
            accessibility_label: accessibility_label.clone(),
        },
        PropertyPatch::Status {
            title,
            message,
            tone,
        } => Props::Status {
            title: title.clone(),
            message: message.clone(),
            tone: *tone,
        },
    }
}

fn structural(handle: &WindowsHandle) -> bool {
    matches!(
        handle.0.kind,
        HostKind::Root
            | HostKind::Element(
                ElementKind::Stack
                    | ElementKind::Scroll
                    | ElementKind::Split
                    | ElementKind::Workspace
                    | ElementKind::List
                    | ElementKind::Status
            )
    )
}

fn is_flexible(handle: &WindowsHandle, axis: Axis) -> bool {
    match handle.0.props.borrow().as_ref() {
        Some(Props::Spacer {
            horizontal,
            vertical,
        }) => match axis {
            Axis::Horizontal => *horizontal,
            Axis::Vertical => *vertical,
        },
        Some(
            Props::List { .. }
            | Props::Scroll { .. }
            | Props::Split { .. }
            | Props::Workspace { .. },
        ) => true,
        Some(Props::Stack { .. }) => handle
            .0
            .children
            .borrow()
            .iter()
            .any(|child| is_flexible(child, axis)),
        _ => false,
    }
}

fn find_ancestor_list(root: &WindowsHandle, owner: HWND) -> Option<WindowsHandle> {
    if root.0.kind == HostKind::Element(ElementKind::List) && root.0.hwnd == owner {
        return Some(root.clone());
    }
    for child in root.0.children.borrow().iter() {
        if let Some(value) = find_ancestor_list(child, owner) {
            return Some(value);
        }
    }
    None
}

fn create_window(
    class: &str,
    text: &str,
    style: u32,
    extended: u32,
    parent: HWND,
    menu: HMENU,
) -> Result<HWND, WindowsDiagnostic> {
    let class = wide(class);
    let text = wide(text);
    let instance = module_instance()?;
    // SAFETY: the class and title UTF-16 buffers remain alive for the synchronous call.
    let hwnd = unsafe {
        CreateWindowExW(
            extended,
            class.as_ptr(),
            text.as_ptr(),
            style,
            0,
            0,
            1,
            1,
            parent,
            menu,
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        Err(last_error("CreateWindowExW(control)"))
    } else {
        Ok(hwnd)
    }
}

fn module_instance() -> Result<HINSTANCE, WindowsDiagnostic> {
    // SAFETY: a null module name requests the current executable module.
    let instance = unsafe { GetModuleHandleW(null()) };
    if instance.is_null() {
        Err(last_error("GetModuleHandleW"))
    } else {
        Ok(instance)
    }
}

fn last_error(operation: &'static str) -> WindowsDiagnostic {
    // SAFETY: GetLastError reads thread-local operating-system state.
    WindowsDiagnostic::NativeOperation {
        operation,
        code: unsafe { GetLastError() },
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn class_name(hwnd: HWND) -> String {
    let mut storage = [0u16; 128];
    // SAFETY: storage is writable and hwnd remains owned by the handle.
    let length = unsafe { GetClassNameW(hwnd, storage.as_mut_ptr(), storage.len() as i32) };
    String::from_utf16_lossy(&storage[..usize::try_from(length).unwrap_or_default()])
}

fn system_message_font(dpi: u32) -> Result<Rc<NativeFont>, WindowsDiagnostic> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: u32::try_from(size_of::<NONCLIENTMETRICSW>()).unwrap_or(u32::MAX),
        ..Default::default()
    };
    // SAFETY: metrics is writable for the duration of the synchronous system query.
    unsafe {
        if SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast::<c_void>(),
            0,
            dpi.max(96),
        ) == 0
        {
            return Err(last_error("SystemParametersInfoForDpi(message font)"));
        }
        let font = CreateFontIndirectW(&raw const metrics.lfMessageFont);
        if font.is_null() {
            return Err(last_error("CreateFontIndirectW(message font)"));
        }
        Ok(Rc::new(NativeFont(font)))
    }
}

fn system_symbol_font(dpi: u32) -> Result<Rc<NativeFont>, WindowsDiagnostic> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: u32::try_from(size_of::<NONCLIENTMETRICSW>()).unwrap_or(u32::MAX),
        ..Default::default()
    };
    // SAFETY: metrics is writable for the duration of the synchronous system query.
    unsafe {
        if SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast::<c_void>(),
            0,
            dpi.max(96),
        ) == 0
        {
            return Err(last_error("SystemParametersInfoForDpi(symbol font)"));
        }
        metrics.lfMessageFont.lfHeight = -scale(16, dpi.max(96));
        metrics.lfMessageFont.lfWeight = 400;
        let family = wide("Segoe Fluent Icons");
        let copy_length = family.len().min(metrics.lfMessageFont.lfFaceName.len());
        metrics.lfMessageFont.lfFaceName[..copy_length].copy_from_slice(&family[..copy_length]);
        let font = CreateFontIndirectW(&raw const metrics.lfMessageFont);
        if font.is_null() {
            return Err(last_error("CreateFontIndirectW(symbol font)"));
        }
        Ok(Rc::new(NativeFont(font)))
    }
}

fn apply_semantic_font_tree(
    handle: &WindowsHandle,
    dpi: u32,
    base_font: HFONT,
) -> Result<(), WindowsDiagnostic> {
    apply_semantic_font(handle, dpi, base_font)?;
    for child in handle.0.children.borrow().iter() {
        apply_semantic_font_tree(child, dpi, base_font)?;
    }
    Ok(())
}

fn apply_semantic_font(
    handle: &WindowsHandle,
    dpi: u32,
    base_font: HFONT,
) -> Result<(), WindowsDiagnostic> {
    let role = match handle.0.props.borrow().as_ref() {
        Some(Props::Label { role, .. }) => Some(*role),
        _ => None,
    };
    let font = match role {
        Some(role @ (TextRole::Title | TextRole::Heading | TextRole::Monospace)) => {
            Some(system_text_role_font(dpi, role)?)
        }
        Some(TextRole::Body | TextRole::Secondary) | None => None,
    };
    set_native_font(
        handle.0.hwnd,
        font.as_ref().map_or(base_font, |font| font.0),
    );
    *handle.0.semantic_font.borrow_mut() = font;
    Ok(())
}

fn clear_semantic_fonts(handle: &WindowsHandle, base_font: HFONT) {
    set_native_font(handle.0.hwnd, base_font);
    *handle.0.semantic_font.borrow_mut() = None;
    for child in handle.0.children.borrow().iter() {
        clear_semantic_fonts(child, base_font);
    }
}

fn system_text_role_font(dpi: u32, role: TextRole) -> Result<Rc<NativeFont>, WindowsDiagnostic> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: u32::try_from(size_of::<NONCLIENTMETRICSW>()).unwrap_or(u32::MAX),
        ..Default::default()
    };
    // SAFETY: metrics is writable for the duration of the synchronous system query.
    unsafe {
        if SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast::<c_void>(),
            0,
            dpi.max(96),
        ) == 0
        {
            return Err(last_error("SystemParametersInfoForDpi(text role font)"));
        }
        match role {
            TextRole::Title => {
                metrics.lfMessageFont.lfHeight = -scale(22, dpi.max(96));
                metrics.lfMessageFont.lfWeight = 600;
                set_logfont_family(
                    &mut metrics.lfMessageFont.lfFaceName,
                    "Segoe UI Variable Display",
                );
            }
            TextRole::Heading => {
                metrics.lfMessageFont.lfHeight = -scale(16, dpi.max(96));
                metrics.lfMessageFont.lfWeight = 600;
                set_logfont_family(
                    &mut metrics.lfMessageFont.lfFaceName,
                    "Segoe UI Variable Display",
                );
            }
            TextRole::Monospace => {
                set_logfont_family(&mut metrics.lfMessageFont.lfFaceName, "Cascadia Mono");
            }
            TextRole::Body | TextRole::Secondary => {}
        }
        let font = CreateFontIndirectW(&raw const metrics.lfMessageFont);
        if font.is_null() {
            return Err(last_error("CreateFontIndirectW(text role font)"));
        }
        Ok(Rc::new(NativeFont(font)))
    }
}

fn set_logfont_family(target: &mut [u16], family: &str) {
    target.fill(0);
    let family = wide(family);
    let copy_length = family.len().min(target.len());
    target[..copy_length].copy_from_slice(&family[..copy_length]);
}

fn set_native_font(hwnd: HWND, font: HFONT) {
    // SAFETY: the retained NativeFont outlives every HWND receiving this synchronous message.
    unsafe {
        let _ = send_message(hwnd, WM_SETFONT, font as usize, 1);
    }
}

fn outer_size_for_content(
    width: i32,
    height: i32,
    style: u32,
    extended_style: u32,
    dpi: u32,
) -> Result<(i32, i32), WindowsDiagnostic> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width.max(1),
        bottom: height.max(1),
    };
    // SAFETY: rect is writable for the synchronous content-to-window conversion.
    unsafe {
        if AdjustWindowRectExForDpi(&mut rect, style, 0, extended_style, dpi.max(96)) == 0 {
            return Err(last_error("AdjustWindowRectExForDpi"));
        }
    }
    Ok((rect.right - rect.left, rect.bottom - rect.top))
}

fn set_window_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    // SAFETY: UTF-16 storage remains alive for the synchronous SetWindowTextW call.
    unsafe {
        let _ = SetWindowTextW(hwnd, text.as_ptr());
    }
}

fn window_text(hwnd: HWND) -> String {
    // SAFETY: the queried HWND is live and the allocated buffer includes a terminator slot.
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        let mut storage = vec![0u16; usize::try_from(length).unwrap_or_default() + 1];
        let copied = GetWindowTextW(
            hwnd,
            storage.as_mut_ptr(),
            i32::try_from(storage.len()).unwrap_or(i32::MAX),
        );
        String::from_utf16_lossy(&storage[..usize::try_from(copied).unwrap_or_default()])
    }
}

fn set_cue_banner(hwnd: HWND, text: &str) {
    let text = wide(text);
    // SAFETY: EM_SETCUEBANNER synchronously copies text into the native edit control.
    unsafe {
        let _ = send_message(hwnd, EM_SETCUEBANNER, 1, text.as_ptr() as isize);
    }
}

fn set_progress(hwnd: HWND, fraction: f64) {
    let position = (fraction.clamp(0.0, 1.0) * 100.0).round() as usize;
    // SAFETY: the progress control accepts PBM_SETPOS with an integer percentage.
    unsafe {
        let _ = send_message(hwnd, PBM_SETPOS, position, 0);
    }
}

fn set_button_checked(hwnd: HWND, checked: bool) {
    // SAFETY: the button control accepts BM_SETCHECK.
    unsafe {
        let _ = send_message(hwnd, BM_SETCHECK, usize::from(checked), 0);
    }
}

fn set_enabled(hwnd: HWND, enabled: bool) {
    // SAFETY: the HWND is live and owned by the current UI thread.
    unsafe {
        let _ = EnableWindow(hwnd, i32::from(enabled));
    }
}

fn button_checked(hwnd: HWND) -> bool {
    // SAFETY: the button control accepts BM_GETCHECK.
    unsafe { send_message(hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED as isize }
}

fn apply_native_theme(hwnd: HWND, dark: bool) {
    let theme = wide(if dark {
        "DarkMode_Explorer"
    } else {
        "Explorer"
    });
    // SAFETY: the theme strings remain alive during this synchronous call.
    unsafe {
        let _ = SetWindowTheme(hwnd, theme.as_ptr(), null());
    }
}

fn dark_appearance() -> bool {
    match std::env::var("RINKA_WINDOWS_APPEARANCE") {
        Ok(value) if value.eq_ignore_ascii_case("dark") => true,
        Ok(value) if value.eq_ignore_ascii_case("light") => false,
        Ok(_) | Err(_) => system_prefers_dark_apps(),
    }
}

fn system_prefers_dark_apps() -> bool {
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value = 1u32;
    let mut value_size = u32::try_from(size_of::<u32>()).unwrap_or(u32::MAX);
    // SAFETY: RegGetValueW synchronously writes one DWORD into `value`; both UTF-16 strings
    // and the byte-count pointer remain valid for the call.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&raw mut value).cast::<c_void>(),
            &mut value_size,
        )
    };
    result == ERROR_SUCCESS && value == 0
}

fn configure_dark_device_context(device_context: HDC, brush: HBRUSH) -> LRESULT {
    // SAFETY: the device context belongs to the synchronous color message and the brush lives
    // through the current host or retained handle.
    unsafe {
        let _ = SetBkColor(device_context, DARK_BACKGROUND);
        let _ = SetTextColor(device_context, DARK_TEXT);
    }
    brush as LRESULT
}

fn paint_dark_background(hwnd: HWND, device_context: HDC, brush: HBRUSH) {
    let mut rect = RECT::default();
    // SAFETY: the device context belongs to WM_ERASEBKGND and all values are live for the call.
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
        let _ = FillRect(device_context, &rect, brush);
    }
}

fn set_dark_title_bar(hwnd: HWND, dark: bool) {
    let value = i32::from(dark);
    // SAFETY: DwmSetWindowAttribute synchronously reads the supplied i32.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const value).cast::<c_void>(),
            u32::try_from(size_of::<i32>()).unwrap_or(u32::MAX),
        );
    }
}

fn dpi_for_window(hwnd: HWND) -> u32 {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDpiForWindow(hwnd: HWND) -> u32;
    }
    // SAFETY: the HWND is live and owned by the current UI thread.
    unsafe { GetDpiForWindow(hwnd).max(96) }
}

fn scale(value: i32, dpi: u32) -> i32 {
    value.saturating_mul(dpi as i32) / 96
}

fn show(hwnd: HWND, visible: bool) {
    // SAFETY: the HWND remains live while it is laid out.
    unsafe {
        ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }
}

fn move_window(hwnd: HWND, x: i32, y: i32, width: i32, height: i32, repaint: bool) {
    // SAFETY: the HWND remains live while its owning host performs layout.
    unsafe {
        let _ = MoveWindow(hwnd, x, y, width.max(0), height.max(0), i32::from(repaint));
    }
}

fn low_word(value: usize) -> u16 {
    (value & 0xffff) as u16
}

fn high_word(value: usize) -> u16 {
    ((value >> 16) & 0xffff) as u16
}

unsafe fn send_message(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendMessageW(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    }
    // SAFETY: the caller establishes the message-specific pointer and lifetime invariants.
    unsafe { SendMessageW(hwnd, message, wparam, lparam) }
}
