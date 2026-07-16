//! Deterministic driver over the `rinka-headless` backend.
//!
//! The headless host gives the driver verbs — find by key or accessibility
//! label, press, toggle, type, read, settle — deterministic semantics with
//! no window server, so the harness's own behavior is testable everywhere.
//! Message delivery in the window runtime is synchronous, which makes each
//! verb settle before it returns; the bounded [`HeadlessHost::settle_until`]
//! exists so consumer conditions carry the same named-timeout contract the
//! live driver enforces.

use crate::error::HarnessError;
use crate::query::{Locator, find_node, tree_snapshot};
use rinka_core::{
    ElementKind, EventBindings, MountedNode, PlatformServices, Props, WindowContent, WindowRuntime,
};
use rinka_headless::HeadlessBackend;

/// Bounded number of settlement attempts before a typed timeout.
const MAX_SETTLE_TURNS: usize = 200;

/// One element located in the mounted headless tree.
///
/// The handle stores its locator and re-resolves on every verb, so it never
/// goes stale across reconciliations that replace native identities.
#[derive(Clone, Debug)]
pub struct HeadlessElement {
    locator: Locator,
}

/// Window content mounted over the deterministic headless backend.
pub struct HeadlessHost {
    runtime: WindowRuntime<HeadlessBackend>,
}

impl HeadlessHost {
    /// Mounts window content with default (fake-free) platform services.
    pub fn mount(content: WindowContent) -> Result<Self, HarnessError> {
        Self::mount_with_services(content, PlatformServices::default())
    }

    /// Mounts window content with injected services (fake clipboard, fake
    /// dialog presenter, and so on).
    pub fn mount_with_services(
        content: WindowContent,
        services: PlatformServices,
    ) -> Result<Self, HarnessError> {
        let runtime = WindowRuntime::mount(
            rinka_core::Renderer::new(HeadlessBackend::new()),
            content,
            services,
        )
        .map_err(|error| HarnessError::Render(error.to_string()))?;
        Ok(Self { runtime })
    }

    /// Finds an element by declarative key.
    pub fn find_by_key(&self, key: &str) -> Result<HeadlessElement, HarnessError> {
        self.find(Locator::Key(key.to_owned()))
    }

    /// Finds an element by accessibility label.
    pub fn find_by_label(&self, label: &str) -> Result<HeadlessElement, HarnessError> {
        self.find(Locator::Label(label.to_owned()))
    }

    fn find(&self, locator: Locator) -> Result<HeadlessElement, HarnessError> {
        self.resolve(&locator, |_| ())?;
        Ok(HeadlessElement { locator })
    }

    fn resolve<R>(
        &self,
        locator: &Locator,
        read: impl FnOnce(&MountedNode<rinka_headless::Handle>) -> R,
    ) -> Result<R, HarnessError> {
        self.runtime
            .with_renderer(|renderer| {
                renderer
                    .mounted()
                    .and_then(|root| find_node(root, locator))
                    .map(read)
            })
            .ok_or_else(|| HarnessError::NotFound {
                locator: locator.clone(),
            })
    }

    fn resolve_for_verb(
        &self,
        element: &HeadlessElement,
        verb: &'static str,
        accepts: impl Fn(ElementKind) -> bool,
    ) -> Result<(EventBindings, ElementKind), HarnessError> {
        let (events, kind) = self.resolve(&element.locator, |node| {
            (node.events().clone(), node.element().kind())
        })?;
        if accepts(kind) {
            Ok((events, kind))
        } else {
            Err(HarnessError::WrongRole {
                locator: element.locator.clone(),
                verb,
                found: kind,
            })
        }
    }

    /// Activates a button or list row through its stable event binding —
    /// the same slot the native adapters' target/action fires.
    pub fn press(&self, element: &HeadlessElement) -> Result<(), HarnessError> {
        let (events, _) = self.resolve_for_verb(element, "press", |kind| {
            matches!(kind, ElementKind::Button | ElementKind::ListRow)
        })?;
        events.emit_activate();
        self.take_render_error()
    }

    /// Sets a toggle's value through its stable event binding.
    pub fn toggle(&self, element: &HeadlessElement, value: bool) -> Result<(), HarnessError> {
        let (events, _) =
            self.resolve_for_verb(element, "toggle", |kind| kind == ElementKind::Toggle)?;
        events.emit_toggle(value);
        self.take_render_error()
    }

