//! Win32 retained-object implementation.

use crate::{WindowsDiagnostic, validate_element};
use rinka_core::{
    Align, ApplicationSpec, Axis, ButtonRole, CollectionPattern, ControlSize, Element, ElementKind,
    EventBindings, InputKind, Justify, ListRowRole, MenuEntry, NativeBackend, PanelBehavior,
    PropertyPatch, Props, Renderer, SortDirection, Spacing, StatusTone, Symbol, TableSort,
    TextRole, ToolbarAction, ToolbarDisplay, ToolbarItem, ToolbarItemKind, UiPattern, WindowKind,
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
    IDI_APPLICATION, IsDialogMessageW, IsWindow, LoadCursorW, LoadIconW, MF_CHECKED, MF_GRAYED,
    MF_SEPARATOR, MF_STRING, MINMAXINFO, MSG, MoveWindow, NONCLIENTMETRICSW, PostQuitMessage,
    RegisterClassExW, SPI_GETNONCLIENTMETRICS, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SW_SHOWNORMAL,
    SetParent, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, TranslateMessage, WM_ACTIVATEAPP, WM_COMMAND, WM_CREATE, WM_DESTROY,
    WM_DPICHANGED, WM_GETMINMAXINFO, WM_KEYUP, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NCCREATE,
    WM_NCDESTROY, WM_SETFONT, WM_SIZE, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_GROUP, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_SYSMENU,
    WS_TABSTOP, WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL,
};

include!("platform/native_types.rs");
include!("platform/backend_layout.rs");
include!("platform/backend_contract.rs");
include!("platform/window_host.rs");
include!("platform/events.rs");
include!("platform/native_primitives.rs");
