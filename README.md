# Rinka

Rinka is a Rust-native declarative UI library for macOS and Ubuntu. An
application describes an immutable element tree and typed window set in Rust;
Rinka reconciles changes into AppKit on macOS and GTK 4/libadwaita on
Ubuntu. The architecture follows the useful part of the React Native model — a
platform-neutral declarative tree driving retained native views — without a
JavaScript runtime or a generic serialized bridge.

The first product milestone provides a reusable core, deterministic headless
adapter, AppKit adapter, GTK/libadwaita adapter, and a file-explorer consumer
that exercises navigation, toolbar actions, lists, content, status, utility
panels, text input, empty state, and multiple windows.

Product code is developed in a purpose-named worktree. See `AGENTS.md` for
ownership, visual constraints, and verification requirements.
