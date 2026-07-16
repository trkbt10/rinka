//! Focus, raw-key, and IME composition contracts for input-accepting
//! canvases.
//!
//! A canvas that declares [`crate::Element::accepts_input`] participates in
//! keyboard focus and receives two complementary streams:
//!
//! - **Raw keys** ([`KeyEvent`]): every key-down the platform's input method
//!   did not consume into a composition, carrying the layout-independent
//!   [`KeyIdentity`]/[`Modifiers`] vocabulary shared with
//!   [`crate::KeyChord`], the text the key produced (already translated by
//!   the platform, so dead-key results and shifted characters arrive
//!   correct), and the key-repeat flag a terminal needs.
//! - **Composition** ([`ImeEvent`]): the operating-system input method's
//!   preedit updates, commits, and cancellations. The application owns
//!   rendering the preedit inside the canvas (it alone knows its cell grid);
//!   the platform adapter owns the OS protocol, and the OS owns the
//!   candidate window, anchored at the rectangle the application declares
//!   through [`crate::Element::ime_caret`].
//!
//! A key-down consumed by an active or beginning composition produces only
//! [`ImeEvent`]s, never a duplicate [`KeyEvent`]: otherwise the Return that
//! commits a Japanese composition would also arrive as a raw Enter and a
//! terminal would forward a spurious newline. Keys outside composition
//! produce exactly one [`KeyEvent`].

use crate::chord::{KeyChord, KeyIdentity, Modifiers};
use std::fmt;

/// One raw key-down delivered to a focused input-accepting canvas.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    /// Layout-independent key identity, shared with [`crate::KeyChord`].
    ///
    /// `None` when the pressed key is outside the chord vocabulary (for
    /// example punctuation); [`KeyEvent::text`] still carries what the key
    /// produced.
    pub key: Option<KeyIdentity>,
    /// Semantic modifier set held during the key-down; the platform's
    /// command key arrives as [`Modifiers::primary`], exactly as in
    /// accelerator routing.
    pub modifiers: Modifiers,
    /// Text this key produced after platform translation, `None` for keys
    /// that produce none (arrows, function keys, modified chords).
    pub text: Option<String>,
    /// Whether the event is a key-repeat of a held key.
    pub repeat: bool,
}

impl fmt::Display for KeyEvent {
    /// Formats the chord-like reading used by diagnostics and probes:
    /// the canonical chord text when the key has an identity, the produced
    /// text otherwise, with a ` repeat` suffix for held keys.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.key, self.text.as_deref()) {
            (Some(key), _) => KeyChord::new(key, self.modifiers).fmt(formatter)?,
            (None, Some(text)) => write!(formatter, "{text:?}")?,
            (None, None) => formatter.write_str("unidentified")?,
        }
        if self.repeat {
            formatter.write_str(" repeat")?;
        }
        Ok(())
    }
}

/// Selected range within the preedit text, in Unicode scalar values.
///
/// `start == end` describes a plain caret position; a wider range marks the
/// composition segment the input method currently highlights. Adapters
/// convert their native units (UTF-16 code units on macOS) into scalar
/// offsets so applications never see a platform encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreeditCaret {
    /// First selected scalar offset within the preedit.
    pub start: usize,
    /// One past the last selected scalar offset within the preedit.
    pub end: usize,
}

impl PreeditCaret {
    /// Creates a caret span.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// One IME composition event delivered to a focused input-accepting canvas.
///
/// A composition begins with its first [`ImeEvent::Preedit`] and ends with
/// either [`ImeEvent::Commit`] or [`ImeEvent::Cancel`]; both terminators
/// imply the preedit is cleared, so no empty `Preedit` follows them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImeEvent {
    /// The preedit text was replaced.
    Preedit {
        /// Complete current preedit text.
        text: String,
        /// Caret or highlighted segment within the preedit, when the input
        /// method reported one.
        caret: Option<PreeditCaret>,
    },
    /// The composition committed text; the preedit is cleared.
    Commit {
        /// Text to insert at the application's insertion point.
        text: String,
    },
    /// The composition was abandoned without committing; the preedit is
    /// cleared.
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::{ImeEvent, KeyEvent, PreeditCaret};
    use crate::chord::{KeyIdentity, Modifiers};

    #[test]
    fn key_events_read_like_chords_with_text_and_repeat_variants() {
        let plain = KeyEvent {
            key: KeyIdentity::letter('h'),
            modifiers: Modifiers::NONE,
            text: Some("h".to_owned()),
            repeat: false,
        };
        assert_eq!(plain.to_string(), "H");

        let chorded = KeyEvent {
            key: KeyIdentity::letter('c'),
            modifiers: Modifiers::NONE.with_control(),
            text: None,
            repeat: false,
        };
        assert_eq!(chorded.to_string(), "Control+C");

        let repeated = KeyEvent {
            key: Some(KeyIdentity::ARROW_RIGHT),
            modifiers: Modifiers::NONE,
            text: None,
            repeat: true,
        };
        assert_eq!(repeated.to_string(), "Right repeat");

        let outside_vocabulary = KeyEvent {
            key: None,
            modifiers: Modifiers::NONE,
            text: Some(";".to_owned()),
            repeat: false,
        };
        assert_eq!(outside_vocabulary.to_string(), "\";\"");
    }

    #[test]
    fn composition_events_compare_by_value() {
        let preedit = ImeEvent::Preedit {
            text: "にほんご".to_owned(),
            caret: Some(PreeditCaret::new(4, 4)),
        };
        assert_eq!(
            preedit,
            ImeEvent::Preedit {
                text: "にほんご".to_owned(),
                caret: Some(PreeditCaret::new(4, 4)),
            }
        );
        assert_ne!(
            preedit,
            ImeEvent::Commit {
                text: "日本語".to_owned(),
            }
        );
        assert_ne!(ImeEvent::Cancel, preedit);
    }
}
