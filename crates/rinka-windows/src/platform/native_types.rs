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
