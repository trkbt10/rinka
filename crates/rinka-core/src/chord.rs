//! Platform-neutral key-chord vocabulary.
//!
//! A chord names a key identity plus a semantic modifier set. The semantic
//! `primary` modifier resolves to Command on macOS and Control on GTK and
//! Windows hosts; the resolution is performed by each platform adapter through
//! [`Modifiers::resolve`], never by spelling platform keys in common code.

use std::error::Error;
use std::fmt;
use std::str::FromStr;

/// Semantic identity of one keyboard key.
///
/// The identity is layout-independent vocabulary, not a scan code: `letter('s')`
/// and `letter('S')` are the same identity, and shifted variants are expressed
/// through [`Modifiers::shift`] on the owning [`KeyChord`].
///
/// `Backspace` is the key that deletes backward (⌫ on macOS) and `Delete` is
/// the key that deletes forward (⌦ on macOS, the Delete key elsewhere). The
/// two are distinct identities so a chord never changes meaning between
/// platforms.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyIdentity(KeyRepr);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum KeyRepr {
    /// ASCII uppercase letter byte.
    Letter(u8),
    /// Decimal digit value in `0..=9`.
    Digit(u8),
    /// Function key number in `1..=24`.
    Function(u8),
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Escape,
    Enter,
    Tab,
    Space,
    Backspace,
    Delete,
    PageUp,
    PageDown,
    Home,
    End,
}

impl KeyIdentity {
    /// Up arrow key.
    pub const ARROW_UP: Self = Self(KeyRepr::ArrowUp);
    /// Down arrow key.
    pub const ARROW_DOWN: Self = Self(KeyRepr::ArrowDown);
    /// Left arrow key.
    pub const ARROW_LEFT: Self = Self(KeyRepr::ArrowLeft);
    /// Right arrow key.
    pub const ARROW_RIGHT: Self = Self(KeyRepr::ArrowRight);
    /// Escape key.
    pub const ESCAPE: Self = Self(KeyRepr::Escape);
    /// Main Enter or Return key.
    pub const ENTER: Self = Self(KeyRepr::Enter);
    /// Tab key.
    pub const TAB: Self = Self(KeyRepr::Tab);
    /// Space bar.
    pub const SPACE: Self = Self(KeyRepr::Space);
    /// Backward delete key (⌫ on macOS keyboards).
    pub const BACKSPACE: Self = Self(KeyRepr::Backspace);
    /// Forward delete key (⌦ on macOS keyboards).
    pub const DELETE: Self = Self(KeyRepr::Delete);
    /// Page Up key.
    pub const PAGE_UP: Self = Self(KeyRepr::PageUp);
    /// Page Down key.
    pub const PAGE_DOWN: Self = Self(KeyRepr::PageDown);
    /// Home key.
    pub const HOME: Self = Self(KeyRepr::Home);
    /// End key.
    pub const END: Self = Self(KeyRepr::End);

    /// Creates a letter identity from an ASCII letter in either case.
    pub const fn letter(value: char) -> Option<Self> {
        if value.is_ascii_alphabetic() {
            Some(Self(KeyRepr::Letter(value.to_ascii_uppercase() as u8)))
        } else {
            None
        }
    }

    /// Creates a digit identity from a value in `0..=9`.
    pub const fn digit(value: u8) -> Option<Self> {
        if value <= 9 {
            Some(Self(KeyRepr::Digit(value)))
        } else {
            None
        }
    }

    /// Creates a function-key identity from a number in `1..=24`.
    pub const fn function(number: u8) -> Option<Self> {
        if matches!(number, 1..=24) {
            Some(Self(KeyRepr::Function(number)))
        } else {
            None
        }
    }

    /// Returns the ASCII uppercase letter, if this identity is a letter.
    pub const fn as_letter(self) -> Option<char> {
        match self.0 {
            KeyRepr::Letter(value) => Some(value as char),
            _ => None,
        }
    }

    /// Returns the digit value, if this identity is a digit.
    pub const fn as_digit(self) -> Option<u8> {
        match self.0 {
            KeyRepr::Digit(value) => Some(value),
            _ => None,
        }
    }

