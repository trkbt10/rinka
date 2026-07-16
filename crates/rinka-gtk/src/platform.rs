//! GTK 4 and libadwaita implementation.

use adw::prelude::*;
use gtk::{gio, glib};
use rinka_core::{
    Align, ApplicationSpec, Axis, ButtonMaterial, ButtonRole, ControlSize, Element, ElementKind,
    EventBindings, InputKind, Justify, ListRowRole, ListStyle, MountedNode, NativeBackend,
    PanelBehavior, PropertyPatch, Props, Renderer, SortDirection, Spacing, SplitRole, StatusTone,
    Symbol, TableColumn, TableSort, TextRole, ToolbarAction, ToolbarDisplay, ToolbarGroupDisplay,
    ToolbarItem, ToolbarItemKind, ToolbarMenuEntry, ToolbarPlacement, WindowKind, WindowRuntime,
    WindowSpec,
};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::error::Error;
use std::fmt;
use std::rc::Rc;

const UTILITY_PANE_MIN_WIDTH_SP: f64 = 280.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutContext {
    Standard,
    AuxiliaryPanel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostKind {
    Root,
    Element(ElementKind),
}

#[derive(Clone)]
struct Presentation {
    source: gtk::Widget,
    view: gtk::Widget,
}

struct HandleInner {
    widget: gtk::Widget,
    host_kind: HostKind,
    split_role: Option<SplitRole>,
    split_collapsible: Cell<bool>,
    workspace: Option<WorkspaceData>,
    list: Option<Rc<ListData>>,
    row: Option<Rc<RowData>>,
    row_object: Option<glib::BoxedAnyObject>,
    presentations: RefCell<Vec<Presentation>>,
    auxiliaries: Vec<gtk::Widget>,
    suppress_events: Rc<Cell<bool>>,
}

struct HandleDetails {
    host_kind: HostKind,
    split_role: Option<SplitRole>,
    workspace: Option<WorkspaceData>,
    list: Option<Rc<ListData>>,
    row: Option<Rc<RowData>>,
    row_object: Option<glib::BoxedAnyObject>,
    auxiliaries: Vec<gtk::Widget>,
    suppress_events: Rc<Cell<bool>>,
}

#[derive(Clone)]
struct WorkspaceData {
    navigation: adw::OverlaySplitView,
    inspector: adw::OverlaySplitView,
    sidebar_collapsible: Rc<Cell<bool>>,
    inspector_collapsible: Rc<Cell<bool>>,
}

type RowBinding = Box<dyn Fn(&RowData) -> bool>;

struct RowData {
    title: RefCell<String>,
    subtitle: RefCell<Option<String>>,
    cells: RefCell<Vec<String>>,
    role: Cell<ListRowRole>,
    expanded: Cell<bool>,
    symbol: Cell<Option<Symbol>>,
    selected: Cell<bool>,
    disclosure: Cell<bool>,
    accessibility_label: RefCell<String>,
    events: EventBindings,
    children: gio::ListStore,
    bindings: RefCell<Vec<RowBinding>>,
    tree_rows: RefCell<Vec<glib::WeakRef<gtk::TreeListRow>>>,
    list_owners: RefCell<Vec<std::rc::Weak<ListData>>>,
    suppress_expansion: Cell<bool>,
}

struct TableViewState {
    view: gtk::ColumnView,
    columns: Vec<(TableColumn, gtk::ColumnViewColumn)>,
}

struct ListData {
    scroll: gtk::ScrolledWindow,
    store: gio::ListStore,
    style: Cell<ListStyle>,
    columns: RefCell<Vec<TableColumn>>,
    accessibility_label: RefCell<String>,
    events: EventBindings,
    selection: RefCell<Option<gtk::SingleSelection>>,
    table: RefCell<Option<TableViewState>>,
    suppress_selection: Cell<bool>,
    suppress_sort: Cell<bool>,
    last_native_sort: RefCell<Option<TableSort>>,
}

/// Main-loop retained GTK widget handle.
#[derive(Clone)]
pub struct GtkHandle(Rc<HandleInner>);

impl GtkHandle {
    fn with_details(widget: gtk::Widget, details: HandleDetails) -> Self {
        Self(Rc::new(HandleInner {
            widget,
            host_kind: details.host_kind,
            split_role: details.split_role,
            split_collapsible: Cell::new(false),
            workspace: details.workspace,
            list: details.list,
            row: details.row,
            row_object: details.row_object,
            presentations: RefCell::new(Vec::new()),
            auxiliaries: details.auxiliaries,
            suppress_events: details.suppress_events,
        }))
    }

    fn new(
        widget: impl IsA<gtk::Widget>,
        host_kind: HostKind,
        split_role: Option<SplitRole>,
        auxiliaries: Vec<gtk::Widget>,
    ) -> Self {
        Self::with_details(
            widget.upcast(),
            HandleDetails {
                host_kind,
                split_role,
                workspace: None,
                list: None,
                row: None,
                row_object: None,
                auxiliaries,
                suppress_events: Rc::new(Cell::new(false)),
            },
        )
    }

    fn with_suppression(
        widget: impl IsA<gtk::Widget>,
        host_kind: HostKind,
        auxiliaries: Vec<gtk::Widget>,
        suppress_events: Rc<Cell<bool>>,
    ) -> Self {
        Self::with_details(
            widget.upcast(),
            HandleDetails {
                host_kind,
                split_role: None,
                workspace: None,
                list: None,
                row: None,
                row_object: None,
                auxiliaries,
                suppress_events,
            },
        )
    }

    fn workspace(widget: adw::OverlaySplitView, data: WorkspaceData) -> Self {
        Self::with_details(
            widget.upcast(),
            HandleDetails {
                host_kind: HostKind::Element(ElementKind::Workspace),
                split_role: None,
                workspace: Some(data),
                list: None,
                row: None,
                row_object: None,
                auxiliaries: Vec::new(),
                suppress_events: Rc::new(Cell::new(false)),
            },
        )
    }

    fn list(widget: gtk::ScrolledWindow, data: Rc<ListData>) -> Self {
        Self::with_details(
            widget.upcast(),
            HandleDetails {
                host_kind: HostKind::Element(ElementKind::List),
                split_role: None,
                workspace: None,
                list: Some(data),
                row: None,
                row_object: None,
                auxiliaries: Vec::new(),
                suppress_events: Rc::new(Cell::new(false)),
            },
        )
    }

    fn row(data: Rc<RowData>, object: glib::BoxedAnyObject) -> Self {
        let identity = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        Self::with_details(
            identity.upcast(),
            HandleDetails {
                host_kind: HostKind::Element(ElementKind::ListRow),
                split_role: None,
                workspace: None,
                list: None,
                row: Some(data),
                row_object: Some(object),
                auxiliaries: Vec::new(),
                suppress_events: Rc::new(Cell::new(false)),
            },
        )
    }

    fn widget(&self) -> &gtk::Widget {
        &self.0.widget
    }
}

impl RowData {
    #[allow(clippy::too_many_arguments)]
    fn new(
        title: &str,
        subtitle: Option<&str>,
        cells: &[String],
        role: ListRowRole,
        expanded: bool,
        symbol: Option<Symbol>,
        selected: bool,
        disclosure: bool,
        accessibility_label: &str,
        events: EventBindings,
    ) -> Rc<Self> {
        Rc::new(Self {
            title: RefCell::new(title.to_owned()),
            subtitle: RefCell::new(subtitle.map(str::to_owned)),
            cells: RefCell::new(cells.to_vec()),
            role: Cell::new(role),
            expanded: Cell::new(expanded),
            symbol: Cell::new(symbol),
            selected: Cell::new(selected),
            disclosure: Cell::new(disclosure),
            accessibility_label: RefCell::new(accessibility_label.to_owned()),
            events,
            children: gio::ListStore::new::<glib::BoxedAnyObject>(),
            bindings: RefCell::new(Vec::new()),
            tree_rows: RefCell::new(Vec::new()),
            list_owners: RefCell::new(Vec::new()),
            suppress_expansion: Cell::new(false),
        })
    }

    fn add_binding(&self, binding: RowBinding) {
        if binding(self) {
            self.bindings.borrow_mut().push(binding);
        }
    }

    fn refresh(&self) {
        self.bindings.borrow_mut().retain(|binding| binding(self));
    }

    fn sync_expansion(&self) {
        self.suppress_expansion.set(true);
        self.tree_rows.borrow_mut().retain(|weak| {
            let Some(row) = weak.upgrade() else {
                return false;
            };
            row.set_expanded(self.expanded.get());
            true
        });
        self.suppress_expansion.set(false);
    }

    fn attach_owner(self: &Rc<Self>, owner: &Rc<ListData>) {
        let mut owners = self.list_owners.borrow_mut();
        let weak_owner = Rc::downgrade(owner);
        if !owners.iter().any(|candidate| candidate.ptr_eq(&weak_owner)) {
            owners.push(weak_owner);
        }
        drop(owners);
        for position in 0..self.children.n_items() {
            if let Some(object) = self.children.item(position)
                && let Some(child) = row_from_object(&object)
            {
                child.attach_owner(owner);
            }
        }
    }

    fn sync_owners(&self) {
        self.list_owners.borrow_mut().retain(|owner| {
            let Some(owner) = owner.upgrade() else {
                return false;
            };
            owner.sync_selection();
            true
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &self,
        title: &str,
        subtitle: Option<&str>,
        cells: &[String],
        role: ListRowRole,
        expanded: bool,
        symbol: Option<Symbol>,
        selected: bool,
        disclosure: bool,
        accessibility_label: &str,
    ) {
        *self.title.borrow_mut() = title.to_owned();
        *self.subtitle.borrow_mut() = subtitle.map(str::to_owned);
        *self.cells.borrow_mut() = cells.to_vec();
        self.role.set(role);
        self.expanded.set(expanded);
        self.symbol.set(symbol);
        self.selected.set(selected);
        self.disclosure.set(disclosure);
        *self.accessibility_label.borrow_mut() = accessibility_label.to_owned();
        self.refresh();
        self.sync_expansion();
        self.sync_owners();
    }

    fn value_for_column(&self, index: usize) -> String {
        if index == 0 {
            self.title.borrow().clone()
        } else {
            self.cells
                .borrow()
                .get(index - 1)
                .cloned()
                .unwrap_or_default()
        }
    }
}

impl ListData {
    fn new(
        accessibility_label: &str,
        style: ListStyle,
        columns: &[TableColumn],
        events: EventBindings,
    ) -> Rc<Self> {
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hexpand(true);
        scroll.set_vexpand(true);
        configure_scroll(&scroll, Axis::Vertical);
        let data = Rc::new(Self {
            scroll,
            store: gio::ListStore::new::<glib::BoxedAnyObject>(),
            style: Cell::new(style),
            columns: RefCell::new(columns.to_vec()),
            accessibility_label: RefCell::new(accessibility_label.to_owned()),
            events,
            selection: RefCell::new(None),
            table: RefCell::new(None),
            suppress_selection: Cell::new(false),
            suppress_sort: Cell::new(false),
            last_native_sort: RefCell::new(None),
        });
        data.rebuild_presentation();
        data
    }

    fn update(
        self: &Rc<Self>,
        accessibility_label: &str,
        style: ListStyle,
        columns: &[TableColumn],
    ) {
        let previous_style = self.style.replace(style);
        let previous_columns = self.columns.replace(columns.to_vec());
        *self.accessibility_label.borrow_mut() = accessibility_label.to_owned();
        let schema_changed = !same_table_schema(&previous_columns, columns);
        if previous_style != style || (style == ListStyle::Table && schema_changed) {
            self.rebuild_presentation();
        } else {
            if let Some(child) = self.scroll.child() {
                child.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            }
            if style == ListStyle::Table {
                self.sync_table_sort();
            }
        }
        self.sync_selection();
    }

    fn make_selection(self: &Rc<Self>, model: impl IsA<gio::ListModel>) -> gtk::SingleSelection {
        let selection = gtk::SingleSelection::new(Some(model));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let weak = Rc::downgrade(self);
        selection.connect_selected_item_notify(move |selection| {
            let Some(data) = weak.upgrade() else {
                return;
            };
            if data.suppress_selection.get() {
                return;
            }
            let Some(object) = selection.selected_item() else {
                return;
            };
            let Some(row) = row_from_model_item(&object) else {
                return;
            };
            if row.role.get() == ListRowRole::Item {
                row.events.emit_activate();
            }
        });
        selection
    }

    fn rebuild_presentation(self: &Rc<Self>) {
        self.selection.borrow_mut().take();
        self.table.borrow_mut().take();
        let label = self.accessibility_label.borrow().clone();
        let widget: gtk::Widget = match self.style.get() {
            ListStyle::Source => {
                let tree_model =
                    gtk::TreeListModel::new(self.store.clone(), false, false, |object| {
                        row_from_object(object)
                            .map(|row| row.children.clone().upcast::<gio::ListModel>())
                    });
                let selection = self.make_selection(tree_model.clone());
                let factory = source_row_factory();
                let view = gtk::ListView::new(Some(selection.clone()), Some(factory));
                view.add_css_class("navigation-sidebar");
                view.set_vexpand(true);
                view.update_property(&[gtk::accessible::Property::Label(&label)]);
                view.connect_activate(|view, position| {
                    let Some(model) = view.model() else {
                        return;
                    };
                    let Some(object) = model.item(position) else {
                        return;
                    };
                    let Ok(tree_row) = object.downcast::<gtk::TreeListRow>() else {
                        return;
                    };
                    let Some(row) = tree_row.item().and_then(|item| row_from_object(&item)) else {
                        return;
                    };
                    if row.role.get() == ListRowRole::Section {
                        tree_row.set_expanded(!tree_row.is_expanded());
                    } else {
                        row.events.emit_activate();
                    }
                });
                *self.selection.borrow_mut() = Some(selection);
                view.upcast()
            }
            ListStyle::Table => self.build_table(&label),
            ListStyle::Content | ListStyle::Plain => {
                let selection = self.make_selection(self.store.clone());
                let factory = flat_row_factory();
                let view = gtk::ListView::new(Some(selection.clone()), Some(factory));
                if self.style.get() == ListStyle::Content {
                    view.add_css_class("rich-list");
                }
                view.set_vexpand(true);
                view.update_property(&[gtk::accessible::Property::Label(&label)]);
                view.connect_activate(|view, position| {
                    let Some(model) = view.model() else {
                        return;
                    };
                    let Some(object) = model.item(position) else {
                        return;
                    };
                    if let Some(row) = row_from_model_item(&object) {
                        row.events.emit_activate();
                    }
                });
                *self.selection.borrow_mut() = Some(selection);
                view.upcast()
            }
        };
        self.scroll.set_child(Some(&widget));
        self.sync_selection();
    }

    fn build_table(self: &Rc<Self>, accessibility_label: &str) -> gtk::Widget {
        let root_sort_model =
            gtk::SortListModel::new(Some(self.store.clone()), None::<gtk::Sorter>);
        let current_sorter = Rc::new(RefCell::new(None::<gtk::Sorter>));
        let sorted_models = Rc::new(RefCell::new(vec![root_sort_model.clone()]));
        let child_sorter = current_sorter.clone();
        let child_models = sorted_models.clone();
        let tree_model =
            gtk::TreeListModel::new(root_sort_model.clone(), false, false, move |object| {
                row_from_object(object).map(|row| {
                    let sorted = gtk::SortListModel::new(
                        Some(row.children.clone()),
                        child_sorter.borrow().clone(),
                    );
                    child_models.borrow_mut().push(sorted.clone());
                    sorted.upcast::<gio::ListModel>()
                })
            });
        let selection = self.make_selection(tree_model.clone());
        let view = gtk::ColumnView::new(Some(selection.clone()));
        view.set_vexpand(true);
        view.set_show_column_separators(true);
        view.set_show_row_separators(true);
        view.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
        view.connect_activate(|view, position| {
            let Some(model) = view.model() else {
                return;
            };
            let Some(object) = model.item(position) else {
                return;
            };
            if let Some(row) = row_from_model_item(&object) {
                row.events.emit_activate();
            }
        });

        let descriptions = self.columns.borrow().clone();
        let mut native_columns = Vec::with_capacity(descriptions.len());
        for (index, description) in descriptions.iter().enumerate() {
            let factory = table_cell_factory(index, &description.title);
            let column = gtk::ColumnViewColumn::new(Some(&description.title), Some(factory));
            column.set_id(Some(&description.id));
            column.set_resizable(true);
            column.set_expand(index == 0);
            if description.sortable {
                let sorter = gtk::CustomSorter::new(move |left, right| {
                    let left = row_from_model_item(left)
                        .map(|row| row.value_for_column(index))
                        .unwrap_or_default();
                    let right = row_from_model_item(right)
                        .map(|row| row.value_for_column(index))
                        .unwrap_or_default();
                    match left.cmp(&right) {
                        Ordering::Less => gtk::Ordering::Smaller,
                        Ordering::Equal => gtk::Ordering::Equal,
                        Ordering::Greater => gtk::Ordering::Larger,
                    }
                });
                column.set_sorter(Some(&sorter));
            }
            view.append_column(&column);
            native_columns.push((description.clone(), column));
        }
        if let Some(sorter) = view.sorter() {
            *current_sorter.borrow_mut() = Some(sorter.clone());
            for model in sorted_models.borrow().iter() {
                model.set_sorter(Some(&sorter));
            }
            if let Ok(sorter) = sorter.downcast::<gtk::ColumnViewSorter>() {
                let weak = Rc::downgrade(self);
                sorter.connect_primary_sort_column_notify(move |sorter| {
                    if let Some(data) = weak.upgrade() {
                        data.report_native_sort(sorter);
                    }
                });
                let weak = Rc::downgrade(self);
                sorter.connect_primary_sort_order_notify(move |sorter| {
                    if let Some(data) = weak.upgrade() {
                        data.report_native_sort(sorter);
                    }
                });
            }
        }
        *self.selection.borrow_mut() = Some(selection);
        *self.table.borrow_mut() = Some(TableViewState {
            view: view.clone(),
            columns: native_columns,
        });
        self.sync_table_sort();
        view.upcast()
    }

    fn report_native_sort(&self, sorter: &gtk::ColumnViewSorter) {
        if self.suppress_sort.get() {
            return;
        }
        let Some(column) = sorter.primary_sort_column() else {
            return;
        };
        let Some(column_id) = column.id().map(|id| id.to_string()) else {
            return;
        };
        let direction = match sorter.primary_sort_order() {
            gtk::SortType::Ascending => SortDirection::Ascending,
            gtk::SortType::Descending => SortDirection::Descending,
            _ => return,
        };
        let next = TableSort {
            column_id,
            direction,
        };
        if self.last_native_sort.borrow().as_ref() == Some(&next) {
            return;
        }
        *self.last_native_sort.borrow_mut() = Some(next.clone());
        self.events.emit_sort(next);
    }

    fn sync_table_sort(&self) {
        let descriptions = self.columns.borrow();
        let active = descriptions.iter().find_map(|column| {
            column
                .sort_direction
                .map(|direction| (column.id.as_str(), direction))
        });
        let table = self.table.borrow();
        let Some(table) = table.as_ref() else {
            return;
        };
        let Some(active) = active else {
            self.suppress_sort.set(true);
            table.view.sort_by_column(None, gtk::SortType::Ascending);
            self.suppress_sort.set(false);
            self.last_native_sort.borrow_mut().take();
            return;
        };
        let Some((_, native)) = table
            .columns
            .iter()
            .find(|(description, _)| description.id == active.0)
        else {
            return;
        };
        let direction = match active.1 {
            SortDirection::Ascending => gtk::SortType::Ascending,
            SortDirection::Descending => gtk::SortType::Descending,
        };
        self.suppress_sort.set(true);
        table.view.sort_by_column(Some(native), direction);
        self.suppress_sort.set(false);
        *self.last_native_sort.borrow_mut() = Some(TableSort {
            column_id: active.0.to_owned(),
            direction: active.1,
        });
    }

    fn sync_selection(&self) {
        let Some(selection) = self.selection.borrow().as_ref().cloned() else {
            return;
        };
        let Some(model) = selection.model() else {
            return;
        };
        let selected = (0..model.n_items()).find(|position| {
            model
                .item(*position)
                .and_then(|object| row_from_model_item(&object))
                .is_some_and(|row| row.selected.get())
        });
        self.suppress_selection.set(true);
        selection.set_selected(selected.unwrap_or(gtk::INVALID_LIST_POSITION));
        self.suppress_selection.set(false);
    }
}

fn same_table_schema(old: &[TableColumn], new: &[TableColumn]) -> bool {
    old.len() == new.len()
        && old.iter().zip(new).all(|(old, new)| {
            old.id == new.id && old.title == new.title && old.sortable == new.sortable
        })
}

fn row_from_object(object: &glib::Object) -> Option<Rc<RowData>> {
    let boxed = object.clone().downcast::<glib::BoxedAnyObject>().ok()?;
    let row = boxed.try_borrow::<Rc<RowData>>().ok()?.clone();
    Some(row)
}

fn row_from_model_item(object: &glib::Object) -> Option<Rc<RowData>> {
    if let Ok(tree_row) = object.clone().downcast::<gtk::TreeListRow>() {
        return tree_row.item().and_then(|item| row_from_object(&item));
    }
    row_from_object(object)
}

fn source_row_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_bind(|_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(object) = item.item() else {
            return;
        };
        let Ok(tree_row) = object.downcast::<gtk::TreeListRow>() else {
            return;
        };
        let Some(row) = tree_row.item().and_then(|item| row_from_object(&item)) else {
            return;
        };

        let content = gtk::Box::new(
            gtk::Orientation::Horizontal,
            spacing_pixels(Spacing::Compact),
        );
        let icon = gtk::Image::new();
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_hexpand(true);
        let subtitle = gtk::Label::new(None);
        subtitle.set_xalign(0.0);
        subtitle.add_css_class("caption");
        subtitle.add_css_class("dim-label");
        labels.append(&title);
        labels.append(&subtitle);
        let disclosure = gtk::Image::from_icon_name("go-next-symbolic");
        content.append(&icon);
        content.append(&labels);
        content.append(&disclosure);
        let expander = gtk::TreeExpander::new();
        expander.set_list_row(Some(&tree_row));
        expander.set_child(Some(&content));
        item.set_child(Some(&expander));

        let weak_item = item.downgrade();
        let weak_expander_for_binding = expander.downgrade();
        let weak_icon = icon.downgrade();
        let weak_title = title.downgrade();
        let weak_subtitle = subtitle.downgrade();
        let weak_disclosure = disclosure.downgrade();
        row.add_binding(Box::new(move |data| {
            let (
                Some(item),
                Some(expander),
                Some(icon),
                Some(title),
                Some(subtitle),
                Some(disclosure),
            ) = (
                weak_item.upgrade(),
                weak_expander_for_binding.upgrade(),
                weak_icon.upgrade(),
                weak_title.upgrade(),
                weak_subtitle.upgrade(),
                weak_disclosure.upgrade(),
            )
            else {
                return false;
            };
            let role = data.role.get();
            item.set_accessible_label(&data.accessibility_label.borrow());
            item.set_selectable(role == ListRowRole::Item);
            item.set_activatable(true);
            title.set_label(&data.title.borrow());
            if role == ListRowRole::Section {
                title.add_css_class("heading");
            } else {
                title.remove_css_class("heading");
            }
            let subtitle_text = data.subtitle.borrow();
            subtitle.set_label(subtitle_text.as_deref().unwrap_or_default());
            subtitle.set_visible(subtitle_text.is_some());
            let symbol = data.symbol.get();
            icon.set_icon_name(symbol.map(symbol_name));
            icon.set_visible(symbol.is_some());
            expander.set_hide_expander(data.children.n_items() == 0);
            disclosure.set_visible(data.disclosure.get() && data.children.n_items() == 0);
            true
        }));

        row.tree_rows.borrow_mut().push(tree_row.downgrade());
        let weak_row = Rc::downgrade(&row);
        let weak_expander = expander.downgrade();
        tree_row.connect_expanded_notify(move |tree_row| {
            let (Some(row), Some(expander)) = (weak_row.upgrade(), weak_expander.upgrade()) else {
                return;
            };
            if row.suppress_expansion.get() || expander.list_row().as_ref() != Some(tree_row) {
                return;
            }
            let expanded = tree_row.is_expanded();
            if row.expanded.get() != expanded {
                row.events.emit_toggle(expanded);
            }
        });
        row.sync_expansion();
    });
    factory
}

fn flat_row_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_bind(|_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(object) = item.item() else {
            return;
        };
        let Some(row) = row_from_model_item(&object) else {
            return;
        };
        let content = gtk::Box::new(
            gtk::Orientation::Horizontal,
            spacing_pixels(Spacing::Compact),
        );
        let icon = gtk::Image::new();
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_hexpand(true);
        let subtitle = gtk::Label::new(None);
        subtitle.set_xalign(0.0);
        subtitle.add_css_class("caption");
        subtitle.add_css_class("dim-label");
        labels.append(&title);
        labels.append(&subtitle);
        let disclosure = gtk::Image::from_icon_name("go-next-symbolic");
        content.append(&icon);
        content.append(&labels);
        content.append(&disclosure);
        item.set_child(Some(&content));
        let weak_item = item.downgrade();
        let weak_icon = icon.downgrade();
        let weak_title = title.downgrade();
        let weak_subtitle = subtitle.downgrade();
        let weak_disclosure = disclosure.downgrade();
        row.add_binding(Box::new(move |data| {
            let (Some(item), Some(icon), Some(title), Some(subtitle), Some(disclosure)) = (
                weak_item.upgrade(),
                weak_icon.upgrade(),
                weak_title.upgrade(),
                weak_subtitle.upgrade(),
                weak_disclosure.upgrade(),
            ) else {
                return false;
            };
            item.set_accessible_label(&data.accessibility_label.borrow());
            item.set_selectable(true);
            item.set_activatable(true);
            title.set_label(&data.title.borrow());
            let subtitle_text = data.subtitle.borrow();
            subtitle.set_label(subtitle_text.as_deref().unwrap_or_default());
            subtitle.set_visible(subtitle_text.is_some());
            let symbol = data.symbol.get();
            icon.set_icon_name(symbol.map(symbol_name));
            icon.set_visible(symbol.is_some());
            disclosure.set_visible(data.disclosure.get());
            true
        }));
    });
    factory
}

