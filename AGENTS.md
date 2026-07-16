# Rinka product contract

Read this file and `README.md` before changing product code.

## Ownership

- `crates/rinka-core` owns the declarative element tree, reconciliation,
  state runtime, semantic roles, window descriptions, diagnostics, and the
  platform adapter contracts. It must not depend on an operating-system UI
  toolkit.
- `crates/rinka-headless` owns deterministic mutation recording and
  consumer tests for the common runtime.
- `crates/rinka-macos` owns AppKit object creation, target/action bridging,
  macOS window and panel behavior, materials, system colors, system fonts, and
  accessibility mapping.
- `crates/rinka-gtk` owns GTK 4 and libadwaita object creation, signals,
  Ubuntu window behavior, adaptive layout, native styles, and accessibility
  mapping.
- `crates/rinka-winui` owns the default Windows projection into native
  WinUI 3 controls, Windows App SDK window and panel hosting, Mica materials,
  adaptive NavigationView layout, Fluent tokens, and UI Automation mapping.
  It uses the pinned Microsoft `windows-reactor` source revision recorded in
  its manifest and requires Rust 1.95 or newer.
- `crates/rinka-windows` owns the classic Win32/Common Controls v6 contract
  probe, including per-monitor DPI behavior, native HWND identity, and the
  server-side compatibility checks. It is not the default Explorer renderer.
- `examples/explorer` is the required consumer. A public UI contract is not
  considered verified until the explorer uses it on both supported platforms.
  Windows contracts additionally require the Explorer consumer on Windows
  Server 2025 Desktop Experience.

## Visual rules

1. Common code expresses meaning, not platform pixels. Typography, material,
   control prominence, window kind, panel role, and spacing density are semantic
   values translated by each platform adapter.
2. Prefer native controls and containers. Canvas drawing is reserved for a
   component whose content is inherently graphical — terminal cell grids,
   audio meters, dashboard widget faces. The `Canvas` element is not an
   escape hatch for imitating a control: a canvas that draws a fake button,
   list, input, or any other native control violates this contract. Adapters
   that do not implement the canvas reject it with a typed diagnostic and
   never substitute another control for it.
3. Platform-owned geometry is queried from the platform or left to native
   layout. Do not duplicate traffic-light, title-bar, header-bar, font, corner,
   or control metrics in the common crate.
4. macOS uses current AppKit window, toolbar, sidebar, control, color, font, and
   material behavior. Glass is a navigation and control layer; content remains
   visually primary.
5. Ubuntu uses GTK 4 and libadwaita patterns, including header bars, adaptive
   split views, native status pages, and platform spacing.
6. The default Windows consumer uses WinUI 3 controls, the native TitleBar and
   NavigationView, Mica base surfaces, the Segoe UI Variable native default,
   and Fluent spacing on a four-epx grid. Acrylic is reserved for transient
   light-dismiss surfaces.
7. Windows Server 2025 Desktop Experience is the automated Windows host. Its
   captures prove the implementation on that server and do not certify genuine
   Windows 11 DWM corners or backdrop composition.
8. Every interactive element has a visible label or tooltip, an accessibility
   label, keyboard behavior, enabled state, and focus behavior.
9. Visual verification requires real light and dark captures at useful and
   narrow window sizes. A test-only tree snapshot is not visual evidence.

## Runtime rules

- Native objects live on the UI thread.
- Event connections are stable; reconciliation replaces the Rust handler held
  by the connection rather than reconnecting native signals every render.
- Sibling keys are unique. Duplicate keys are an error, not a debug-only
  assertion.
- Reconciliation validates a mutation plan before changing the host tree.
- Unsupported semantic capabilities return a typed diagnostic. The runtime
  does not silently substitute a visually unrelated control.
- Window and panel lifecycle is separate from child-view reconciliation.
- Unsafe Rust is confined to the macOS binding boundary and each unsafe block
  documents the platform invariant it relies on.
- Windows unsafe Rust is confined to `crates/rinka-windows/src/platform.rs`;
  every unsafe block documents the HWND, pointer, thread, or message-lifetime
  invariant it relies on.
- `crates/rinka-winui` does not exchange toolkit objects with
  `crates/rinka-windows`; the two adapters use different Windows binding
  generations and run as separate host paths.

## Gate

Run `make typecheck`, `make lint`, `make test`, and `make build`. Then run the
consumer fixture and capture the real platform rendering before landing.
Windows changes additionally require a Windows Server 2025 MSVC build,
PerMonitorV2 verification, UI Automation extraction, native class inspection,
three navigation/inspector pane cycles with a fixed top-level frame, and the
complete scene/appearance/size capture matrix.
Executable packages that use `rinka-winui` own their self-contained
Windows App SDK staging through `windows-reactor-setup` in `build.rs`; the
reusable adapter crate must not stage runtime files.