    /// Returns the function-key number, if this identity is a function key.
    pub const fn as_function(self) -> Option<u8> {
        match self.0 {
            KeyRepr::Function(number) => Some(number),
            _ => None,
        }
    }
}

impl fmt::Display for KeyIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            KeyRepr::Letter(value) => write!(formatter, "{}", value as char),
            KeyRepr::Digit(value) => write!(formatter, "{value}"),
            KeyRepr::Function(number) => write!(formatter, "F{number}"),
            KeyRepr::ArrowUp => formatter.write_str("Up"),
            KeyRepr::ArrowDown => formatter.write_str("Down"),
            KeyRepr::ArrowLeft => formatter.write_str("Left"),
            KeyRepr::ArrowRight => formatter.write_str("Right"),
            KeyRepr::Escape => formatter.write_str("Escape"),
            KeyRepr::Enter => formatter.write_str("Enter"),
            KeyRepr::Tab => formatter.write_str("Tab"),
            KeyRepr::Space => formatter.write_str("Space"),
            KeyRepr::Backspace => formatter.write_str("Backspace"),
            KeyRepr::Delete => formatter.write_str("Delete"),
            KeyRepr::PageUp => formatter.write_str("PageUp"),
            KeyRepr::PageDown => formatter.write_str("PageDown"),
            KeyRepr::Home => formatter.write_str("Home"),
            KeyRepr::End => formatter.write_str("End"),
        }
    }
}

fn parse_key_identity(token: &str) -> Option<KeyIdentity> {
    let mut characters = token.chars();
    if let (Some(only), None) = (characters.next(), characters.next()) {
        if let Some(letter) = KeyIdentity::letter(only) {
            return Some(letter);
        }
        if let Some(digit) = only.to_digit(10) {
            return KeyIdentity::digit(digit as u8);
        }
    }
    if let Some(number) = token
        .strip_prefix(['F', 'f'])
        .and_then(|digits| digits.parse::<u8>().ok())
    {
        return KeyIdentity::function(number);
    }
    let named = match token.to_ascii_lowercase().as_str() {
        "up" | "arrowup" => KeyIdentity::ARROW_UP,
        "down" | "arrowdown" => KeyIdentity::ARROW_DOWN,
        "left" | "arrowleft" => KeyIdentity::ARROW_LEFT,
        "right" | "arrowright" => KeyIdentity::ARROW_RIGHT,
        "escape" | "esc" => KeyIdentity::ESCAPE,
        "enter" | "return" => KeyIdentity::ENTER,
        "tab" => KeyIdentity::TAB,
        "space" => KeyIdentity::SPACE,
        "backspace" => KeyIdentity::BACKSPACE,
        "delete" => KeyIdentity::DELETE,
        "pageup" => KeyIdentity::PAGE_UP,
        "pagedown" => KeyIdentity::PAGE_DOWN,
        "home" => KeyIdentity::HOME,
        "end" => KeyIdentity::END,
        _ => return None,
    };
    Some(named)
}

/// Semantic modifier set attached to a key identity.
///
/// `primary` is the platform-neutral command modifier; `control` always means
/// the physical Control key. On hosts whose primary modifier is Control the
/// two resolve to the same physical key, so a chord should declare one of
/// them, not both.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Modifiers {
    /// Platform-neutral primary command modifier.
    pub primary: bool,
    /// Physical Control key on every platform.
    pub control: bool,
    /// Alt on PC keyboards, Option on macOS.
    pub alt: bool,
    /// Shift key.
    pub shift: bool,
}

impl Modifiers {
    /// No modifiers.
    pub const NONE: Self = Self {
        primary: false,
        control: false,
        alt: false,
        shift: false,
    };
    /// Only the semantic primary modifier.
    pub const PRIMARY: Self = Self {
        primary: true,
        control: false,
        alt: false,
        shift: false,
    };

    /// Adds the semantic primary modifier.
    pub const fn with_primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// Adds the physical Control modifier.
    pub const fn with_control(mut self) -> Self {
        self.control = true;
        self
    }