fn table_cell_factory(column_index: usize, column_title: &str) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let column_title = column_title.to_owned();
    factory.connect_bind(move |_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(object) = item.item() else {
            return;
        };
        let tree_row = object.clone().downcast::<gtk::TreeListRow>().ok();
        let Some(row) = row_from_model_item(&object) else {
            return;
        };
        let content = gtk::Box::new(
            gtk::Orientation::Horizontal,
            spacing_pixels(Spacing::Compact),
        );
        let icon = gtk::Image::new();
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_hexpand(column_index == 0);
        let disclosure = gtk::Image::from_icon_name("go-next-symbolic");
        if column_index == 0 {
            content.append(&icon);
        }
        content.append(&label);
        if column_index == 0 {
            content.append(&disclosure);
        }
        let expander = if column_index == 0 {
            tree_row.as_ref().map(|tree_row| {
                let expander = gtk::TreeExpander::new();
                expander.set_list_row(Some(tree_row));
                expander.set_child(Some(&content));
                expander
            })
        } else {
            None
        };
        if let Some(expander) = expander.as_ref() {
            item.set_child(Some(expander));
        } else {
            item.set_child(Some(&content));
        }
        let weak_item = item.downgrade();
        let weak_icon = icon.downgrade();
        let weak_label = label.downgrade();
        let weak_disclosure = disclosure.downgrade();
        let weak_expander_for_binding = expander.as_ref().map(|expander| expander.downgrade());
        let column_title = column_title.clone();
        row.add_binding(Box::new(move |data| {
            let (Some(item), Some(icon), Some(label), Some(disclosure)) = (
                weak_item.upgrade(),
                weak_icon.upgrade(),
                weak_label.upgrade(),
                weak_disclosure.upgrade(),
            ) else {
                return false;
            };
            let cell_value = data.value_for_column(column_index);
            label.update_property(&[gtk::accessible::Property::Description(
                &table_cell_accessible_description(&column_title),
            )]);
            item.set_selectable(true);
            item.set_activatable(true);
            label.set_label(&cell_value);
            let symbol = data.symbol.get();
            icon.set_icon_name(symbol.map(symbol_name));
            icon.set_visible(column_index == 0 && symbol.is_some());
            disclosure.set_visible(
                column_index == 0 && data.disclosure.get() && data.children.n_items() == 0,
            );
            if let Some(expander) = weak_expander_for_binding
                .as_ref()
                .and_then(|weak| weak.upgrade())
            {
                expander.set_hide_expander(data.children.n_items() == 0);
            }
            true
        }));
        if let (Some(tree_row), Some(expander)) = (tree_row, expander) {
            row.tree_rows.borrow_mut().push(tree_row.downgrade());
            let weak_row = Rc::downgrade(&row);
            let weak_expander = expander.downgrade();
            tree_row.connect_expanded_notify(move |tree_row| {
                let (Some(row), Some(expander)) = (weak_row.upgrade(), weak_expander.upgrade())
                else {
                    return;
                };
                if row.suppress_expansion.get() || expander.list_row().as_ref() != Some(tree_row) {
                    return;
                }
                let expanded = tree_row.is_expanded();
                if row.expanded.get() != expanded {
                    row.events.emit_toggle(expanded);
                }
            });
            row.sync_expansion();
        }
    });
    factory
}

