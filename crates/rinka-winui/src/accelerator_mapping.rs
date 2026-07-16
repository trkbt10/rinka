//! Chord-to-KeyboardAccelerator mapping for the accelerator contract.
//!
//! The WinUI host's primary modifier is Control, so one declaration written
//! with the semantic primary modifier resolves to Control here and to
//! Command on macOS. The produced values are the `Windows.System.VirtualKey`
//! and `Windows.System.VirtualKeyModifiers` numeric identities a
//! `KeyboardAccelerator` declares; the host-side integration that consumes
//! them is tracked in `reports/keyboard-shortcuts-and-key-events`.

use rinka_core::{KeyChord, KeyIdentity, PrimaryModifier};

/// `VirtualKeyModifiers.Control`.
pub const MODIFIER_CONTROL: u32 = 1;
/// `VirtualKeyModifiers.Menu` (Alt).
pub const MODIFIER_MENU: u32 = 2;
/// `VirtualKeyModifiers.Shift`.
pub const MODIFIER_SHIFT: u32 = 4;
/// `VirtualKeyModifiers.Windows`.
pub const MODIFIER_WINDOWS: u32 = 8;

/// One chord resolved into `KeyboardAccelerator` identities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyboardAccelerator {
    /// `Windows.System.VirtualKey` value.
    pub key: u32,
    /// `Windows.System.VirtualKeyModifiers` combination.
    pub modifiers: u32,
}

/// Returns the `KeyboardAccelerator` identities for one declarative chord.
pub fn keyboard_accelerator(chord: KeyChord) -> KeyboardAccelerator {
    let resolved = chord.modifiers.resolve(PrimaryModifier::Control);
    let mut modifiers = 0;
    if resolved.command {
        // Unreachable through a Control-primary resolution; kept so the
        // mapping stays total over ResolvedModifiers.
        modifiers |= MODIFIER_WINDOWS;
    }
    if resolved.control {
        modifiers |= MODIFIER_CONTROL;
    }
    if resolved.alt {
        modifiers |= MODIFIER_MENU;
    }
    if resolved.shift {
        modifiers |= MODIFIER_SHIFT;
    }
    KeyboardAccelerator {
        key: virtual_key(chord.key),
        modifiers,
    }
}

/// Returns the `VirtualKey` value of one key identity.
fn virtual_key(key: KeyIdentity) -> u32 {
    if let Some(letter) = key.as_letter() {
        // VirtualKey.A..Z equal the uppercase ASCII letters.
        return u32::from(letter as u8);
    }
    if let Some(digit) = key.as_digit() {
        // VirtualKey.Number0..Number9 equal the ASCII digits.
        return u32::from(b'0' + digit);
    }
    if let Some(function) = key.as_function() {
        // VirtualKey.F1 is 0x70.
        return 0x70 + u32::from(function) - 1;
    }
    if key == KeyIdentity::ESCAPE {
        0x1B
    } else if key == KeyIdentity::ENTER {
        0x0D
    } else if key == KeyIdentity::TAB {
        0x09
    } else if key == KeyIdentity::SPACE {
        0x20
    } else if key == KeyIdentity::BACKSPACE {
        0x08
    } else if key == KeyIdentity::DELETE {
        0x2E
    } else if key == KeyIdentity::ARROW_UP {
        0x26
    } else if key == KeyIdentity::ARROW_DOWN {
        0x28
    } else if key == KeyIdentity::ARROW_LEFT {
        0x25
    } else if key == KeyIdentity::ARROW_RIGHT {
        0x27
    } else if key == KeyIdentity::HOME {
        0x24
    } else if key == KeyIdentity::END {
        0x23
    } else if key == KeyIdentity::PAGE_UP {
        0x21
    } else {
        0x22
    }
}

#[cfg(test)]
mod tests {
    use super::{
        KeyboardAccelerator, MODIFIER_CONTROL, MODIFIER_MENU, MODIFIER_SHIFT, keyboard_accelerator,
    };
    use rinka_core::KeyChord;

    fn chord(text: &str) -> KeyChord {
        text.parse().expect("test chord")
    }

    #[test]
    fn the_semantic_primary_modifier_resolves_to_control() {
        assert_eq!(
            keyboard_accelerator(chord("Primary+S")),
            KeyboardAccelerator {
                key: u32::from(b'S'),
                modifiers: MODIFIER_CONTROL,
            }
        );
        assert_eq!(
            keyboard_accelerator(chord("Primary+Shift+H")),
            KeyboardAccelerator {
                key: u32::from(b'H'),
                modifiers: MODIFIER_CONTROL | MODIFIER_SHIFT,
            }
        );
    }

    #[test]
    fn explicit_control_merges_with_the_control_primary() {
        assert_eq!(
            keyboard_accelerator(chord("Primary+Control+K")).modifiers,
            MODIFIER_CONTROL
        );
    }

    #[test]
    fn named_keys_use_virtual_key_values() {
        assert_eq!(keyboard_accelerator(chord("F5")).key, 0x74);
        assert_eq!(
            keyboard_accelerator(chord("Alt+Enter")),
            KeyboardAccelerator {
                key: 0x0D,
                modifiers: MODIFIER_MENU,
            }
        );
        assert_eq!(keyboard_accelerator(chord("Shift+PageDown")).key, 0x22);
        assert_eq!(
            keyboard_accelerator(chord("Primary+2")).key,
            u32::from(b'2')
        );
        assert_eq!(keyboard_accelerator(chord("Delete")).key, 0x2E);
    }
}