    /// Adds the Alt or Option modifier.
    pub const fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    /// Adds the Shift modifier.
    pub const fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// Resolves the semantic primary modifier into a physical modifier set.
    ///
    /// When the platform's primary modifier is Control, a chord declaring both
    /// `primary` and `control` merges into one Control requirement.
    pub const fn resolve(self, primary: PrimaryModifier) -> ResolvedModifiers {
        ResolvedModifiers {
            command: matches!(primary, PrimaryModifier::Command) && self.primary,
            control: self.control || (matches!(primary, PrimaryModifier::Control) && self.primary),
            alt: self.alt,
            shift: self.shift,
        }
    }
}

/// Physical key the semantic primary modifier resolves to on one platform.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PrimaryModifier {
    /// Command (⌘); the macOS AppKit host.
    Command,
    /// Control; the GTK and Windows hosts.
    Control,
}

/// Physical modifier set produced by [`Modifiers::resolve`].
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ResolvedModifiers {
    /// Command on macOS, Super or the Windows key elsewhere.
    pub command: bool,
    /// Physical Control key.
    pub control: bool,
    /// Alt or Option key.
    pub alt: bool,
    /// Shift key.
    pub shift: bool,
}

/// One declarative key chord: a key identity plus semantic modifiers.
///
/// The canonical text form is the modifier names `Primary`, `Control`, `Alt`,
/// and `Shift` in that order, joined to the key name with `+`, for example
/// `Primary+Shift+H`. [`FromStr`] accepts the tokens case-insensitively and in
/// any order and round-trips with [`fmt::Display`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyChord {
    /// Key identity completing the chord.
    pub key: KeyIdentity,
    /// Semantic modifier set.
    pub modifiers: Modifiers,
}

impl KeyChord {
    /// Creates a chord.
    pub const fn new(key: KeyIdentity, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.primary {
            formatter.write_str("Primary+")?;
        }
        if self.modifiers.control {
            formatter.write_str("Control+")?;
        }
        if self.modifiers.alt {
            formatter.write_str("Alt+")?;
        }
        if self.modifiers.shift {
            formatter.write_str("Shift+")?;
        }
        self.key.fmt(formatter)
    }
}

/// Invalid chord text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChordParseError {
    /// The chord text contains no tokens.
    Empty,
    /// The same modifier appears twice.
    DuplicateModifier {
        /// Repeated modifier name in canonical spelling.
        modifier: String,
    },
    /// A modifier token follows the key token or no key token exists.
    MissingKey,
    /// A token names neither a modifier nor a key.
    UnknownToken {
        /// Unrecognized token text.
        token: String,
    },
}

impl fmt::Display for ChordParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("chord text is empty"),
            Self::DuplicateModifier { modifier } => {
                write!(formatter, "modifier '{modifier}' appears twice")
            }
            Self::MissingKey => formatter.write_str("chord does not end with a key"),
            Self::UnknownToken { token } => {
                write!(formatter, "unknown chord token '{token}'")
            }
        }
    }
}

impl Error for ChordParseError {}

fn parse_modifier(token: &str) -> Option<&'static str> {
    match token.to_ascii_lowercase().as_str() {
        "primary" => Some("Primary"),
        "control" | "ctrl" => Some("Control"),
        "alt" | "option" => Some("Alt"),
        "shift" => Some("Shift"),
        _ => None,
    }
}