fn table_cell_accessible_description(column_title: &str) -> String {
    format!("{column_title} column")
}

impl fmt::Debug for GtkHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GtkHandle")
            .field("type", &self.widget().type_().name())
            .field("kind", &self.0.host_kind)
            .field("presentation_count", &self.0.presentations.borrow().len())
            .finish()
    }
}

/// GTK adapter diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GtkError(String);

impl fmt::Display for GtkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for GtkError {}

/// Reconciler adapter for GTK widgets.
#[derive(Debug)]
pub struct GtkBackend {
    root: GtkHandle,
    layout_context: LayoutContext,
}

impl GtkBackend {
    fn new(root: &gtk::Box, layout_context: LayoutContext) -> Self {
        Self {
            root: GtkHandle::new(root.clone(), HostKind::Root, None, Vec::new()),
            layout_context,
        }
    }
}

impl NativeBackend for GtkBackend {
    type Handle = GtkHandle;
    type Error = GtkError;

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
        create_element(element, events, self.layout_context)
    }

    fn apply(&mut self, handle: &Self::Handle, patch: &PropertyPatch) -> Result<(), Self::Error> {
        apply_patch(handle, patch, self.layout_context)
    }

    fn insert_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        insert_child(parent, child, index)
    }

    fn remove_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        index: usize,
    ) -> Result<(), Self::Error> {
        remove_child(parent, child, index)
    }

    fn move_child(
        &mut self,
        parent: &Self::Handle,
        child: &Self::Handle,
        from: usize,
        to: usize,
    ) -> Result<(), Self::Error> {
        move_child(parent, child, from, to)
    }
}

fn validate_element(element: &Element) -> Result<(), GtkError> {
    match element.props() {
        Props::Button {
            label,
            material,
            accessibility_label,
            ..
        } => {
            require_text("button label", label)?;
            require_accessible_name("button", accessibility_label)?;
            if *material == ButtonMaterial::Glass {
                return Err(GtkError(
                    "GTK does not provide the requested glass button material".to_owned(),
                ));
            }
        }
        Props::Input {
            accessibility_label,
            ..
        } => require_accessible_name("input", accessibility_label)?,
        Props::Toggle {
            label,
            accessibility_label,
            ..
        } => {
            require_text("toggle label", label)?;
            require_accessible_name("toggle", accessibility_label)?;
        }
        Props::Progress {
            fraction,
            accessibility_label,
        } => {
            if !fraction.is_finite() || !(0.0..=1.0).contains(fraction) {
                return Err(GtkError(format!(
                    "progress fraction must be finite and within 0..=1, received {fraction}"
                )));
            }
            require_accessible_name("progress", accessibility_label)?;
        }
        Props::List {
            accessibility_label,
            columns,
            ..
        } => {
            require_accessible_name("list", accessibility_label)?;
            for column in columns {
                require_text("table column title", &column.title)?;
            }
        }
        Props::ListRow {
            accessibility_label,
            ..
        } => require_accessible_name("list row", accessibility_label)?,
        Props::Status { title, message, .. } => {
            require_text("status title", title)?;
            require_text("status message", message)?;
        }
        Props::Label { .. }
        | Props::Separator { .. }
        | Props::Spacer { .. }
        | Props::Stack { .. }
        | Props::Scroll { .. }
        | Props::Split { .. }
        | Props::Workspace { .. } => {}
    }
    Ok(())
}

fn require_accessible_name(kind: &str, value: &str) -> Result<(), GtkError> {
    require_text(&format!("{kind} accessibility label"), value)
}

