# Rinka

Rinka is a Rust-native declarative UI library for macOS, Ubuntu, and
Windows. An
application describes an immutable element tree and typed window set in Rust;
Rinka reconciles changes into AppKit on macOS, GTK 4/libadwaita on Ubuntu,
and WinUI 3 on Windows. The architecture follows the useful part of the React Native model — a
platform-neutral declarative tree driving retained native views — without a
JavaScript runtime or a generic serialized bridge.

`crates/rinka-winui` is the default Windows adapter. It projects the common
retained tree through the pinned Microsoft `windows-reactor` revision
`36bd4a592df4fc5565f9e589138bc27110016904`, hosts main windows and activity
panels in one Windows App SDK application loop, and uses native TitleBar,
NavigationView, CommandBar, list, input, toggle, progress, and accessibility
objects. `crates/rinka-windows` remains the classic Win32/Common Controls v6
contract probe selected only by `--windows-contract-probe`.

Windows Server 2025 Desktop Experience is the automated Windows verification
host. Evidence captured there applies to that server and does not certify
genuine Windows 11 DWM corners or backdrop composition.

The first product milestone provides a reusable core, deterministic headless
adapter, AppKit adapter, GTK/libadwaita adapter, and a file-explorer consumer
that exercises navigation, toolbar actions, lists, content, status, utility
panels, text input, empty state, and multiple windows.
The Windows adapters and their headful consumers exercise the same semantic
surface through stable projected identities, native window identities, and
Microsoft UI Automation.

Product code is developed in a purpose-named worktree. See `AGENTS.md` for
ownership, visual constraints, and verification requirements.

## Native workspace contract

`UiPattern` is the shared vocabulary for standard desktop compositions. A
consumer mounts semantic regions in the order declared by the pattern; it does
not choose an operating-system widget. `NavigationSplit` mounts a navigation
sidebar and content, `UtilitySplit` mounts content and an inspector, and
`NavigationWorkspace` mounts all three regions.

| Shared pattern | AppKit route | GTK/libadwaita route | WinUI route |
| --- | --- | --- | --- |
| `NavigationSplit` | `NSSplitViewController` sidebar item | adaptive split surface | navigation layout |
| `UtilitySplit` | `NSSplitViewController` inspector item | `AdwOverlaySplitView` utility pane | content/inspector grid |
| `NavigationWorkspace` | one three-item `NSSplitViewController` | nested navigation and utility split surfaces | adaptive `NavigationView` with inspector |

Win32 uses standard tree/list controls inside the same pattern contract when
the legacy toolkit has no equivalent high-level container. Collapse policy is
declarative, while pane thickness, safe areas, dividers, transitions, and
system materials remain owned by each native toolkit.

On macOS a workspace window receives the standard Sidebar and Inspector toolbar
items and the corresponding tracking separators. Applications add only their
own actions:

```rust
let content = mount_pattern(
    UiPattern::NavigationWorkspace {
        sidebar_collapsible: true,
        inspector_collapsible: true,
    },
    [sidebar, directory, inspector],
);
```

Collapsing either auxiliary region keeps the top-level window frame fixed and
redistributes space among sibling panes. A window retains one root element kind
for its lifetime; a component that changes that kind receives a typed render
error before the promoted native root is mutated.

## Reactive window content

`WindowContent::component` connects native events to a retained Rust
`Component`. Its state is the single owner of controlled selection, expansion,
toggle, and sort values, and reconciliation patches the existing native root:

```rust
WindowSpec {
    content: WindowContent::component(ExplorerComponent::new(scene)),
    // window identity, size, toolbar, and kind omitted
}
```

Event callbacks must emit a component message instead of maintaining a second
adapter-side model. This keeps visible content, native selection, accessibility
state, and inspectors derived from the same state transition.

## Native collection patterns

`CollectionPattern::NavigationSidebar` requests navigation placement and
selection semantics. `CollectionPattern::Outline` requests hierarchical
content without sidebar treatment. Both route to the toolkit's native tree or
outline control. `section_header`, `outline_children`, `expanded`, and
`on_expansion_change` declare sections and hierarchy. A terminal row should not
request a disclosure indicator.

