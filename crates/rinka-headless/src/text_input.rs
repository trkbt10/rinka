//! Deterministic synthetic key and IME streams for canvas text-input tests.
//!
//! The driver speaks through the same stable [`EventBindings`] a platform
//! adapter uses, so a test exercises exactly the surface a native host
//! exercises: focus changes, raw key-downs, and IME composition sequences
//! arrive at the component as ordinary dispatched messages. Every sequence
//! is explicit data — no timers, no platform state — so a composition test
//! replays identically on every run.

use rinka_core::{EventBindings, ImeEvent, KeyEvent, PreeditCaret};

/// Synthetic text-input source over one canvas's stable event binding.
#[derive(Clone, Debug)]
pub struct SyntheticTextInput {
    events: EventBindings,
}

impl SyntheticTextInput {
    /// Wraps the stable event binding of one mounted input-accepting canvas.
    pub fn new(events: EventBindings) -> Self {
        Self { events }
    }

    /// Delivers focus-in, as a platform does when the canvas becomes the
    /// focused text input.
    pub fn focus(&self) {
        self.events.emit_focus(true);
    }

    /// Delivers focus-out.
    pub fn blur(&self) {
        self.events.emit_focus(false);
    }

    /// Delivers one raw key-down.
    pub fn key(&self, event: KeyEvent) {
        self.events.emit_key(event);
    }

    /// Delivers one IME composition event.
    pub fn ime(&self, event: ImeEvent) {
        self.events.emit_ime(event);
    }

    /// Replays a composition that ends in a commit: one
    /// [`ImeEvent::Preedit`] per step in order, then [`ImeEvent::Commit`]
    /// with `commit` — the begin → update → commit shape of a real input
    /// method.
    pub fn compose_and_commit(&self, preedit_steps: &[(&str, Option<PreeditCaret>)], commit: &str) {
        self.replay_preedits(preedit_steps);
        self.events.emit_ime(ImeEvent::Commit {
            text: commit.to_owned(),
        });
    }

    /// Replays a composition that is abandoned: one [`ImeEvent::Preedit`]
    /// per step in order, then [`ImeEvent::Cancel`] — the begin → cancel
    /// shape of an escaped composition.
    pub fn compose_and_cancel(&self, preedit_steps: &[(&str, Option<PreeditCaret>)]) {
        self.replay_preedits(preedit_steps);
        self.events.emit_ime(ImeEvent::Cancel);
    }

    fn replay_preedits(&self, preedit_steps: &[(&str, Option<PreeditCaret>)]) {
        for (text, caret) in preedit_steps {
            self.events.emit_ime(ImeEvent::Preedit {
                text: (*text).to_owned(),
                caret: *caret,
            });
        }
    }
}