fn require_text(field: &str, value: &str) -> Result<(), GtkError> {
    if value.trim().is_empty() {
        Err(GtkError(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

fn create_element(
    element: &Element,
    events: EventBindings,
    layout_context: LayoutContext,
) -> Result<GtkHandle, GtkError> {
    match element.props() {
        Props::Label {
            text,
            role,
            selectable,
        } => {
            let label = gtk::Label::new(Some(text));
            label.set_xalign(0.0);
            label.set_selectable(*selectable);
            label.set_wrap(true);
            label.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
            configure_label(&label, *role);
            Ok(GtkHandle::new(
                label,
                HostKind::Element(ElementKind::Label),
                None,
                Vec::new(),
            ))
        }
        Props::Button {
            label,
            role,
            size,
            material,
            enabled,
            tooltip,
            accessibility_label,
        } => {
            let button = gtk::Button::with_label(label);
            let action = events.clone();
            button.connect_clicked(move |_| action.emit_activate());
            configure_button(
                &button,
                *role,
                *size,
                *material,
                *enabled,
                tooltip.as_deref(),
                accessibility_label,
            );
            configure_button_context(&button, *size, layout_context);
            Ok(GtkHandle::new(
                button,
                HostKind::Element(ElementKind::Button),
                None,
                Vec::new(),
            ))
        }
        Props::Input {
            value,
            placeholder,
            kind,
            enabled,
            accessibility_label,
        } => create_input(
            value,
            placeholder,
            *kind,
            *enabled,
            accessibility_label,
            events,
        ),
        Props::Toggle {
            label,
            value,
            size: _,
            enabled,
            accessibility_label,
        } => {
            let row = adw::ActionRow::builder().title(label).build();
            let suppress_events = Rc::new(Cell::new(false));
            let toggle = gtk::Switch::builder()
                .active(*value)
                .sensitive(*enabled)
                .valign(gtk::Align::Center)
                .build();
            let action = events.clone();
            let signal_guard = suppress_events.clone();
            toggle.connect_active_notify(move |toggle| {
                if !signal_guard.get() {
                    action.emit_toggle(toggle.is_active());
                }
            });
            toggle.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            row.add_suffix(&toggle);
            row.set_activatable_widget(Some(&toggle));
            Ok(GtkHandle::with_suppression(
                row,
                HostKind::Element(ElementKind::Toggle),
                vec![toggle.upcast()],
                suppress_events,
            ))
        }
        Props::Progress {
            fraction,
            accessibility_label,
        } => {
            let progress = gtk::ProgressBar::new();
            progress.set_fraction(*fraction);
            progress.set_show_text(true);
            progress.set_text(Some(&progress_percentage_text(*fraction)));
            progress.add_css_class("caption-heading");
            progress.set_hexpand(false);
            progress.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            let clamp = adw::Clamp::new();
            clamp.set_hexpand(false);
            clamp.set_maximum_size(240);
            clamp.set_tightening_threshold(240);
            clamp.set_margin_start(content_spacing_pixels(layout_context, Spacing::Content));
            clamp.set_margin_end(content_spacing_pixels(layout_context, Spacing::Content));
            clamp.set_child(Some(&progress));
            Ok(GtkHandle::new(
                clamp,
                HostKind::Element(ElementKind::Progress),
                None,
                vec![progress.upcast()],
            ))
        }
        Props::Separator { axis } => Ok(GtkHandle::new(
            gtk::Separator::new(orientation(*axis)),
            HostKind::Element(ElementKind::Separator),
            None,
            Vec::new(),
        )),
        Props::Spacer {
            horizontal,
            vertical,
        } => {
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_hexpand(*horizontal);
            spacer.set_vexpand(*vertical);
            Ok(GtkHandle::new(
                spacer,
                HostKind::Element(ElementKind::Spacer),
                None,
                Vec::new(),
            ))
        }
        Props::Stack {
            axis,
            spacing,
            padding,
            align,
            justify,
        } => {
            let container = gtk::Box::new(
                orientation(*axis),
                content_spacing_pixels(layout_context, *spacing),
            );
            configure_stack(
                &container,
                *axis,
                *padding,
                *align,
                *justify,
                layout_context,
            );
            Ok(GtkHandle::new(
                container,
                HostKind::Element(ElementKind::Stack),
                None,
                Vec::new(),
            ))
        }
        Props::Scroll { axis } => {
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_hexpand(true);
            scroll.set_vexpand(true);
            configure_scroll(&scroll, *axis);
            Ok(GtkHandle::new(
                scroll,
                HostKind::Element(ElementKind::Scroll),
                None,
                Vec::new(),
            ))
        }
        Props::Split { role, collapsible } => {
            let split = adw::OverlaySplitView::new();
            split.set_hexpand(true);
            split.set_vexpand(true);
            split.set_collapsed(false);
            split.set_enable_show_gesture(*collapsible);
            split.set_enable_hide_gesture(*collapsible);
            if *role == SplitRole::Utility {
                split.set_sidebar_position(gtk::PackType::End);
                split.set_sidebar_width_unit(adw::LengthUnit::Sp);
                split.set_min_sidebar_width(UTILITY_PANE_MIN_WIDTH_SP);
            }
            let handle = GtkHandle::new(
                split,
                HostKind::Element(ElementKind::Split),
                Some(*role),
                Vec::new(),
            );
            handle.0.split_collapsible.set(*collapsible);
            Ok(handle)
        }
        Props::Workspace {
            sidebar_collapsible,
            inspector_collapsible,
        } => {
            let navigation = adw::OverlaySplitView::new();
            navigation.set_hexpand(true);
            navigation.set_vexpand(true);
            navigation.set_enable_show_gesture(*sidebar_collapsible);
            navigation.set_enable_hide_gesture(*sidebar_collapsible);
            navigation.set_collapsed(false);
            let inspector = adw::OverlaySplitView::new();
            inspector.set_hexpand(true);
            inspector.set_vexpand(true);
            inspector.set_sidebar_position(gtk::PackType::End);
            inspector.set_sidebar_width_unit(adw::LengthUnit::Sp);
            inspector.set_min_sidebar_width(UTILITY_PANE_MIN_WIDTH_SP);
            inspector.set_enable_show_gesture(*inspector_collapsible);
            inspector.set_enable_hide_gesture(*inspector_collapsible);
            inspector.set_collapsed(false);
            navigation.set_content(Some(&inspector));
            let data = WorkspaceData {
                navigation: navigation.clone(),
                inspector,
                sidebar_collapsible: Rc::new(Cell::new(*sidebar_collapsible)),
                inspector_collapsible: Rc::new(Cell::new(*inspector_collapsible)),
            };
            Ok(GtkHandle::workspace(navigation, data))
        }
        Props::List {
            accessibility_label,
            style,
            columns,
        } => {
            let data = ListData::new(accessibility_label, *style, columns, events);
            Ok(GtkHandle::list(data.scroll.clone(), data))
        }
        Props::ListRow {
            title,
            subtitle,
            cells,
            role,
            expanded,
            symbol,
            selected,
            disclosure,
            accessibility_label,
        } => {
            let data = RowData::new(
                title,
                subtitle.as_deref(),
                cells,
                *role,
                *expanded,
                *symbol,
                *selected,
                *disclosure,
                accessibility_label,
                events,
            );
            let object = glib::BoxedAnyObject::new(data.clone());
            Ok(GtkHandle::row(data, object))
        }
        Props::Status {
            title,
            message,
            tone,
        } => {
            let page = adw::StatusPage::builder()
                .title(title)
                .description(message)
                .icon_name(status_icon(*tone))
                .build();
            page.add_css_class("compact");
            page.set_vexpand(false);
            Ok(GtkHandle::new(
                page,
                HostKind::Element(ElementKind::Status),
                None,
                Vec::new(),
            ))
        }
    }
}

fn create_input(
    value: &str,
    placeholder: &str,
    kind: InputKind,
    enabled: bool,
    accessibility_label: &str,
    events: EventBindings,
) -> Result<GtkHandle, GtkError> {
    let suppress_events = Rc::new(Cell::new(false));
    let widget: gtk::Widget = match kind {
        InputKind::Search => {
            let input = gtk::SearchEntry::new();
            input.set_text(value);
            input.set_placeholder_text(Some(placeholder));
            let action = events.clone();
            let signal_guard = suppress_events.clone();
            input.connect_search_changed(move |input| {
                if !signal_guard.get() {
                    action.emit_input(input.text().to_string());
                }
            });
            input.upcast()
        }
        InputKind::Text => {
            let input = gtk::Entry::new();
            input.set_text(value);
            input.set_placeholder_text(Some(placeholder));
            let action = events.clone();
            let signal_guard = suppress_events.clone();
            input.connect_changed(move |input| {
                if !signal_guard.get() {
                    action.emit_input(input.text().to_string());
                }
            });
            input.upcast()
        }
        InputKind::Secure => {
            let input = gtk::PasswordEntry::new();
            input.set_text(value);
            input.set_placeholder_text(Some(placeholder));
            let action = events.clone();
            let signal_guard = suppress_events.clone();
            input.connect_changed(move |input| {
                if !signal_guard.get() {
                    action.emit_input(input.text().to_string());
                }
            });
            input.upcast()
        }
    };
    widget.set_sensitive(enabled);
    widget.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
    Ok(GtkHandle::with_suppression(
        widget,
        HostKind::Element(ElementKind::Input),
        Vec::new(),
        suppress_events,
    ))
}

fn configure_label(label: &gtk::Label, role: TextRole) {
    for class in [
        "title-1",
        "heading",
        "body",
        "caption",
        "dim-label",
        "monospace",
    ] {
        label.remove_css_class(class);
    }
    match role {
        TextRole::Title => label.add_css_class("title-1"),
        TextRole::Heading => label.add_css_class("heading"),
        TextRole::Body => label.add_css_class("body"),
        // A secondary label still carries actionable application state in a
        // compact status row. Adwaita's standard body token keeps that state
        // legible at narrow sizes without introducing adapter-owned font or
        // color values.
        TextRole::Secondary => label.add_css_class("body"),
        TextRole::Monospace => label.add_css_class("monospace"),
    }
}

fn configure_button(
    button: &gtk::Button,
    role: ButtonRole,
    size: ControlSize,
    material: ButtonMaterial,
    enabled: bool,
    tooltip: Option<&str>,
    accessibility_label: &str,
) {
    for class in [
        "suggested-action",
        "destructive-action",
        "flat",
        "pill",
        "compact",
    ] {
        button.remove_css_class(class);
    }
    match role {
        ButtonRole::Standard => {}
        ButtonRole::Primary => button.add_css_class("suggested-action"),
        ButtonRole::Destructive => button.add_css_class("destructive-action"),
        ButtonRole::Toolbar => button.add_css_class("flat"),
    }
    match size {
        ControlSize::Mini | ControlSize::Small => button.add_css_class("compact"),
        ControlSize::Regular => {}
        ControlSize::Large => button.add_css_class("pill"),
        ControlSize::ExtraLarge => button.add_css_class("pill"),
    }
    if material == ButtonMaterial::Glass {
        button.add_css_class("flat");
    }
    button.set_sensitive(enabled);
    button.set_tooltip_text(tooltip);
    button.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
}

fn configure_button_context(
    button: &gtk::Button,
    size: ControlSize,
    layout_context: LayoutContext,
) {
    if size == ControlSize::Regular && layout_context == LayoutContext::AuxiliaryPanel {
        button.add_css_class("compact");
    }
}

fn configure_stack(
    container: &gtk::Box,
    axis: Axis,
    padding: Option<Spacing>,
    align: Align,
    justify: Justify,
    layout_context: LayoutContext,
) {
    container.set_orientation(orientation(axis));
    if let Some(padding) = padding {
        let (horizontal_inset, vertical_inset) = stack_insets(layout_context, padding);
        container.set_margin_start(horizontal_inset);
        container.set_margin_end(horizontal_inset);
        container.set_margin_top(vertical_inset);
        container.set_margin_bottom(vertical_inset);
    } else {
        container.set_margin_start(0);
        container.set_margin_end(0);
        container.set_margin_top(0);
        container.set_margin_bottom(0);
    }
    match axis {
        Axis::Horizontal => {
            container.set_valign(gtk_align(align));
            container.set_vexpand(align == Align::Stretch);
        }
        Axis::Vertical => {
            container.set_halign(gtk_align(align));
            container.set_hexpand(align == Align::Stretch);
        }
    }
    let main_align = match justify {
        Justify::Start => gtk::Align::Fill,
        Justify::Center => gtk::Align::Center,
        Justify::End => gtk::Align::End,
    };
    match axis {
        Axis::Horizontal => {
            container.set_halign(main_align);
            container.set_hexpand(justify != Justify::Start);
        }
        Axis::Vertical => {
            container.set_valign(main_align);
            container.set_vexpand(justify != Justify::Start);
        }
    }
}

fn configure_scroll(scroll: &gtk::ScrolledWindow, axis: Axis) {
    match axis {
        Axis::Horizontal => scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never),
        Axis::Vertical => scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic),
    }
}

fn apply_patch(
    handle: &GtkHandle,
    patch: &PropertyPatch,
    layout_context: LayoutContext,
) -> Result<(), GtkError> {
    match patch {
        PropertyPatch::Label {
            text,
            role,
            selectable,
        } => {
            let label = downcast::<gtk::Label>(handle)?;
            label.set_label(text);
            label.set_selectable(*selectable);
            configure_label(&label, *role);
        }
        PropertyPatch::Button {
            label,
            role,
            size,
            material,
            enabled,
            tooltip,
            accessibility_label,
        } => {
            let button = downcast::<gtk::Button>(handle)?;
            button.set_label(label);
            configure_button(
                &button,
                *role,
                *size,
                *material,
                *enabled,
                tooltip.as_deref(),
                accessibility_label,
            );
            configure_button_context(&button, *size, layout_context);
        }
        PropertyPatch::Input {
            value,
            placeholder,
            enabled,
            accessibility_label,
            ..
        } => {
            handle.0.suppress_events.set(true);
            handle.widget().set_sensitive(*enabled);
            handle
                .widget()
                .update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            if let Ok(input) = handle.widget().clone().downcast::<gtk::SearchEntry>() {
                input.set_text(value);
                input.set_placeholder_text(Some(placeholder));
            } else if let Ok(input) = handle.widget().clone().downcast::<gtk::Entry>() {
                input.set_text(value);
                input.set_placeholder_text(Some(placeholder));
            } else if let Ok(input) = handle.widget().clone().downcast::<gtk::PasswordEntry>() {
                input.set_text(value);
                input.set_placeholder_text(Some(placeholder));
            }
            handle.0.suppress_events.set(false);
        }
        PropertyPatch::Toggle {
            label,
            value,
            size: _,
            enabled,
            accessibility_label,
        } => {
            let row = downcast::<adw::ActionRow>(handle)?;
            row.set_title(label);
            let toggle = handle
                .0
                .auxiliaries
                .first()
                .and_then(|widget| widget.clone().downcast::<gtk::Switch>().ok())
                .ok_or_else(|| GtkError("toggle has no native switch".to_owned()))?;
            handle.0.suppress_events.set(true);
            toggle.set_active(*value);
            toggle.set_sensitive(*enabled);
            toggle.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            handle.0.suppress_events.set(false);
        }
        PropertyPatch::Progress {
            fraction,
            accessibility_label,
        } => {
            let progress = native_progress(handle)?;
            progress.set_fraction(*fraction);
            progress.set_text(Some(&progress_percentage_text(*fraction)));
            progress.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
        }
        PropertyPatch::Separator { axis } => {
            downcast::<gtk::Separator>(handle)?.set_orientation(orientation(*axis));
        }
        PropertyPatch::Stack {
            axis,
            spacing,
            padding,
            align,
            justify,
        } => {
            let container = downcast::<gtk::Box>(handle)?;
            container.set_spacing(content_spacing_pixels(layout_context, *spacing));
            configure_stack(
                &container,
                *axis,
                *padding,
                *align,
                *justify,
                layout_context,
            );
        }
        PropertyPatch::Spacer {
            horizontal,
            vertical,
        } => {
            handle.widget().set_hexpand(*horizontal);
            handle.widget().set_vexpand(*vertical);
        }
        PropertyPatch::Scroll { axis } => configure_scroll(&downcast(handle)?, *axis),
        PropertyPatch::Split { collapsible, .. } => {
            let split = downcast::<adw::OverlaySplitView>(handle)?;
            handle.0.split_collapsible.set(*collapsible);
            if !*collapsible {
                split.set_collapsed(false);
            }
            split.set_enable_show_gesture(*collapsible);
            split.set_enable_hide_gesture(*collapsible);
        }
        PropertyPatch::Workspace {
            sidebar_collapsible,
            inspector_collapsible,
        } => {
            let workspace = handle
                .0
                .workspace
                .as_ref()
                .ok_or_else(|| GtkError("workspace has no native split views".to_owned()))?;
            workspace.sidebar_collapsible.set(*sidebar_collapsible);
            workspace.inspector_collapsible.set(*inspector_collapsible);
            workspace
                .navigation
                .set_enable_show_gesture(*sidebar_collapsible);
            workspace
                .navigation
                .set_enable_hide_gesture(*sidebar_collapsible);
            workspace
                .inspector
                .set_enable_show_gesture(*inspector_collapsible);
            workspace
                .inspector
                .set_enable_hide_gesture(*inspector_collapsible);
            if !*sidebar_collapsible {
                workspace.navigation.set_collapsed(false);
            }
            if !*inspector_collapsible {
                workspace.inspector.set_collapsed(false);
            }
        }
        PropertyPatch::List {
            accessibility_label,
            style,
            columns,
        } => {
            let list = handle
                .0
                .list
                .as_ref()
                .ok_or_else(|| GtkError("list has no native model".to_owned()))?;
            list.update(accessibility_label, *style, columns);
        }
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
        } => {
            let row = handle
                .0
                .row
                .as_ref()
                .ok_or_else(|| GtkError("list row has no native model item".to_owned()))?;
            row.update(
                title,
                subtitle.as_deref(),
                cells,
                *role,
                *expanded,
                *symbol,
                *selected,
                *disclosure,
                accessibility_label,
            );
        }
        PropertyPatch::Status {
            title,
            message,
            tone,
        } => {
            let page = downcast::<adw::StatusPage>(handle)?;
            page.set_title(title);
            page.set_description(Some(message));
            page.set_icon_name(Some(status_icon(*tone)));
        }
    }
    Ok(())
}

fn insert_child(parent: &GtkHandle, child: &GtkHandle, index: usize) -> Result<(), GtkError> {
    let mut presentations = parent.0.presentations.borrow_mut();
    if index > presentations.len() {
        return Err(GtkError(format!(
            "cannot insert GTK child at {index}; count is {}",
            presentations.len()
        )));
    }
    let presentation = Presentation {
        source: child.widget().clone(),
        view: child.widget().clone(),
    };
    match parent.0.host_kind {
        HostKind::Root => {
            presentation.view.set_hexpand(true);
            presentation.view.set_vexpand(true);
            let container = downcast::<gtk::Box>(parent)?;
            let sibling = index
                .checked_sub(1)
                .and_then(|previous| presentations.get(previous))
                .map(|item| &item.view);
            container.insert_child_after(&presentation.view, sibling);
        }
        HostKind::Element(ElementKind::Stack) => {
            let container = downcast::<gtk::Box>(parent)?;
            let sibling = index
                .checked_sub(1)
                .and_then(|previous| presentations.get(previous))
                .map(|item| &item.view);
            container.insert_child_after(&presentation.view, sibling);
        }
        HostKind::Element(ElementKind::List) => {
            let list = parent
                .0
                .list
                .as_ref()
                .ok_or_else(|| GtkError("list has no native model".to_owned()))?;
            let object = child
                .0
                .row_object
                .as_ref()
                .ok_or_else(|| GtkError("list accepts only native row items".to_owned()))?;
            list.store
                .insert(u32::try_from(index).unwrap_or(u32::MAX), object);
            if let Some(row) = child.0.row.as_ref() {
                row.attach_owner(list);
            }
            list.sync_selection();
        }
        HostKind::Element(ElementKind::ListRow) => {
            let row = parent
                .0
                .row
                .as_ref()
                .ok_or_else(|| GtkError("list row has no native model item".to_owned()))?;
            let object = child.0.row_object.as_ref().ok_or_else(|| {
                GtkError("list hierarchy accepts only native row items".to_owned())
            })?;
            row.children
                .insert(u32::try_from(index).unwrap_or(u32::MAX), object);
            if let Some(child_row) = child.0.row.as_ref() {
                for owner in row
                    .list_owners
                    .borrow()
                    .iter()
                    .filter_map(std::rc::Weak::upgrade)
                {
                    child_row.attach_owner(&owner);
                }
            }
            row.refresh();
        }
        HostKind::Element(ElementKind::Scroll) => {
            if index != 0 || !presentations.is_empty() {
                return Err(GtkError("scroll view accepts exactly one child".to_owned()));
            }
            downcast::<gtk::ScrolledWindow>(parent)?.set_child(Some(&presentation.view));
        }
        HostKind::Element(ElementKind::Split) => {
            let split = downcast::<adw::OverlaySplitView>(parent)?;
            presentation.view.set_hexpand(true);
            presentation.view.set_vexpand(true);
            match (parent.0.split_role, index) {
                (Some(SplitRole::Navigation), 0) | (Some(SplitRole::Utility), 1) => {
                    split.set_sidebar(Some(&presentation.view));
                }
                (_, 0 | 1) => split.set_content(Some(&presentation.view)),
                _ => {
                    return Err(GtkError(
                        "split view accepts exactly two children".to_owned(),
                    ));
                }
            }
        }
        HostKind::Element(ElementKind::Workspace) => {
            let workspace = parent
                .0
                .workspace
                .as_ref()
                .ok_or_else(|| GtkError("workspace has no native split views".to_owned()))?;
            presentation.view.set_hexpand(true);
            presentation.view.set_vexpand(true);
            match index {
                0 => workspace.navigation.set_sidebar(Some(&presentation.view)),
                1 => workspace.inspector.set_content(Some(&presentation.view)),
                2 => workspace.inspector.set_sidebar(Some(&presentation.view)),
                _ => {
                    return Err(GtkError(
                        "workspace accepts exactly three children".to_owned(),
                    ));
                }
            }
        }
        HostKind::Element(kind) => {
            return Err(GtkError(format!("{kind:?} cannot contain children")));
        }
    }
    presentations.insert(index, presentation);
    Ok(())
}

fn remove_child(parent: &GtkHandle, child: &GtkHandle, index: usize) -> Result<(), GtkError> {
    let mut presentations = parent.0.presentations.borrow_mut();
    let Some(presentation) = presentations.get(index) else {
        return Err(GtkError(format!("no GTK child at index {index}")));
    };
    if presentation.source != *child.widget() {
        return Err(GtkError(format!("GTK child mismatch at index {index}")));
    }
    match parent.0.host_kind {
        HostKind::Root | HostKind::Element(ElementKind::Stack) => {
            downcast::<gtk::Box>(parent)?.remove(&presentation.view);
        }
        HostKind::Element(ElementKind::List) => {
            let list = parent
                .0
                .list
                .as_ref()
                .ok_or_else(|| GtkError("list has no native model".to_owned()))?;
            list.store.remove(u32::try_from(index).unwrap_or(u32::MAX));
            list.sync_selection();
        }
        HostKind::Element(ElementKind::ListRow) => {
            let row = parent
                .0
                .row
                .as_ref()
                .ok_or_else(|| GtkError("list row has no native model item".to_owned()))?;
            row.children
                .remove(u32::try_from(index).unwrap_or(u32::MAX));
            row.refresh();
        }
        HostKind::Element(ElementKind::Scroll) => {
            downcast::<gtk::ScrolledWindow>(parent)?.set_child(gtk::Widget::NONE);
        }
        HostKind::Element(ElementKind::Split) => {
            let split = downcast::<adw::OverlaySplitView>(parent)?;
            match (parent.0.split_role, index) {
                (Some(SplitRole::Navigation), 0) | (Some(SplitRole::Utility), 1) => {
                    split.set_sidebar(gtk::Widget::NONE);
                }
                _ => split.set_content(gtk::Widget::NONE),
            }
        }
        HostKind::Element(ElementKind::Workspace) => {
            let workspace = parent
                .0
                .workspace
                .as_ref()
                .ok_or_else(|| GtkError("workspace has no native split views".to_owned()))?;
            match index {
                0 => workspace.navigation.set_sidebar(gtk::Widget::NONE),
                1 => workspace.inspector.set_content(gtk::Widget::NONE),
                2 => workspace.inspector.set_sidebar(gtk::Widget::NONE),
                _ => {
                    return Err(GtkError(
                        "workspace has no child at the requested index".to_owned(),
                    ));
                }
            }
        }
        HostKind::Element(kind) => {
            return Err(GtkError(format!("{kind:?} cannot remove children")));
        }
    }
    presentations.remove(index);
    Ok(())
}

fn move_child(
    parent: &GtkHandle,
    child: &GtkHandle,
    from: usize,
    to: usize,
) -> Result<(), GtkError> {
    if from == to {
        return Ok(());
    }
    let mut presentations = parent.0.presentations.borrow_mut();
    if from >= presentations.len() || to >= presentations.len() {
        return Err(GtkError(format!(
            "cannot move GTK child from {from} to {to}; count is {}",
            presentations.len()
        )));
    }
    if presentations[from].source != *child.widget() {
        return Err(GtkError(format!("GTK child mismatch at index {from}")));
    }
    let moved = presentations.remove(from);
    presentations.insert(to, moved);
    match parent.0.host_kind {
        HostKind::Root | HostKind::Element(ElementKind::Stack) => {
            let container = downcast::<gtk::Box>(parent)?;
            let sibling = to
                .checked_sub(1)
                .and_then(|previous| presentations.get(previous))
                .map(|item| &item.view);
            container.reorder_child_after(&presentations[to].view, sibling);
        }
        HostKind::Element(ElementKind::List) => {
            let list = parent
                .0
                .list
                .as_ref()
                .ok_or_else(|| GtkError("list has no native model".to_owned()))?;
            move_model_item(&list.store, from, to)?;
            list.sync_selection();
        }
        HostKind::Element(ElementKind::ListRow) => {
            let row = parent
                .0
                .row
                .as_ref()
                .ok_or_else(|| GtkError("list row has no native model item".to_owned()))?;
            move_model_item(&row.children, from, to)?;
            row.refresh();
        }
        HostKind::Element(ElementKind::Split) => {
            let split = downcast::<adw::OverlaySplitView>(parent)?;
            let first = &presentations[0].view;
            let second = &presentations[1].view;
            match parent.0.split_role {
                Some(SplitRole::Navigation) => {
                    split.set_sidebar(Some(first));
                    split.set_content(Some(second));
                }
                Some(SplitRole::Utility) => {
                    split.set_content(Some(first));
                    split.set_sidebar(Some(second));
                }
                None => {}
            }
        }
        HostKind::Element(ElementKind::Workspace) => {
            let workspace = parent
                .0
                .workspace
                .as_ref()
                .ok_or_else(|| GtkError("workspace has no native split views".to_owned()))?;
            workspace
                .navigation
                .set_sidebar(Some(&presentations[0].view));
            workspace
                .inspector
                .set_content(Some(&presentations[1].view));
            workspace
                .inspector
                .set_sidebar(Some(&presentations[2].view));
        }
        kind => return Err(GtkError(format!("{kind:?} does not support child moves"))),
    }
    Ok(())
}

fn move_model_item(store: &gio::ListStore, from: usize, to: usize) -> Result<(), GtkError> {
    let from = u32::try_from(from).unwrap_or(u32::MAX);
    let to = u32::try_from(to).unwrap_or(u32::MAX);
    let item = store
        .item(from)
        .ok_or_else(|| GtkError(format!("native row model has no item at {from}")))?;
    store.remove(from);
    store.insert(to, &item);
    Ok(())
}

fn downcast<T>(handle: &GtkHandle) -> Result<T, GtkError>
where
    T: IsA<gtk::Widget> + glib::types::StaticType,
{
    handle.widget().clone().downcast::<T>().map_err(|widget| {
        GtkError(format!(
            "expected {}, found {}",
            T::static_type().name(),
            widget.type_().name()
        ))
    })
}

fn native_progress(handle: &GtkHandle) -> Result<gtk::ProgressBar, GtkError> {
    handle
        .0
        .auxiliaries
        .first()
        .and_then(|widget| widget.clone().downcast::<gtk::ProgressBar>().ok())
        .ok_or_else(|| GtkError("progress has no native progress bar".to_owned()))
}

fn progress_percentage_text(fraction: f64) -> String {
    format!("{:.0}%", fraction * 100.0)
}

fn build_toolbar(
    spec: &WindowSpec,
    renderer: &Renderer<GtkBackend>,
    narrow_layout: Rc<Cell<bool>>,
) -> (adw::ToolbarView, adw::HeaderBar, Vec<gtk::Stack>) {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let mut adaptive_items = Vec::new();
    if let Some(split) = pane_for(renderer.mounted(), SplitRole::Navigation) {
        let expanded = pane_toggle_button(
            "Sidebar",
            "sidebar-show-symbolic",
            spec.toolbar_display,
            "Show or hide the navigation sidebar",
            split.clone(),
            narrow_layout.clone(),
        );
        let compact = pane_toggle_button(
            "Navigation",
            "sidebar-show-symbolic",
            ToolbarDisplay::IconAndLabel,
            "Show or hide the navigation sidebar",
            split,
            narrow_layout.clone(),
        );
        let stack = gtk::Stack::new();
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        stack.add_named(&expanded, Some("expanded"));
        stack.add_named(&compact, Some("compact"));
        stack.set_visible_child_name("expanded");
        header.pack_start(&stack);
        adaptive_items.push(stack);
    }

    let center = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    center.add_css_class("linked");
    let mut has_center = false;
    for item in &spec.toolbar {
        let presentation = toolbar_widget(item, spec.toolbar_display);
        if let Some(stack) = presentation.adaptive_stack {
            adaptive_items.push(stack);
        }
        let widget = presentation.widget;
        match item.placement {
            ToolbarPlacement::Leading => header.pack_start(&widget),
            ToolbarPlacement::Center => {
                center.append(&widget);
                has_center = true;
            }
            ToolbarPlacement::Trailing => header.pack_end(&widget),
        }
    }
    if has_center {
        header.set_title_widget(Some(&center));
    }

    if let Some(split) = pane_for(renderer.mounted(), SplitRole::Utility) {
        let button = gtk::Button::new();
        button.set_child(Some(&toolbar_named_item_content(
            "Inspector",
            "sidebar-show-right-symbolic",
            spec.toolbar_display,
        )));
        button.set_tooltip_text(Some("Show or hide the inspector"));
        button.update_property(&[gtk::accessible::Property::Label("Inspector")]);
        let split = split.clone();
        button.connect_clicked(move |_| toggle_split_sidebar(&split, narrow_layout.get()));
        header.pack_end(&button);
    }
    toolbar.add_top_bar(&header);
    (toolbar, header, adaptive_items)
}

fn pane_toggle_button(
    label: &str,
    icon_name: &str,
    display: ToolbarDisplay,
    help: &str,
    split: adw::OverlaySplitView,
    narrow_layout: Rc<Cell<bool>>,
) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&toolbar_named_item_content(label, icon_name, display)));
    button.set_tooltip_text(Some(help));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button.connect_clicked(move |_| toggle_split_sidebar(&split, narrow_layout.get()));
    button
}