`CollectionPattern::DataTable` accepts a stable schema. The primary row title
belongs to the first column; `table_cells` supplies values for the remaining
columns:

```rust
let files = list(
    "Files",
    [
        list_row(
            "Cargo.toml",
            None,
            Some(Symbol::Code),
            true,
            false,
            "Cargo.toml, selected, 2.4 kilobytes",
            || {},
        )
        .table_cells(["Today, 10:42", "2.4 KB", "TOML document"]),
    ],
)
.table_columns([
    TableColumn::new("name", "Name").sorted(SortDirection::Ascending),
    TableColumn::new("modified", "Date Modified").sortable(true),
    TableColumn::new("size", "Size").sortable(true),
    TableColumn::new("kind", "Kind").sortable(true),
])
.collection_pattern(CollectionPattern::DataTable)
.on_sort_change(|sort| dispatch.emit(Message::SetSort(sort)));
```

Column identifiers must be unique, and each row must supply exactly one value
for every column after the primary column. Invalid schemas are rejected before
the retained native tree is mutated. AppKit derives header, cell, minimum, and
scroll geometry from the current native metrics rather than library-owned
pixel constants.

## Native toolbar composition

The toolbar model distinguishes plain actions, grouped actions,
single-selection groups, menus, and search. On macOS these map to native
`NSToolbarItem`, `NSToolbarItemGroup`, `NSMenuToolbarItem`, and
`NSSearchToolbarItem` objects. `ToolbarPlacement::Center` supplies the window's
centered item identity; leading and trailing placements remain native toolbar
regions rather than application-drawn titlebar content.

On Windows the same model maps to WinUI TitleBar and CommandBar controls.
Toolbar menus use native secondary commands, and search uses AutoSuggestBox.
Windows executable packages run `windows-reactor-setup::as_self_contained()`
from their target-specific `build.rs`; `crates/rinka-winui` remains a
runtime-staging-free library adapter.

## Verification

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

macOS changes additionally require a signed bundle, AppKit layout diagnostics,
and consumer-side visual verification for pane transitions, minimum window
size, every scene, and both system appearances. Automated gates do not replace
the visual gate.

```text
make macos-scene-matrix
make macos-transition-matrix
make macos-visual-matrix
```

The transition matrix checks 48 settled states in each appearance: three
Sidebar cycles, three Inspector cycles, and three combined cycles at both the
1120 by 720 useful size and the 760 by 520 minimum size. Every state must keep
the exact top-level window frame.

The visual matrix identifies windows by the launched process identifier,
requires stable CoreGraphics bounds, captures eight main-window PNGs and two
Busy-panel PNGs, and records point bounds, Retina pixel dimensions, and
SHA-256 digests. The resulting images still require source-blind human review.

Windows verification runs on a Windows Server 2025 Desktop Experience host
with Rust 1.97 MSVC and Windows SDK 26100. The WinUI adapter manifest declares
MSRV 1.95 because the pinned Microsoft reactor requires it; the rest of the
workspace retains its Rust 1.88 package contract where platform dependencies
permit it.

```text
powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/windows-bootstrap.ps1
cargo fetch --locked
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace --all-targets
powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/windows-scene-matrix.ps1
```

The Windows matrix records native class names, UI Automation properties,
PerMonitorV2 DPI awareness, three navigation and inspector pane cycles that
must preserve the top-level window rectangle, and Ready, Empty, Busy, and Error
captures in light/dark appearance at wide/narrow sizes.

The elevated bootstrap installs the Visual Studio 2022 C++ Build Tools workload,
the explicit MSVC x64/x86 and Windows SDK 26100 components, and the repository's
Rust 1.97 MSVC toolchain. It records the discovered installation and exact tool
versions in `target/windows-bootstrap.json`. Component identifiers and unattended
arguments follow Microsoft's [Build Tools component catalog](https://learn.microsoft.com/en-us/visualstudio/install/workload-component-id-vs-build-tools?view=visualstudio)
and [command-line installer contract](https://learn.microsoft.com/en-us/visualstudio/install/use-command-line-parameters-to-install-visual-studio?view=visualstudio);
Rust components follow the [rustup component contract](https://rust-lang.github.io/rustup/concepts/components.html).
