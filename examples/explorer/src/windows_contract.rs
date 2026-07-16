//! Executable consumer probe covering every declarative element kind on Windows.

use rinka::{
    ApplicationSpec, Axis, CollectionPattern, InputKind, Size, StatusTone, Symbol, ToolbarDisplay,
    UiPattern, WindowId, WindowKind, WindowSpec, button, column, input, label, list, list_row,
    mount_pattern, progress, row, scroll, separator, spacer, status, toggle,
};

/// Builds a headful native-control contract probe.
pub fn application() -> ApplicationSpec {
    let navigation = list(
        "Contract navigation",
        [list_row(
            "Native controls",
            None,
            Some(Symbol::Folder),
            true,
            false,
            "Native controls",
            || {},
        )],
    )
    .collection_pattern(CollectionPattern::NavigationSidebar)
    .with_key("contract-navigation");

    let form = column([
        label("Windows native element contract").with_key("contract-label"),
        row([
            button("Apply", "Apply contract values", || {}).with_key("contract-button"),
            input(
                "",
                "Filter native controls",
                InputKind::Search,
                "Filter native controls",
                |_| {},
            )
            .with_key("contract-input"),
            toggle("Include hidden", true, "Include hidden items", |_| {})
                .with_key("contract-toggle"),
        ])
        .with_key("contract-row"),
        progress(0.62, "Contract progress 62 percent").with_key("contract-progress"),
        separator(Axis::Horizontal).with_key("contract-separator"),
        scroll(
            Axis::Vertical,
            column([
                status(
                    "Native controls are active",
                    "Win32 and Common Controls own the rendered objects.",
                    StatusTone::Informational,
                )
                .with_key("contract-status"),
                spacer(false, true).with_key("contract-spacer"),
            ])
            .with_key("contract-scroll-content"),
        )
        .with_key("contract-scroll"),
    ])
    .with_key("contract-form");

    let inspector = mount_pattern(
        UiPattern::UtilitySplit {
            inspector_collapsible: true,
        },
        [
            column([label("Properties")]).with_key("contract-properties"),
            column([label("Events")]).with_key("contract-events"),
        ],
    )
    .with_key("contract-utility-split");

    ApplicationSpec {
        id: "jp.bunko.rinka.windows-contract".to_owned(),
        name: "Rinka Windows Contract".to_owned(),
        // The Win32 contract probe declares no menu bar; the classic host
        // rejects a declared one with a typed diagnostic.
        menu_bar: rinka::MenuBar::default(),
        windows: vec![WindowSpec {
            id: WindowId::new("windows-contract-main"),
            title: "Rinka Windows Native Contract".to_owned(),
            kind: WindowKind::Main,
            initial_size: Size::new(1120.0, 720.0),
            minimum_size: Size::new(760.0, 520.0),
            toolbar: Vec::new(),
            toolbar_display: ToolbarDisplay::IconAndLabel,
            content: mount_pattern(
                UiPattern::NavigationWorkspace {
                    sidebar_collapsible: true,
                    inspector_collapsible: true,
                },
                [navigation, form, inspector],
            )
            .with_key("contract-workspace")
            .into(),
        }],
        // The contract probe's process lifetime is its single window.
        last_window_closed: rinka::LastWindowClosedPolicy::PlatformDefault,
    }
}