fn toggle_split_sidebar(split: &adw::OverlaySplitView, narrow_layout: bool) {
    if split.sidebar().is_none() {
        return;
    }
    if narrow_layout {
        split.set_show_sidebar(!split.shows_sidebar());
    } else if split.is_collapsed() {
        split.set_collapsed(false);
    } else {
        split.set_show_sidebar(false);
        split.set_collapsed(true);
    }
}

struct ToolbarPresentation {
    widget: gtk::Widget,
    adaptive_stack: Option<gtk::Stack>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolbarLayout {
    Expanded,
    Compact,
    Adaptive,
}

fn toolbar_layout(item: &ToolbarItem) -> ToolbarLayout {
    let has_compact_representation = matches!(
        &item.kind,
        ToolbarItemKind::ActionGroup { .. }
            | ToolbarItemKind::SelectionGroup { .. }
            | ToolbarItemKind::Search { .. }
    );
    if !has_compact_representation {
        return ToolbarLayout::Expanded;
    }
    match item.group_display {
        ToolbarGroupDisplay::Automatic => ToolbarLayout::Adaptive,
        ToolbarGroupDisplay::Expanded => ToolbarLayout::Expanded,
        ToolbarGroupDisplay::Collapsed => ToolbarLayout::Compact,
    }
}

fn toolbar_widget(item: &ToolbarItem, display: ToolbarDisplay) -> ToolbarPresentation {
    let expanded = toolbar_expanded_widget(item, display);
    match toolbar_layout(item) {
        ToolbarLayout::Expanded => ToolbarPresentation {
            widget: expanded,
            adaptive_stack: None,
        },
        ToolbarLayout::Compact => ToolbarPresentation {
            widget: toolbar_compact_widget(item, display)
                .expect("compact toolbar layout requires a compact representation"),
            adaptive_stack: None,
        },
        ToolbarLayout::Adaptive => {
            let compact = toolbar_compact_widget(item, display)
                .expect("adaptive toolbar layout requires a compact representation");
            let stack = gtk::Stack::new();
            stack.set_hhomogeneous(false);
            stack.set_vhomogeneous(false);
            stack.add_named(&expanded, Some("expanded"));
            stack.add_named(&compact, Some("compact"));
            stack.set_visible_child_name("expanded");
            ToolbarPresentation {
                widget: stack.clone().upcast(),
                adaptive_stack: Some(stack),
            }
        }
    }
}

fn toolbar_expanded_widget(item: &ToolbarItem, display: ToolbarDisplay) -> gtk::Widget {
    let widget: gtk::Widget = match &item.kind {
        ToolbarItemKind::Action {
            symbol,
            on_activate,
        } => toolbar_action_button(
            &item.label,
            &item.help,
            *symbol,
            display,
            item.enabled,
            on_activate.clone(),
        )
        .upcast(),
        ToolbarItemKind::ActionGroup { actions } => {
            let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            group.add_css_class("linked");
            group.update_property(&[gtk::accessible::Property::Label(&item.label)]);
            for action in actions {
                group.append(&toolbar_action_button(
                    &action.label,
                    &action.help,
                    action.symbol,
                    display,
                    item.enabled && action.enabled,
                    action.on_activate.clone(),
                ));
            }
            group.upcast()
        }
        ToolbarItemKind::SelectionGroup {
            choices,
            selected_id,
            on_select,
        } => {
            let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            group.add_css_class("linked");
            group.update_property(&[gtk::accessible::Property::Label(&item.label)]);
            let mut previous: Option<gtk::ToggleButton> = None;
            for choice in choices {
                let button = gtk::ToggleButton::new();
                button.set_child(Some(&toolbar_item_content(
                    &choice.label,
                    choice.symbol,
                    display,
                )));
                button.set_tooltip_text(Some(&choice.label));
                button.set_sensitive(item.enabled && choice.enabled);
                button.update_property(&[gtk::accessible::Property::Label(&choice.label)]);
                if let Some(previous) = previous.as_ref() {
                    button.set_group(Some(previous));
                }
                button.set_active(choice.id == *selected_id);
                let selected = choice.id.clone();
                let on_select = on_select.clone();
                button.connect_toggled(move |button| {
                    if button.is_active() {
                        on_select(selected.clone());
                    }
                });
                group.append(&button);
                previous = Some(button);
            }
            group.upcast()
        }
        ToolbarItemKind::Menu { symbol, entries } => {
            let button = gtk::MenuButton::new();
            button.set_child(Some(&toolbar_item_content(&item.label, *symbol, display)));
            button.set_tooltip_text(Some(&item.help));
            button.set_sensitive(item.enabled);
            button.update_property(&[gtk::accessible::Property::Label(&item.label)]);
            let prefix = native_action_name(&item.id);
            let actions = gio::SimpleActionGroup::new();
            let menu = gio::Menu::new();
            let mut section = gio::Menu::new();
            for entry in entries {
                match entry {
                    ToolbarMenuEntry::Action(action) => {
                        let action_name = native_action_name(&action.id);
                        let detailed_action = format!("{prefix}.{action_name}");
                        let menu_item =
                            gio::MenuItem::new(Some(&action.label), Some(&detailed_action));
                        let icon = gio::ThemedIcon::new(symbol_name(action.symbol));
                        menu_item.set_icon(&icon);
                        section.append_item(&menu_item);
                        let native_action = gio::SimpleAction::new(&action_name, None);
                        native_action.set_enabled(item.enabled && action.enabled);
                        let handler = action.on_activate.clone();
                        native_action.connect_activate(move |_, _| handler());
                        actions.add_action(&native_action);
                    }
                    ToolbarMenuEntry::Separator => {
                        if section.n_items() > 0 {
                            menu.append_section(None, &section);
                            section = gio::Menu::new();
                        }
                    }
                }
            }
            if section.n_items() > 0 {
                menu.append_section(None, &section);
            }
            button.insert_action_group(&prefix, Some(&actions));
            button.set_menu_model(Some(&menu));
            button.upcast()
        }
        ToolbarItemKind::Search {
            value,
            placeholder,
            accessibility_label,
            on_input,
        } => {
            let search = gtk::SearchEntry::new();
            search.set_text(value);
            search.set_placeholder_text(Some(placeholder));
            search.set_sensitive(item.enabled);
            search.set_tooltip_text(Some(&item.help));
            search.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            let on_input = on_input.clone();
            search.connect_search_changed(move |search| on_input(search.text().to_string()));
            search.upcast()
        }
    };
    widget
}

fn toolbar_compact_widget(item: &ToolbarItem, _display: ToolbarDisplay) -> Option<gtk::Widget> {
    match &item.kind {
        ToolbarItemKind::ActionGroup { actions }
            if action_group_uses_direct_compact_buttons(actions.len()) =>
        {
            Some(compact_action_group(item, actions).upcast())
        }
        ToolbarItemKind::ActionGroup { actions } => {
            Some(action_group_menu_button(item, actions).upcast())
        }
        ToolbarItemKind::SelectionGroup {
            choices,
            selected_id,
            on_select,
        } => Some(
            selection_group_menu_button(item, choices, selected_id, on_select.clone()).upcast(),
        ),
        ToolbarItemKind::Search {
            value,
            placeholder,
            accessibility_label,
            on_input,
        } => Some(
            compact_search_button(
                item,
                value,
                placeholder,
                accessibility_label,
                on_input.clone(),
            )
            .upcast(),
        ),
        ToolbarItemKind::Action { .. } | ToolbarItemKind::Menu { .. } => None,
    }
}

fn action_group_uses_direct_compact_buttons(action_count: usize) -> bool {
    (1..=2).contains(&action_count)
}

fn compact_action_group(item: &ToolbarItem, actions: &[ToolbarAction]) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    group.add_css_class("linked");
    group.update_property(&[gtk::accessible::Property::Label(&item.label)]);
    for action in actions {
        group.append(&toolbar_action_button(
            &action.label,
            &action.help,
            action.symbol,
            ToolbarDisplay::IconOnly,
            item.enabled && action.enabled,
            action.on_activate.clone(),
        ));
    }
    group
}

