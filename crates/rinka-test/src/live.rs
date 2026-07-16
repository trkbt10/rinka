//! Live platform driver.
//!
//! On macOS the driver mounts the real AppKit adapter in-process and drives
//! it through the run loop the test owns (see [`TestHost`]). On the GTK and
//! WinUI platforms the live driver is not implemented yet: every verb is
//! present so consumer suites compile unchanged, and each returns the typed
//! [`HarnessError::UnsupportedPlatform`] diagnostic instead of silently
//! doing nothing. The deterministic [`crate::HeadlessHost`] works on every
//! platform.

#[cfg(target_os = "macos")]
mod macos {
    use crate::error::HarnessError;
    use crate::query::{Locator, find_node, tree_snapshot};
    use rinka_core::{ApplicationSpec, ElementKind};
    use rinka_macos::{AppKitHandle, AppKitTestHost, window_server_session_available};
    use std::path::Path;

    /// Bounded number of main-loop turns a settlement wait is granted; the
    /// same allowance the adapter's transition probe uses.
    const MAX_SETTLE_TURNS: usize = 200;
    /// Turns the settlement conditions must hold consecutively while the
    /// split-resize epoch stays unchanged — the probe's quiet-epoch rule.
    const REQUIRED_QUIET_TURNS: usize = 2;

    /// `NSEventModifierFlagShift`.
    const MODIFIER_SHIFT: usize = 1 << 17;
    /// `NSEventModifierFlagControl`.
    const MODIFIER_CONTROL: usize = 1 << 18;
    /// `NSEventModifierFlagOption`.
    const MODIFIER_OPTION: usize = 1 << 19;
    /// `NSEventModifierFlagCommand`.
    const MODIFIER_COMMAND: usize = 1 << 20;

