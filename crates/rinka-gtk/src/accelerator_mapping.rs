//! Chord-to-GTK-trigger mapping for the accelerator contract.
//!
//! The GTK host's primary modifier is Control, so one declaration written
//! with the semantic primary modifier resolves to `<Control>` here and to
//! Command on macOS. The produced string is the `gtk_accelerator_parse`
//! syntax consumed by `GtkShortcutTrigger`; the host-side
//! `GtkShortcutController` integration that consumes it is tracked in
//! `reports/keyboard-shortcuts-and-key-events`.

use rinka_core::{KeyChord, KeyIdentity, PrimaryModifier};

/// Returns the GTK accelerator string for one declarative chord.
pub fn shortcut_trigger(chord: KeyChord) -> String {
    let resolved = chord.modifiers.resolve(PrimaryModifier::Control);
    let mut trigger = String::new();
    // The GTK host has no Command key; `resolved.command` can never be set
    // by a Control-primary resolution.
    if resolved.control {
        trigger.push_str("<Control>");
    }
    if resolved.alt {
        trigger.push_str("<Alt>");
    }
    if resolved.shift {
        trigger.push_str("<Shift>");
    }
    trigger.push_str(&key_name(chord.key));
    trigger
}

/// Returns the GDK key name of one key identity.
fn key_name(key: KeyIdentity) -> String {
    if let Some(letter) = key.as_letter() {
        return letter.to_ascii_lowercase().to_string();
    }
    if let Some(digit) = key.as_digit() {
        return digit.to_string();
    }
    if let Some(function) = key.as_function() {
        return format!("F{function}");
    }
    let named = if key == KeyIdentity::ESCAPE {
        "Escape"
    } else if key == KeyIdentity::ENTER {
        "Return"
    } else if key == KeyIdentity::TAB {
        "Tab"
    } else if key == KeyIdentity::SPACE {
        "space"
    } else if key == KeyIdentity::BACKSPACE {
        "BackSpace"
    } else if key == KeyIdentity::DELETE {
        "Delete"
    } else if key == KeyIdentity::ARROW_UP {
        "Up"
    } else if key == KeyIdentity::ARROW_DOWN {
        "Down"
    } else if key == KeyIdentity::ARROW_LEFT {
        "Left"
    } else if key == KeyIdentity::ARROW_RIGHT {
        "Right"
    } else if key == KeyIdentity::HOME {
        "Home"
    } else if key == KeyIdentity::END {
        "End"
    } else if key == KeyIdentity::PAGE_UP {
        "Page_Up"
    } else {
        "Page_Down"
    };
    named.to_owned()
}

#[cfg(test)]
mod tests {
    use super::shortcut_trigger;
    use rinka_core::KeyChord;

    fn chord(text: &str) -> KeyChord {
        text.parse().expect("test chord")
    }

    #[test]
    fn the_semantic_primary_modifier_resolves_to_control() {
        assert_eq!(shortcut_trigger(chord("Primary+S")), "<Control>s");
        assert_eq!(
            shortcut_trigger(chord("Primary+Shift+H")),
            "<Control><Shift>h"
        );
    }

    #[test]
    fn explicit_control_merges_with_the_control_primary() {
        assert_eq!(shortcut_trigger(chord("Primary+Control+K")), "<Control>k");
    }

    #[test]
    fn named_keys_use_gdk_key_names() {
        assert_eq!(shortcut_trigger(chord("F5")), "F5");
        assert_eq!(shortcut_trigger(chord("Alt+Enter")), "<Alt>Return");
        assert_eq!(
            shortcut_trigger(chord("Shift+PageDown")),
            "<Shift>Page_Down"
        );
        assert_eq!(
            shortcut_trigger(chord("Primary+Backspace")),
            "<Control>BackSpace"
        );
        assert_eq!(shortcut_trigger(chord("Primary+2")), "<Control>2");
        assert_eq!(shortcut_trigger(chord("Space")), "space");
    }
}