fn action_group_menu_button(item: &ToolbarItem, actions: &[ToolbarAction]) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_child(Some(&toolbar_named_item_content(
        &item.label,
        symbol_name(Symbol::More),
        ToolbarDisplay::IconOnly,
    )));
    button.set_tooltip_text(Some(&item.help));
    button.set_sensitive(item.enabled);
    button.update_property(&[gtk::accessible::Property::Label(&item.label)]);

    let prefix = native_action_name(&item.id);
    let action_group = gio::SimpleActionGroup::new();
    let menu = gio::Menu::new();
    for action in actions {
        let action_name = native_action_name(&action.id);
        let detailed_action = format!("{prefix}.{action_name}");
        let menu_item = gio::MenuItem::new(Some(&action.label), Some(&detailed_action));
        menu_item.set_icon(&gio::ThemedIcon::new(symbol_name(action.symbol)));
        menu.append_item(&menu_item);
        let native_action = gio::SimpleAction::new(&action_name, None);
        native_action.set_enabled(item.enabled && action.enabled);
        let handler = action.on_activate.clone();
        native_action.connect_activate(move |_, _| handler());
        action_group.add_action(&native_action);
    }
    button.insert_action_group(&prefix, Some(&action_group));
    button.set_menu_model(Some(&menu));
    button
}