    /// Commits edited text into an input through its stable event binding.
    pub fn type_text(&self, element: &HeadlessElement, text: &str) -> Result<(), HarnessError> {
        let (events, _) =
            self.resolve_for_verb(element, "type_text", |kind| kind == ElementKind::Input)?;
        events.emit_input(text);
        self.take_render_error()
    }

    /// Reads the element's current value from its mounted properties.
    pub fn read_value(&self, element: &HeadlessElement) -> Result<String, HarnessError> {
        let (value, kind) = self.resolve(&element.locator, |node| {
            (props_value(node.element().props()), node.element().kind())
        })?;
        value.ok_or(HarnessError::WrongRole {
            locator: element.locator.clone(),
            verb: "read_value",
            found: kind,
        })
    }

    /// Reads an interactive element's enabled state.
    pub fn is_enabled(&self, element: &HeadlessElement) -> Result<bool, HarnessError> {
        let (enabled, kind) = self.resolve(&element.locator, |node| {
            let enabled = match node.element().props() {
                Props::Button { enabled, .. }
                | Props::Input { enabled, .. }
                | Props::Toggle { enabled, .. } => Some(*enabled),
                _ => None,
            };
            (enabled, node.element().kind())
        })?;
        enabled.ok_or(HarnessError::WrongRole {
            locator: element.locator.clone(),
            verb: "is_enabled",
            found: kind,
        })
    }

    /// Reads a toggle's checked state.
    pub fn is_checked(&self, element: &HeadlessElement) -> Result<bool, HarnessError> {
        let (value, kind) = self.resolve(&element.locator, |node| {
            let value = match node.element().props() {
                Props::Toggle { value, .. } => Some(*value),
                _ => None,
            };
            (value, node.element().kind())
        })?;
        value.ok_or(HarnessError::WrongRole {
            locator: element.locator.clone(),
            verb: "is_checked",
            found: kind,
        })
    }

    /// Confirms the runtime settled: no asynchronous reconciliation error is
    /// pending. Headless delivery is synchronous, so this returns after one
    /// check; the live driver's counterpart pumps real main-loop turns.
    pub fn settle(&self) -> Result<(), HarnessError> {
        self.take_render_error()
    }

    /// Retries `predicate` over bounded attempts; on exhaustion the failure
    /// names the unmet condition, matching the live driver's contract.
    pub fn settle_until(
        &self,
        condition: &str,
        mut predicate: impl FnMut(&Self) -> bool,
    ) -> Result<(), HarnessError> {
        for _ in 0..MAX_SETTLE_TURNS {
            self.take_render_error()?;
            if predicate(self) {
                return Ok(());
            }
        }
        Err(HarnessError::SettlementTimeout {
            turns: MAX_SETTLE_TURNS,
            unmet: vec![condition.to_owned()],
        })
    }

    /// Snapshots the mounted element tree (kind, key, accessibility name).
    pub fn tree_snapshot(&self) -> Result<String, HarnessError> {
        self.runtime
            .with_renderer(|renderer| renderer.mounted().map(tree_snapshot))
            .ok_or_else(|| HarnessError::Render("no mounted root".to_owned()))
    }

    /// Returns whether any mounted element matches the locator right now.
    pub fn exists_by_key(&self, key: &str) -> bool {
        self.resolve(&Locator::Key(key.to_owned()), |_| ()).is_ok()
    }

    /// Reads the mounted runtime directly for assertions the verbs do not
    /// cover (recorded operations, drag simulation, dialog scripting).
    pub fn runtime(&self) -> &WindowRuntime<HeadlessBackend> {
        &self.runtime
    }

    fn take_render_error(&self) -> Result<(), HarnessError> {
        match self.runtime.take_error() {
            None => Ok(()),
            Some(error) => Err(HarnessError::Render(error.to_string())),
        }
    }
}

impl std::fmt::Debug for HeadlessHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HeadlessHost")
            .finish_non_exhaustive()
    }
}

/// Extracts the value a native control would report for these properties.
fn props_value(props: &Props) -> Option<String> {
    match props {
        Props::Label { text, .. } => Some(text.clone()),
        Props::Button { label, .. } | Props::Toggle { label, .. } => Some(label.clone()),
        Props::Input { value, .. } => Some(value.clone()),
        Props::TextArea { content, .. } => Some(content.text().to_owned()),
        Props::Progress { fraction, .. } => Some(fraction.to_string()),
        Props::ListRow { title, .. } | Props::Status { title, .. } => Some(title.clone()),
        _ => None,
    }
}
