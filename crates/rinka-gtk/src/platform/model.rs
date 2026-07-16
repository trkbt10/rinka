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
    pattern: RefCell<Option<UiPattern>>,
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
    pattern: Option<UiPattern>,
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
    pattern: Cell<CollectionPattern>,
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
            pattern: RefCell::new(details.pattern),
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
        pattern: Option<UiPattern>,
        auxiliaries: Vec<gtk::Widget>,
    ) -> Self {
        Self::with_details(
            widget.upcast(),
            HandleDetails {
                host_kind,
                pattern,
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
                pattern: None,
                workspace: None,
                list: None,
                row: None,
                row_object: None,
                auxiliaries,
                suppress_events,
            },
        )
    }

    fn workspace(widget: adw::OverlaySplitView, pattern: UiPattern, data: WorkspaceData) -> Self {
        Self::with_details(
            widget.upcast(),
            HandleDetails {
                host_kind: HostKind::Element(ElementKind::Pattern),
                pattern: Some(pattern),
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
                pattern: None,
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
                pattern: None,
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
        pattern: CollectionPattern,
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
            pattern: Cell::new(pattern),
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
        pattern: CollectionPattern,
        columns: &[TableColumn],
    ) {
        let previous_pattern = self.pattern.replace(pattern);
        let previous_columns = self.columns.replace(columns.to_vec());
        *self.accessibility_label.borrow_mut() = accessibility_label.to_owned();
        let schema_changed = !same_table_schema(&previous_columns, columns);
        if previous_pattern != pattern || (pattern.presents_columns() && schema_changed) {
            self.rebuild_presentation();
        } else {
            if let Some(child) = self.scroll.child() {
                child.update_property(&[gtk::accessible::Property::Label(accessibility_label)]);
            }
            if pattern.presents_columns() {
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
        let widget: gtk::Widget = match self.pattern.get() {
            pattern @ (CollectionPattern::NavigationSidebar | CollectionPattern::Outline) => {
                let tree_model =
                    gtk::TreeListModel::new(self.store.clone(), false, false, |object| {
                        row_from_object(object)
                            .map(|row| row.children.clone().upcast::<gio::ListModel>())
                    });
                let selection = self.make_selection(tree_model.clone());
                let factory = source_row_factory();
                let view = gtk::ListView::new(Some(selection.clone()), Some(factory));
                if pattern == CollectionPattern::NavigationSidebar {
                    view.add_css_class("navigation-sidebar");
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
            CollectionPattern::DataTable => self.build_table(&label),
            CollectionPattern::ContentList | CollectionPattern::EmbeddedList => {
                let selection = self.make_selection(self.store.clone());
                let factory = flat_row_factory();
                let view = gtk::ListView::new(Some(selection.clone()), Some(factory));
                if self.pattern.get() == CollectionPattern::ContentList {
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