fn selection_group_menu_button(
    item: &ToolbarItem,
    choices: &[rinka_core::ToolbarChoice],
    selected_id: &str,
    on_select: rinka_core::InputHandler,
) -> gtk::MenuButton {
    let selected_symbol = choices
        .iter()
        .find(|choice| choice.id == selected_id)
        .map_or(Symbol::More, |choice| choice.symbol);
    let button = gtk::MenuButton::new();
    button.set_child(Some(&toolbar_named_item_content(
        &item.label,
        symbol_name(selected_symbol),
        ToolbarDisplay::IconOnly,
    )));
    button.set_tooltip_text(Some(&item.help));
    button.set_sensitive(item.enabled);
    button.update_property(&[gtk::accessible::Property::Label(&item.label)]);

    let prefix = native_action_name(&item.id);
    let action_group = gio::SimpleActionGroup::new();
    let menu = gio::Menu::new();
    for choice in choices {
        let action_name = native_action_name(&choice.id);
        let detailed_action = format!("{prefix}.{action_name}");
        let menu_item = gio::MenuItem::new(Some(&choice.label), Some(&detailed_action));
        let icon_name = if choice.id == selected_id {
            "object-select-symbolic"
        } else {
            symbol_name(choice.symbol)
        };
        menu_item.set_icon(&gio::ThemedIcon::new(icon_name));
        menu.append_item(&menu_item);
        let native_action = gio::SimpleAction::new(&action_name, None);
        native_action.set_enabled(item.enabled && choice.enabled);
        let selected = choice.id.clone();
        let handler = on_select.clone();
        native_action.connect_activate(move |_, _| handler(selected.clone()));
        action_group.add_action(&native_action);
    }
    button.insert_action_group(&prefix, Some(&action_group));
    button.set_menu_model(Some(&menu));
    button
}

fn compact_search_button(
    item: &ToolbarItem,
    value: &str,
    placeholder: &str,
    accessibility_label: &str,
    on_input: rinka_core::InputHandler,
) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_child(Some(&toolbar_named_item_content(
        &item.label,
        symbol_name(Symbol::Search),
        ToolbarDisplay::IconOnly,
    )));
    button.set_tooltip_text(Some(&item.help));
    button.set_sensitive(item.enabled);
    button.update_property(&[gtk::accessible::Property::Label(&item.label)]);

    let search = gtk::SearchEntry::new();
    search.set_text(value);
    search.set_placeholder_text(Some(placeholder));
    search.set_width_chars(24);
    search.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
    search.connect_search_changed(move |search| on_input(search.text().to_string()));
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let inset = spacing_pixels(Spacing::Related);
    content.set_margin_start(inset);
    content.set_margin_end(inset);
    content.set_margin_top(inset);
    content.set_margin_bottom(inset);
    content.append(&search);
    let popover = gtk::Popover::new();
    popover.set_child(Some(&content));
    button.set_popover(Some(&popover));
    button
}

fn toolbar_action_button(
    label: &str,
    help: &str,
    symbol: Symbol,
    display: ToolbarDisplay,
    enabled: bool,
    action: rinka_core::ActivateHandler,
) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&toolbar_item_content(label, symbol, display)));
    button.set_tooltip_text(Some(help));
    button.set_sensitive(enabled);
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button.connect_clicked(move |_| action());
    button
}

fn toolbar_item_content(label: &str, symbol: Symbol, display: ToolbarDisplay) -> gtk::Widget {
    toolbar_named_item_content(label, symbol_name(symbol), display)
}

fn toolbar_named_item_content(
    label: &str,
    icon_name: &str,
    display: ToolbarDisplay,
) -> gtk::Widget {
    match display {
        ToolbarDisplay::Automatic | ToolbarDisplay::IconOnly => {
            gtk::Image::from_icon_name(icon_name).upcast()
        }
        ToolbarDisplay::IconAndLabel => {
            let content = gtk::Box::new(
                gtk::Orientation::Horizontal,
                spacing_pixels(Spacing::Compact),
            );
            content.append(&gtk::Image::from_icon_name(icon_name));
            content.append(&gtk::Label::new(Some(label)));
            content.upcast()
        }
        ToolbarDisplay::LabelOnly => gtk::Label::new(Some(label)).upcast(),
    }
}

fn native_action_name(identifier: &str) -> String {
    identifier
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn pane_for(
    mounted: Option<&MountedNode<GtkHandle>>,
    role: SplitRole,
) -> Option<adw::OverlaySplitView> {
    let node = mounted?;
    if let Some(workspace) = node.handle().0.workspace.as_ref() {
        return Some(match role {
            SplitRole::Navigation => workspace.navigation.clone(),
            SplitRole::Utility => workspace.inspector.clone(),
        });
    }
    if node.handle().0.split_role == Some(role)
        && let Ok(split) = node
            .handle()
            .widget()
            .clone()
            .downcast::<adw::OverlaySplitView>()
    {
        return Some(split);
    }
    node.children()
        .iter()
        .find_map(|child| pane_for(Some(child), role))
}

fn collect_adaptive_splits(
    mounted: Option<&MountedNode<GtkHandle>>,
    output: &mut Vec<(adw::OverlaySplitView, bool)>,
) {
    let Some(node) = mounted else {
        return;
    };
    if let Some(workspace) = node.handle().0.workspace.as_ref() {
        output.push((
            workspace.navigation.clone(),
            workspace.sidebar_collapsible.get(),
        ));
        output.push((
            workspace.inspector.clone(),
            workspace.inspector_collapsible.get(),
        ));
    } else if node.handle().0.split_role.is_some()
        && let Ok(split) = node
            .handle()
            .widget()
            .clone()
            .downcast::<adw::OverlaySplitView>()
    {
        output.push((split, node.handle().0.split_collapsible.get()));
    }
    for child in node.children() {
        collect_adaptive_splits(Some(child), output);
    }
}

struct BuiltWindow {
    window: gtk::Window,
    _runtime: WindowRuntime<GtkBackend>,
}

fn build_window(
    app: &adw::Application,
    spec: &WindowSpec,
    main_window: Option<&gtk::Window>,
) -> Result<BuiltWindow, GtkError> {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let layout_context = match spec.kind {
        WindowKind::Panel(_) => LayoutContext::AuxiliaryPanel,
        WindowKind::Main | WindowKind::Preferences => LayoutContext::Standard,
    };
    let runtime = WindowRuntime::mount(
        Renderer::new(GtkBackend::new(&root, layout_context)),
        spec.content.clone(),
    )
    .map_err(|error| GtkError(error.to_string()))?;
    let narrow_layout = Rc::new(Cell::new(false));
    let (toolbar, header, adaptive_toolbar_items) =
        runtime.with_renderer(|renderer| build_toolbar(spec, renderer, narrow_layout.clone()));
    toolbar.set_content(Some(&root));

    let initial_content_width = spec.initial_size.width.round() as i32;
    let initial_content_height = spec.initial_size.height.round() as i32;
    let minimum_content_width = spec.minimum_size.width.round() as i32;
    let minimum_content_height = spec.minimum_size.height.round() as i32;
    let (_, header_height, _, _) =
        header.measure(gtk::Orientation::Vertical, initial_content_width);
    let (_, minimum_header_height, _, _) =
        header.measure(gtk::Orientation::Vertical, minimum_content_width);
    let initial_window_height = initial_content_height.saturating_add(header_height);
    let minimum_window_height = minimum_content_height.saturating_add(minimum_header_height);

    let window: gtk::Window = match spec.kind {
        WindowKind::Main => adw::ApplicationWindow::builder()
            .application(app)
            .title(&spec.title)
            .default_width(initial_content_width)
            .default_height(initial_window_height)
            .content(&toolbar)
            .build()
            .upcast(),
        WindowKind::Preferences | WindowKind::Panel(_) => adw::Window::builder()
            .application(app)
            .title(&spec.title)
            .default_width(initial_content_width)
            .default_height(initial_window_height)
            .content(&toolbar)
            .build()
            .upcast(),
    };
    // WindowSpec sizes describe the application content below native chrome.
    // Keep the window minimum limited to that content plus the measured
    // header so users can freely resize to any larger extent.
    window.set_size_request(minimum_content_width, minimum_window_height);
    install_initial_content_extent(
        &window,
        &root,
        initial_content_width,
        initial_content_height,
    );
    schedule_layout_probe(
        &window,
        &toolbar,
        &header,
        &root,
        initial_content_width,
        initial_content_height,
    );
    if let WindowKind::Panel(behavior) = spec.kind {
        configure_panel(&window, behavior, main_window);
    }
    install_adaptive_breakpoint(
        &window,
        spec.minimum_size.width,
        &runtime,
        &adaptive_toolbar_items,
        narrow_layout,
    );
    Ok(BuiltWindow {
        window,
        _runtime: runtime,
    })
}

fn install_initial_content_extent(
    window: &gtk::Window,
    root: &gtk::Box,
    expected_content_width: i32,
    expected_content_height: i32,
) {
    // X11 and Wayland compositors can reserve different decoration extents
    // around the first GtkWindow allocation. Correct the first presentation
    // once from the live root allocation; later user resizes are never
    // observed or rewritten by this path.
    let window = window.clone();
    let root = root.clone();
    glib::idle_add_local_once(move || {
        let width_delta = expected_content_width.saturating_sub(root.width());
        let height_delta = expected_content_height.saturating_sub(root.height());
        if width_delta <= 0 && height_delta <= 0 {
            return;
        }
        window.set_default_size(
            window.default_width().saturating_add(width_delta.max(0)),
            window.default_height().saturating_add(height_delta.max(0)),
        );
    });
}

fn install_adaptive_breakpoint(
    window: &gtk::Window,
    minimum_width: f64,
    runtime: &WindowRuntime<GtkBackend>,
    adaptive_toolbar_items: &[gtk::Stack],
    narrow_layout: Rc<Cell<bool>>,
) {
    let mut splits = Vec::new();
    runtime.with_renderer(|renderer| collect_adaptive_splits(renderer.mounted(), &mut splits));
    let splits = splits
        .into_iter()
        .filter_map(|(split, collapsible)| collapsible.then_some(split))
        .collect::<Vec<_>>();
    if splits.is_empty() && adaptive_toolbar_items.is_empty() {
        return;
    }
    let condition = adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        minimum_width,
        adw::LengthUnit::Px,
    );
    let breakpoint = adw::Breakpoint::new(condition);
    let applying = narrow_layout.clone();
    breakpoint.connect_apply(move |_| applying.set(true));
    breakpoint.connect_unapply(move |_| narrow_layout.set(false));
    let collapsed = true.to_value();
    for split in &splits {
        breakpoint.add_setter(split, "collapsed", Some(&collapsed));
    }
    let compact = "compact".to_value();
    for item in adaptive_toolbar_items {
        breakpoint.add_setter(item, "visible-child-name", Some(&compact));
    }
    if let Ok(window) = window.clone().downcast::<adw::ApplicationWindow>() {
        window.add_breakpoint(breakpoint);
    } else if let Ok(window) = window.clone().downcast::<adw::Window>() {
        window.add_breakpoint(breakpoint);
    }
}