    /// Modifier set carried by a posted key chord.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct ChordModifiers {
        /// The Command key.
        pub command: bool,
        /// The Shift key.
        pub shift: bool,
        /// The Control key.
        pub control: bool,
        /// The Option key.
        pub option: bool,
    }

    impl ChordModifiers {
        /// The platform primary modifier alone (Command on macOS).
        pub const fn primary() -> Self {
            Self {
                command: true,
                shift: false,
                control: false,
                option: false,
            }
        }

        fn native_flags(self) -> usize {
            let mut flags = 0;
            if self.command {
                flags |= MODIFIER_COMMAND;
            }
            if self.shift {
                flags |= MODIFIER_SHIFT;
            }
            if self.control {
                flags |= MODIFIER_CONTROL;
            }
            if self.option {
                flags |= MODIFIER_OPTION;
            }
            flags
        }
    }

    /// One element located in the mounted live tree.
    ///
    /// The handle stores its locator and re-resolves on every verb, so it
    /// stays truthful across reconciliations that replace the native view.
    #[derive(Clone, Debug)]
    pub struct ElementHandle {
        locator: Locator,
    }

    /// A live application mounted against the real macOS adapter,
    /// in-process, with the run loop owned by the test.
    ///
    /// Mount on the process main thread (an integration test with
    /// `harness = false`); [`TestHost::settle`] and the act verbs pump the
    /// loop in bounded bursts, so no `NSApplication run` call ever takes
    /// the process over. All input is synthetic and confined to this
    /// process: no other application, no global pointer, no TCC-gated
    /// external accessibility API is touched.
    pub struct TestHost {
        inner: AppKitTestHost,
    }

    impl std::fmt::Debug for TestHost {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("TestHost")
                .field("windows", &self.inner.window_count())
                .finish()
        }
    }

    impl TestHost {
        /// Mounts the application against the live adapter.
        ///
        /// Fails with the typed [`HarnessError::NoWindowServerSession`]
        /// when the process cannot host windows (headless CI); a runner
        /// logs that reason and skips instead of passing silently.
        /// `RINKA_TEST_ASSUME_NO_WINDOW_SESSION=1` forces that failure so
        /// a gate's skip handling is provable from a windowed session (a
        /// control experiment, following the repo's env-gated diagnostics).
        pub fn mount(application: ApplicationSpec) -> Result<Self, HarnessError> {
            if std::env::var_os("RINKA_TEST_ASSUME_NO_WINDOW_SESSION").is_some()
                || !window_server_session_available()
            {
                return Err(HarnessError::NoWindowServerSession);
            }
            let inner = AppKitTestHost::mount(application)
                .map_err(|error| HarnessError::Platform(error.to_string()))?;
            let host = Self { inner };
            host.settle()?;
            Ok(host)
        }

        /// Runs the application loop for one short burst.
        pub fn pump_turn(&self) {
            self.inner.pump_turn();
        }

        /// Returns how many declared windows are hosted.
        pub fn window_count(&self) -> usize {
            self.inner.window_count()
        }

        /// Finds an element by declarative key across every window.
        pub fn find_by_key(&self, key: &str) -> Result<ElementHandle, HarnessError> {
            self.find(Locator::Key(key.to_owned()))
        }

        /// Finds an element by accessibility label across every window.
        ///
        /// The walk matches [`rinka_core::Props::accessibility_name`] on
        /// the mounted tree — in-process, no external AX API and no TCC
        /// grant. Reads of the located element go through the native
        /// control's own `NSAccessibility` surface.
        pub fn find_by_label(&self, label: &str) -> Result<ElementHandle, HarnessError> {
            self.find(Locator::Label(label.to_owned()))
        }

        /// Returns whether any mounted element matches the key right now.
        pub fn exists_by_key(&self, key: &str) -> bool {
            self.lookup(&Locator::Key(key.to_owned())).is_some()
        }

        /// Returns whether any mounted element matches the label right now.
        pub fn exists_by_label(&self, label: &str) -> bool {
            self.lookup(&Locator::Label(label.to_owned())).is_some()
        }

        fn find(&self, locator: Locator) -> Result<ElementHandle, HarnessError> {
            self.resolve(&locator)?;
            Ok(ElementHandle { locator })
        }

        fn lookup(&self, locator: &Locator) -> Option<(AppKitHandle, ElementKind)> {
            (0..self.inner.window_count()).find_map(|index| {
                self.inner
                    .with_mounted(index, |root| {
                        find_node(root, locator)
                            .map(|node| (node.handle().clone(), node.element().kind()))
                    })
                    .flatten()
            })
        }

        fn resolve(&self, locator: &Locator) -> Result<(AppKitHandle, ElementKind), HarnessError> {
            self.lookup(locator).ok_or_else(|| HarnessError::NotFound {
                locator: locator.clone(),
            })
        }

        fn resolve_for_verb(
            &self,
            element: &ElementHandle,
            verb: &'static str,
            accepts: impl Fn(ElementKind) -> bool,
        ) -> Result<AppKitHandle, HarnessError> {
            let (handle, kind) = self.resolve(&element.locator)?;
            if accepts(kind) {
                Ok(handle)
            } else {
                Err(HarnessError::WrongRole {
                    locator: element.locator.clone(),
                    verb,
                    found: kind,
                })
            }
        }

        fn platform(result: Result<(), rinka_macos::AppKitError>) -> Result<(), HarnessError> {
            result.map_err(|error| HarnessError::Platform(error.to_string()))
        }

        /// Presses a native button through `performClick:` — the connected
        /// target/action dispatch a user click performs — then settles.
        pub fn press(&self, element: &ElementHandle) -> Result<(), HarnessError> {
            let handle = self.resolve_for_verb(element, "press", |kind| {
                matches!(kind, ElementKind::Button | ElementKind::Toggle)
            })?;
            Self::platform(self.inner.press(&handle))?;
            self.settle()
        }

        /// Toggles a native checkbox through `performClick:` (the user
        /// path: the click flips the state and fires the action), settles,
        /// and returns the resulting native checked state.
        pub fn toggle(&self, element: &ElementHandle) -> Result<bool, HarnessError> {
            let handle =
                self.resolve_for_verb(element, "toggle", |kind| kind == ElementKind::Toggle)?;
            Self::platform(self.inner.press(&handle))?;
            self.settle()?;
            self.is_checked(element)
        }

        /// Sends one primary click through the window's real event routing
        /// at the element's center (hit testing included), then settles.
        pub fn click_center(&self, element: &ElementHandle) -> Result<(), HarnessError> {
            let (handle, _) = self.resolve(&element.locator)?;
            Self::platform(self.inner.click_center(&handle))?;
            self.settle()
        }

        /// Focuses a native text field and inserts `text` through its field
        /// editor — a real editing session — then settles.
        ///
        /// The control's action (which delivers the value to the
        /// component) fires by the control's own rules: search fields send
        /// it as typing pauses, plain fields on commit; use
        /// [`Self::commit_text`] for the deterministic Return-key commit.
        pub fn type_text(&self, element: &ElementHandle, text: &str) -> Result<(), HarnessError> {
            let handle =
                self.resolve_for_verb(element, "type_text", |kind| kind == ElementKind::Input)?;
            Self::platform(self.inner.type_text(&handle, text))?;
            self.settle()
        }

        /// Commits the focused field by sending a Return key-down through
        /// the window's real event routing, then settles.
        pub fn commit_text(&self, element: &ElementHandle) -> Result<(), HarnessError> {
            let handle =
                self.resolve_for_verb(element, "commit_text", |kind| kind == ElementKind::Input)?;
            Self::platform(self.inner.commit_text(&handle))?;
            self.settle()
        }

        /// Reads the element's value off the native control's accessibility
        /// surface (`accessibilityValue`, falling back to `stringValue`).
        pub fn read_value(&self, element: &ElementHandle) -> Result<String, HarnessError> {
            let (handle, kind) = self.resolve(&element.locator)?;
            self.inner
                .read_value(&handle)
                .ok_or(HarnessError::WrongRole {
                    locator: element.locator.clone(),
                    verb: "read_value",
                    found: kind,
                })
        }

        /// Reads the native control's accessibility label.
        pub fn read_label(&self, element: &ElementHandle) -> Result<String, HarnessError> {
            let (handle, kind) = self.resolve(&element.locator)?;
            self.inner
                .read_accessibility_label(&handle)
                .ok_or(HarnessError::WrongRole {
                    locator: element.locator.clone(),
                    verb: "read_label",
                    found: kind,
                })
        }

        /// Reads the native enabled state of an interactive control.
        pub fn is_enabled(&self, element: &ElementHandle) -> Result<bool, HarnessError> {
            let (handle, kind) = self.resolve(&element.locator)?;
            self.inner
                .is_enabled(&handle)
                .ok_or(HarnessError::WrongRole {
                    locator: element.locator.clone(),
                    verb: "is_enabled",
                    found: kind,
                })
        }

        /// Reads the native checked state of a toggle.
        pub fn is_checked(&self, element: &ElementHandle) -> Result<bool, HarnessError> {
            let (handle, kind) = self.resolve(&element.locator)?;
            self.inner
                .is_checked(&handle)
                .ok_or(HarnessError::WrongRole {
                    locator: element.locator.clone(),
                    verb: "is_checked",
                    found: kind,
                })
        }

        /// Selects a mounted list row in its native table and settles.
        ///
        /// The selection goes through
        /// `selectRowIndexes:byExtendingSelection:`, whose selection-change
        /// notification rinka's table delegate translates into the row's
        /// stable activate binding — the exact consumer path of a user
        /// click. This is the driver verb that closes the recorded gap
        /// where the synthetic pointer probe could not drive collection
        /// rows.
        pub fn select_row(&self, element: &ElementHandle) -> Result<(), HarnessError> {
            let handle =
                self.resolve_for_verb(element, "select_row", |kind| kind == ElementKind::ListRow)?;
            Self::platform(self.inner.select_row(&handle))?;
            self.settle()
        }

        /// Posts one key chord through the real event queue and settles;
        /// the next pump dequeues it through the same path hardware input
        /// takes, so accelerator routing and menu key equivalents observe
        /// it.
        pub fn post_chord(
            &self,
            characters: &str,
            key_code: u16,
            modifiers: ChordModifiers,
        ) -> Result<(), HarnessError> {
            self.inner
                .post_key(characters, key_code, modifiers.native_flags());
            self.settle()
        }

        /// Activates an application menu bar item through native menu
        /// dispatch and settles.
        pub fn activate_menu_item(
            &self,
            menu_title: &str,
            item_title: &str,
        ) -> Result<(), HarnessError> {
            Self::platform(self.inner.activate_menu_item(menu_title, item_title))?;
            self.settle()
        }

        /// Opens the element's context menu through the accessibility
        /// show-menu action (a timer closes it again); returns whether the
        /// native menu actually began tracking.
        pub fn open_context_menu(&self, element: &ElementHandle) -> Result<bool, HarnessError> {
            let (handle, _) = self.resolve(&element.locator)?;
            let opened = self
                .inner
                .open_context_menu(&handle)
                .map_err(|error| HarnessError::Platform(error.to_string()))?;
            self.settle()?;
            Ok(opened)
        }

        /// Activates one item of the element's context menu through its
        /// native target/action pair and settles.
        pub fn activate_context_menu_item(
            &self,
            element: &ElementHandle,
            item_title: &str,
        ) -> Result<(), HarnessError> {
            let (handle, _) = self.resolve(&element.locator)?;
            Self::platform(self.inner.activate_context_menu_item(&handle, item_title))?;
            self.settle()
        }

        /// Waits for the adapter's named settlement conditions: no pending
        /// split restore, settled controlled outlines, resolved source
        /// widths, and a quiet split-resize epoch across consecutive turns.
        ///
        /// No arbitrary sleep is involved: each turn is one bounded pump of
        /// the real loop, and on timeout the error names every condition
        /// still unmet — the generalization of the transition probe's
        /// settlement wait.
        pub fn settle(&self) -> Result<(), HarnessError> {
            let mut observed_epoch = self.inner.observe_settlement().split_epoch;
            let mut quiet_turns = 0;
            let mut last_unmet: Vec<String> = Vec::new();
            for _ in 0..MAX_SETTLE_TURNS {
                self.inner.pump_turn();
                if let Some(error) = self.inner.take_render_error() {
                    return Err(HarnessError::Render(error));
                }
                let observation = self.inner.observe_settlement();
                let unmet = observation.unmet_conditions();
                let epoch_quiet = observation.split_epoch == observed_epoch;
                if unmet.is_empty() && epoch_quiet {
                    quiet_turns += 1;
                    if quiet_turns >= REQUIRED_QUIET_TURNS {
                        return Ok(());
                    }
                } else {
                    observed_epoch = observation.split_epoch;
                    quiet_turns = 0;
                }
                last_unmet = unmet.iter().map(ToString::to_string).collect();
                if !epoch_quiet {
                    last_unmet.push("split-epoch-quiet".to_owned());
                }
            }
            if last_unmet.is_empty() {
                last_unmet.push("split-epoch-quiet".to_owned());
            }
            Err(HarnessError::SettlementTimeout {
                turns: MAX_SETTLE_TURNS,
                unmet: last_unmet,
            })
        }

        /// Pumps until `predicate` holds, bounded, settling the adapter's
        /// built-in conditions on each turn; on timeout the failure names
        /// the consumer condition.
        pub fn settle_until(
            &self,
            condition: &str,
            mut predicate: impl FnMut(&Self) -> bool,
        ) -> Result<(), HarnessError> {
            for _ in 0..MAX_SETTLE_TURNS {
                self.inner.pump_turn();
                if let Some(error) = self.inner.take_render_error() {
                    return Err(HarnessError::Render(error));
                }
                if predicate(self) {
                    return Ok(());
                }
            }
            Err(HarnessError::SettlementTimeout {
                turns: MAX_SETTLE_TURNS,
                unmet: vec![condition.to_owned()],
            })
        }

        /// Renders the window at `index` into a PNG at its backing scale
        /// (in-process capture, no screen-recording permission), decodes
        /// the file's header, and returns its verified pixel dimensions.
        pub fn capture_window_png(
            &self,
            index: usize,
            path: &Path,
        ) -> Result<(u32, u32), HarnessError> {
            self.inner
                .capture_window_png(index, path)
                .map_err(|error| HarnessError::Capture {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                })?;
            crate::capture::png_dimensions(path)
        }

        /// Renders one mounted element into a PNG, decodes the file's
        /// header, and returns its verified pixel dimensions.
        pub fn capture_element_png(
            &self,
            element: &ElementHandle,
            path: &Path,
        ) -> Result<(u32, u32), HarnessError> {
            let (handle, _) = self.resolve(&element.locator)?;
            self.inner
                .capture_element_png(&handle, path)
                .map_err(|error| HarnessError::Capture {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                })?;
            crate::capture::png_dimensions(path)
        }

        /// Snapshots the mounted element trees of every window.
        pub fn tree_snapshot(&self) -> String {
            let mut lines = String::new();
            for index in 0..self.inner.window_count() {
                lines.push_str(&format!("window {index}:\n"));
                if let Some(snapshot) = self.inner.with_mounted(index, tree_snapshot) {
                    lines.push_str(&snapshot);
                }
            }
            lines
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::{ChordModifiers, ElementHandle, TestHost};

#[cfg(not(target_os = "macos"))]
mod unsupported {
    use crate::error::HarnessError;
    use rinka_core::ApplicationSpec;
    use std::convert::Infallible;
    use std::path::Path;

    /// The platform whose live driver is not implemented yet.
    const LIVE_PLATFORM: &str = if cfg!(target_os = "linux") {
        "GTK"
    } else if cfg!(target_os = "windows") {
        "WinUI"
    } else {
        "this platform's"
    };

    /// Modifier set carried by a posted key chord.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct ChordModifiers {
        /// The platform primary command modifier.
        pub command: bool,
        /// The Shift key.
        pub shift: bool,
        /// The Control key.
        pub control: bool,
        /// The Option/Alt key.
        pub option: bool,
    }

    impl ChordModifiers {
        /// The platform primary modifier alone.
        pub const fn primary() -> Self {
            Self {
                command: true,
                shift: false,
                control: false,
                option: false,
            }
        }
    }

    /// One element located in the mounted live tree (never constructible on
    /// a platform without a live driver).
    #[derive(Debug)]
    pub struct ElementHandle {
        never: Infallible,
    }

    impl Clone for ElementHandle {
        fn clone(&self) -> Self {
            match self.never {}
        }
    }

    /// Live application host; on this platform every verb reports the typed
    /// [`HarnessError::UnsupportedPlatform`] diagnostic. The deterministic
    /// [`crate::HeadlessHost`] works everywhere.
    #[derive(Debug)]
    pub struct TestHost {
        never: Infallible,
    }

    /// Builds the typed diagnostic for one unimplemented driver verb.
    fn unsupported(verb: &'static str) -> HarnessError {
        HarnessError::UnsupportedPlatform {
            platform: LIVE_PLATFORM,
            verb,
        }
    }

    impl TestHost {
        /// Reports the live driver as unimplemented on this platform.
        pub fn mount(_application: ApplicationSpec) -> Result<Self, HarnessError> {
            Err(unsupported("mount"))
        }

        /// Unreachable on this platform (no host can be constructed).
        pub fn pump_turn(&self) {
            match self.never {}
        }

        /// Unreachable on this platform (no host can be constructed).
        pub fn window_count(&self) -> usize {
            match self.never {}
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn find_by_key(&self, _key: &str) -> Result<ElementHandle, HarnessError> {
            Err(unsupported("find_by_key"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn find_by_label(&self, _label: &str) -> Result<ElementHandle, HarnessError> {
            Err(unsupported("find_by_label"))
        }

        /// Unreachable on this platform (no host can be constructed).
        pub fn exists_by_key(&self, _key: &str) -> bool {
            match self.never {}
        }

        /// Unreachable on this platform (no host can be constructed).
        pub fn exists_by_label(&self, _label: &str) -> bool {
            match self.never {}
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn press(&self, _element: &ElementHandle) -> Result<(), HarnessError> {
            Err(unsupported("press"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn toggle(&self, _element: &ElementHandle) -> Result<bool, HarnessError> {
            Err(unsupported("toggle"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn click_center(&self, _element: &ElementHandle) -> Result<(), HarnessError> {
            Err(unsupported("click_center"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn type_text(&self, _element: &ElementHandle, _text: &str) -> Result<(), HarnessError> {
            Err(unsupported("type_text"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn commit_text(&self, _element: &ElementHandle) -> Result<(), HarnessError> {
            Err(unsupported("commit_text"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn read_value(&self, _element: &ElementHandle) -> Result<String, HarnessError> {
            Err(unsupported("read_value"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn read_label(&self, _element: &ElementHandle) -> Result<String, HarnessError> {
            Err(unsupported("read_label"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn is_enabled(&self, _element: &ElementHandle) -> Result<bool, HarnessError> {
            Err(unsupported("is_enabled"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn is_checked(&self, _element: &ElementHandle) -> Result<bool, HarnessError> {
            Err(unsupported("is_checked"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn select_row(&self, _element: &ElementHandle) -> Result<(), HarnessError> {
            Err(unsupported("select_row"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn post_chord(
            &self,
            _characters: &str,
            _key_code: u16,
            _modifiers: ChordModifiers,
        ) -> Result<(), HarnessError> {
            Err(unsupported("post_chord"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn activate_menu_item(
            &self,
            _menu_title: &str,
            _item_title: &str,
        ) -> Result<(), HarnessError> {
            Err(unsupported("activate_menu_item"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn open_context_menu(&self, _element: &ElementHandle) -> Result<bool, HarnessError> {
            Err(unsupported("open_context_menu"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn activate_context_menu_item(
            &self,
            _element: &ElementHandle,
            _item_title: &str,
        ) -> Result<(), HarnessError> {
            Err(unsupported("activate_context_menu_item"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn settle(&self) -> Result<(), HarnessError> {
            Err(unsupported("settle"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn settle_until(
            &self,
            _condition: &str,
            _predicate: impl FnMut(&Self) -> bool,
        ) -> Result<(), HarnessError> {
            Err(unsupported("settle_until"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn capture_window_png(
            &self,
            _index: usize,
            _path: &Path,
        ) -> Result<(u32, u32), HarnessError> {
            Err(unsupported("capture_window_png"))
        }

        /// Reports the live driver as unimplemented on this platform.
        pub fn capture_element_png(
            &self,
            _element: &ElementHandle,
            _path: &Path,
        ) -> Result<(u32, u32), HarnessError> {
            Err(unsupported("capture_element_png"))
        }

        /// Unreachable on this platform (no host can be constructed).
        pub fn tree_snapshot(&self) -> String {
            match self.never {}
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub use unsupported::{ChordModifiers, ElementHandle, TestHost};