impl FromStr for KeyChord {
    type Err = ChordParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let tokens: Vec<&str> = value.split('+').map(str::trim).collect();
        let (&key_token, modifier_tokens) = tokens.split_last().ok_or(ChordParseError::Empty)?;
        if tokens.iter().all(|token| token.is_empty()) {
            return Err(ChordParseError::Empty);
        }
        let mut modifiers = Modifiers::NONE;
        for &token in modifier_tokens {
            let Some(canonical) = parse_modifier(token) else {
                return if parse_key_identity(token).is_some() {
                    Err(ChordParseError::MissingKey)
                } else {
                    Err(ChordParseError::UnknownToken {
                        token: token.to_owned(),
                    })
                };
            };
            let slot = match canonical {
                "Primary" => &mut modifiers.primary,
                "Control" => &mut modifiers.control,
                "Alt" => &mut modifiers.alt,
                _ => &mut modifiers.shift,
            };
            if *slot {
                return Err(ChordParseError::DuplicateModifier {
                    modifier: canonical.to_owned(),
                });
            }
            *slot = true;
        }
        if key_token.is_empty() || parse_modifier(key_token).is_some() {
            return Err(ChordParseError::MissingKey);
        }
        let key = parse_key_identity(key_token).ok_or_else(|| ChordParseError::UnknownToken {
            token: key_token.to_owned(),
        })?;
        Ok(Self { key, modifiers })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChordParseError, KeyChord, KeyIdentity, Modifiers, PrimaryModifier, ResolvedModifiers,
    };

    #[test]
    fn key_identity_constructors_enforce_their_vocabulary() {
        assert_eq!(KeyIdentity::letter('s'), KeyIdentity::letter('S'));
        assert_eq!(KeyIdentity::letter('ß'), None);
        assert_eq!(KeyIdentity::digit(9).unwrap().as_digit(), Some(9));
        assert_eq!(KeyIdentity::digit(10), None);
        assert_eq!(KeyIdentity::function(12).unwrap().as_function(), Some(12));
        assert_eq!(KeyIdentity::function(0), None);
        assert_eq!(KeyIdentity::function(25), None);
    }

    #[test]
    fn canonical_chord_text_round_trips() {
        for text in [
            "Primary+S",
            "Primary+Shift+H",
            "Primary+Control+Alt+Shift+F5",
            "Escape",
            "Alt+Enter",
            "Shift+PageDown",
            "Primary+Backspace",
            "Delete",
            "Control+Home",
            "Primary+2",
        ] {
            let chord: KeyChord = text.parse().unwrap();
            assert_eq!(chord.to_string(), text, "round trip for {text}");
        }
    }

    #[test]
    fn parsing_accepts_aliases_and_any_case_or_order() {
        let canonical: KeyChord = "Primary+Alt+Shift+Up".parse().unwrap();
        for text in [
            "shift+alt+primary+ArrowUp",
            "PRIMARY+OPTION+SHIFT+UP",
            " primary + option + shift + arrowup ",
        ] {
            assert_eq!(text.parse::<KeyChord>().unwrap(), canonical, "{text}");
        }
        assert_eq!(
            "ctrl+return".parse::<KeyChord>().unwrap().to_string(),
            "Control+Enter"
        );
        assert_eq!("esc".parse::<KeyChord>().unwrap().to_string(), "Escape");
    }

    #[test]
    fn invalid_chord_text_is_a_typed_error() {
        assert_eq!("".parse::<KeyChord>(), Err(ChordParseError::Empty));
        assert_eq!("+".parse::<KeyChord>(), Err(ChordParseError::Empty));
        assert_eq!(
            "Primary+Primary+S".parse::<KeyChord>(),
            Err(ChordParseError::DuplicateModifier {
                modifier: "Primary".to_owned(),
            })
        );
        assert_eq!(
            "Primary+Shift".parse::<KeyChord>(),
            Err(ChordParseError::MissingKey)
        );
        assert_eq!(
            "S+Primary".parse::<KeyChord>(),
            Err(ChordParseError::MissingKey)
        );
        assert_eq!(
            "Primary+Hyper".parse::<KeyChord>(),
            Err(ChordParseError::UnknownToken {
                token: "Hyper".to_owned(),
            })
        );
        assert_eq!(
            "F25".parse::<KeyChord>(),
            Err(ChordParseError::UnknownToken {
                token: "F25".to_owned(),
            })
        );
    }

    #[test]
    fn primary_resolves_to_command_on_macos_and_control_elsewhere() {
        let declared = Modifiers::PRIMARY.with_shift();
        assert_eq!(
            declared.resolve(PrimaryModifier::Command),
            ResolvedModifiers {
                command: true,
                control: false,
                alt: false,
                shift: true,
            }
        );
        assert_eq!(
            declared.resolve(PrimaryModifier::Control),
            ResolvedModifiers {
                command: false,
                control: true,
                alt: false,
                shift: true,
            }
        );
    }

    #[test]
    fn explicit_control_merges_with_a_control_primary() {
        let declared = Modifiers::PRIMARY.with_control();
        let resolved = declared.resolve(PrimaryModifier::Control);
        assert!(resolved.control);
        assert!(!resolved.command);
        let on_macos = declared.resolve(PrimaryModifier::Command);
        assert!(on_macos.command);
        assert!(on_macos.control);
    }
}