fn configure_panel(window: &gtk::Window, behavior: PanelBehavior, main: Option<&gtk::Window>) {
    if behavior.floating
        && let Some(main) = main
    {
        window.set_transient_for(Some(main));
    }
    window.set_hide_on_close(true);
    window.set_focusable(behavior.accepts_keyboard);
    if behavior.hides_when_inactive {
        let hidden_for_inactivity = Rc::new(Cell::new(false));
        observe_application_inactivity(window, window, hidden_for_inactivity.clone());
        if let Some(main) = main {
            observe_application_inactivity(main, window, hidden_for_inactivity);
        }
    }
}

fn observe_application_inactivity(
    trigger: &gtk::Window,
    panel: &gtk::Window,
    hidden_for_inactivity: Rc<Cell<bool>>,
) {
    let panel = panel.downgrade();
    trigger.connect_is_active_notify(move |_| {
        let panel = panel.clone();
        let hidden_for_inactivity = hidden_for_inactivity.clone();
        glib::idle_add_local_once(move || {
            let Some(panel) = panel.upgrade() else {
                return;
            };
            let Some(application) = panel.application() else {
                return;
            };
            if application.windows().iter().any(gtk::Window::is_active) {
                if hidden_for_inactivity.replace(false) {
                    panel.set_visible(true);
                }
            } else if panel.is_visible() {
                hidden_for_inactivity.set(true);
                panel.set_visible(false);
            }
        });
    });
}

fn schedule_layout_probe(
    window: &gtk::Window,
    toolbar: &adw::ToolbarView,
    header: &adw::HeaderBar,
    root: &gtk::Box,
    expected_content_width: i32,
    expected_content_height: i32,
) {
    if std::env::var_os("RINKA_GTK_LAYOUT_PROBE").is_none() {
        return;
    }
    let allocation_window = window.downgrade();
    let allocation_header = header.downgrade();
    let allocation_root = root.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        let Some(window) = allocation_window.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let Some(header) = allocation_header.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let Some(root) = allocation_root.upgrade() else {
            return glib::ControlFlow::Break;
        };
        emit_content_allocation(&window, &header, &root);
        glib::ControlFlow::Continue
    });
    let window = window.clone();
    let toolbar = toolbar.clone();
    let header = header.clone();
    let root = root.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(750), move || {
        emit_content_allocation(&window, &header, &root);
        let content_matches =
            root.width() == expected_content_width && root.height() == expected_content_height;
        eprintln!(
            "RINKA_GTK_WINDOW_CONTRACT title={:?} expected-content={}x{} content={}x{} header={}x{} toolbar={}x{} window={}x{} result={}",
            window.title().unwrap_or_default(),
            expected_content_width,
            expected_content_height,
            root.width(),
            root.height(),
            header.width(),
            header.height(),
            toolbar.width(),
            toolbar.height(),
            window.width(),
            window.height(),
            if content_matches { "PASS" } else { "FAIL" },
        );
    });
}

fn emit_content_allocation(window: &gtk::Window, header: &adw::HeaderBar, root: &gtk::Box) {
    eprintln!(
        "RINKA_GTK_CONTENT_ALLOCATION title={:?} content={}x{} header={}x{} window={}x{}",
        window.title().unwrap_or_default(),
        root.width(),
        root.height(),
        header.width(),
        header.height(),
        window.width(),
        window.height(),
    );
}

/// Runs a libadwaita application and returns its process status.
pub fn run(application: ApplicationSpec) -> i32 {
    if application.windows.is_empty() {
        eprintln!("GTK host error: application has no windows");
        return 1;
    }
    let app = adw::Application::builder()
        .application_id(&application.id)
        .build();
    let built_windows: Rc<RefCell<Vec<BuiltWindow>>> = Rc::new(RefCell::new(Vec::new()));
    let startup_failed = Rc::new(Cell::new(false));
    let activation_failed = startup_failed.clone();
    app.connect_activate(move |app| {
        if !built_windows.borrow().is_empty() {
            if let Some(main) = built_windows
                .borrow()
                .iter()
                .find(|built| built.window.transient_for().is_none())
            {
                main.window.present();
            }
            return;
        }
        let mut main_window: Option<gtk::Window> = None;
        for spec in &application.windows {
            match build_window(app, spec, main_window.as_ref()) {
                Ok(built) => {
                    if matches!(spec.kind, WindowKind::Main) {
                        main_window = Some(built.window.clone());
                    }
                    built.window.present();
                    built_windows.borrow_mut().push(built);
                }
                Err(error) => {
                    activation_failed.set(true);
                    eprintln!("GTK host error: {error}");
                }
            }
        }
    });
    // Consumer arguments belong to the declarative application, not to
    // GApplication's option parser. Supply only the executable identity after
    // the consumer has already interpreted its own command line.
    let status = app.run_with_args(&["rinka"]).value();
    if startup_failed.get() { 1 } else { status }
}

const fn orientation(axis: Axis) -> gtk::Orientation {
    match axis {
        Axis::Horizontal => gtk::Orientation::Horizontal,
        Axis::Vertical => gtk::Orientation::Vertical,
    }
}

const fn gtk_align(align: Align) -> gtk::Align {
    match align {
        Align::Start => gtk::Align::Start,
        Align::Center => gtk::Align::Center,
        Align::End => gtk::Align::End,
        Align::Stretch => gtk::Align::Fill,
    }
}

const fn spacing_pixels(spacing: Spacing) -> i32 {
    match spacing {
        Spacing::Joined => 0,
        Spacing::Compact => 6,
        Spacing::Related => 12,
        Spacing::Section => 18,
        Spacing::Content => 24,
    }
}

const fn content_spacing_pixels(context: LayoutContext, spacing: Spacing) -> i32 {
    match (context, spacing) {
        (LayoutContext::AuxiliaryPanel, Spacing::Section) => spacing_pixels(Spacing::Compact),
        (LayoutContext::AuxiliaryPanel, Spacing::Content) => spacing_pixels(Spacing::Related),
        _ => spacing_pixels(spacing),
    }
}

const fn stack_insets(context: LayoutContext, spacing: Spacing) -> (i32, i32) {
    match (context, spacing) {
        (LayoutContext::AuxiliaryPanel, Spacing::Content) => (
            spacing_pixels(Spacing::Related),
            spacing_pixels(Spacing::Related),
        ),
        _ => {
            let inset = content_spacing_pixels(context, spacing);
            (inset, inset)
        }
    }
}

const fn symbol_name(symbol: Symbol) -> &'static str {
    match symbol {
        Symbol::Back => "go-previous-symbolic",
        Symbol::Forward => "go-next-symbolic",
        Symbol::Add => "list-add-symbolic",
        Symbol::Refresh => "view-refresh-symbolic",
        Symbol::Search => "system-search-symbolic",
        Symbol::Home => "user-home-symbolic",
        Symbol::Folder => "folder-symbolic",
        Symbol::File => "text-x-generic-symbolic",
        Symbol::Code => "text-x-script-symbolic",
        Symbol::Image => "image-x-generic-symbolic",
        Symbol::Terminal => "utilities-terminal-symbolic",
        Symbol::Settings => "emblem-system-symbolic",
        Symbol::More => "view-more-symbolic",
        Symbol::Grid => "view-grid-symbolic",
        Symbol::List => "view-list-symbolic",
        Symbol::Columns => "view-dual-symbolic",
        Symbol::Gallery => "view-paged-symbolic",
        Symbol::Sort => "view-sort-ascending-symbolic",
        Symbol::Share => "send-to-symbolic",
        Symbol::Tag => "tag-symbolic",
        Symbol::Disclosure => "go-next-symbolic",
        Symbol::Warning => "dialog-warning-symbolic",
    }
}

const fn status_icon(tone: StatusTone) -> &'static str {
    match tone {
        StatusTone::Empty => "folder-open-symbolic",
        StatusTone::Busy => "content-loading-symbolic",
        StatusTone::Error => "dialog-warning-symbolic",
        StatusTone::Informational => "dialog-information-symbolic",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LayoutContext, ToolbarLayout, action_group_uses_direct_compact_buttons,
        content_spacing_pixels, progress_percentage_text, run, stack_insets,
        table_cell_accessible_description, toolbar_layout, validate_element,
    };
    use rinka_core::{
        ApplicationSpec, ButtonMaterial, Spacing, ToolbarAction, ToolbarGroupDisplay, ToolbarItem,
        ToolbarPlacement, button, progress,
    };

    #[test]
    fn auxiliary_panel_resolves_compact_native_spacing() {
        assert_eq!(
            content_spacing_pixels(LayoutContext::AuxiliaryPanel, Spacing::Content),
            12
        );
        assert_eq!(
            content_spacing_pixels(LayoutContext::AuxiliaryPanel, Spacing::Section),
            6
        );
        assert_eq!(
            content_spacing_pixels(LayoutContext::AuxiliaryPanel, Spacing::Related),
            12
        );
        assert_eq!(
            content_spacing_pixels(LayoutContext::Standard, Spacing::Content),
            24
        );
        assert_eq!(
            stack_insets(LayoutContext::AuxiliaryPanel, Spacing::Content),
            (12, 12)
        );
    }

    #[test]
    fn validation_rejects_unrepresentable_or_inaccessible_elements() {
        let glass = button("Open", "Open file", || {}).button_material(ButtonMaterial::Glass);
        assert!(validate_element(&glass).is_err());

        let unnamed = button("Open", "", || {});
        assert!(validate_element(&unnamed).is_err());

        let invalid_progress = progress(f64::NAN, "Transfer progress");
        assert!(validate_element(&invalid_progress).is_err());

        let valid = button("Open", "Open file", || {});
        assert!(validate_element(&valid).is_ok());
    }

    #[test]
    fn toolbar_group_display_selects_the_declared_native_representation() {
        let group = || {
            ToolbarItem::action_group(
                "navigation",
                "Navigation",
                "Move through history",
                ToolbarPlacement::Leading,
                [ToolbarAction::new(
                    "back",
                    "Back",
                    rinka_core::Symbol::Back,
                    "Go back",
                    || {},
                )],
            )
        };
        assert_eq!(toolbar_layout(&group()), ToolbarLayout::Adaptive);
        assert_eq!(
            toolbar_layout(&group().group_display(ToolbarGroupDisplay::Expanded)),
            ToolbarLayout::Expanded
        );
        assert_eq!(
            toolbar_layout(&group().group_display(ToolbarGroupDisplay::Collapsed)),
            ToolbarLayout::Compact
        );
    }

    #[test]
    fn compact_navigation_keeps_a_small_action_group_directly_visible() {
        assert!(action_group_uses_direct_compact_buttons(1));
        assert!(action_group_uses_direct_compact_buttons(2));
        assert!(!action_group_uses_direct_compact_buttons(0));
        assert!(!action_group_uses_direct_compact_buttons(3));
    }

    #[test]
    fn progress_text_exposes_the_declared_fraction_as_a_percentage() {
        assert_eq!(progress_percentage_text(0.58), "58%");
        assert_eq!(progress_percentage_text(1.0), "100%");
    }

    #[test]
    fn table_cells_expose_column_specific_descriptions() {
        assert_eq!(table_cell_accessible_description("Name"), "Name column");
        assert_eq!(table_cell_accessible_description("Size"), "Size column");
    }

    #[test]
    fn empty_application_returns_a_failure_status_without_starting_gtk() {
        assert_eq!(
            run(ApplicationSpec {
                id: "jp.bunko.rinka.empty".to_owned(),
                name: "Empty".to_owned(),
                windows: Vec::new(),
            }),
            1
        );
    }
}
