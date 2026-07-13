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
- `examples/explorer` is the required consumer. A public UI contract is not
  considered verified until the explorer uses it on both supported platforms.

## Visual rules

1. Common code expresses meaning, not platform pixels. Typography, material,
   control prominence, window kind, panel role, and spacing density are semantic
   values translated by each platform adapter.
2. Prefer native controls and containers. Canvas drawing is reserved for a
   component whose content is inherently graphical.
3. Platform-owned geometry is queried from the platform or left to native
   layout. Do not duplicate traffic-light, title-bar, header-bar, font, corner,
   or control metrics in the common crate.
4. macOS uses current AppKit window, toolbar, sidebar, control, color, font, and
   material behavior. Glass is a navigation and control layer; content remains
   visually primary.
5. Ubuntu uses GTK 4 and libadwaita patterns, including header bars, adaptive
   split views, native status pages, and platform spacing.
6. Every interactive element has a visible label or tooltip, an accessibility
   label, keyboard behavior, enabled state, and focus behavior.
7. Visual verification requires real light and dark captures at useful and
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

## Gate

Run `make typecheck`, `make lint`, `make test`, and `make build`. Then run the
consumer fixture and capture the real platform rendering before landing.
