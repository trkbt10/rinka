// Accelerator delivery through one application-local NSEvent key monitor.
//
// Delivery decision, recorded for `reports/keyboard-shortcuts-and-key-events`:
// the host installs a single `addLocalMonitorForEventsMatchingMask:` monitor
// for key-down events instead of NSWindow `performKeyEquivalent:` overrides.
// The monitor sees unmodified chords (function keys, arrows) that AppKit
// never routes through the key-equivalent chain, needs no NSWindow subclass,
// and is installed exactly once per application — the stable-connection
// discipline. The monitor only collects platform facts (chord, key window,
// first-responder text focus); the precedence policy itself lives in
// `rinka_core::AcceleratorRouter`, so returning the event unchanged lets a
// focused field editor keep its typing. Menu-bar key equivalents remain the
// HIG-preferred home once `reports/app-menu-bar` lands; `ToolbarAction::chord`
// already carries the display data for menu items.

/// `NSEventModifierFlagShift`.
const NS_EVENT_MODIFIER_SHIFT: usize = 1 << 17;
/// `NSEventModifierFlagControl`.
const NS_EVENT_MODIFIER_CONTROL: usize = 1 << 18;
/// `NSEventModifierFlagOption`.
const NS_EVENT_MODIFIER_OPTION: usize = 1 << 19;
/// `NSEventModifierFlagCommand`.
const NS_EVENT_MODIFIER_COMMAND: usize = 1 << 20;
/// `NSEventMaskKeyDown` (`1 << NSEventTypeKeyDown`).
const NS_EVENT_MASK_KEY_DOWN: usize = 1 << 10;

/// First Unicode code point of the AppKit function-key range (`NSF1FunctionKey`).
const FUNCTION_KEY_BASE: u32 = 0xF704;

/// Maps the resolved chord modifiers onto `NSEvent` modifier-flag bits.
fn native_modifier_flags(modifiers: Modifiers) -> usize {
    let resolved = modifiers.resolve(PrimaryModifier::Command);
    let mut flags = 0;
    if resolved.command {
        flags |= NS_EVENT_MODIFIER_COMMAND;
    }
    if resolved.control {
        flags |= NS_EVENT_MODIFIER_CONTROL;
    }
    if resolved.alt {
        flags |= NS_EVENT_MODIFIER_OPTION;
    }
    if resolved.shift {
        flags |= NS_EVENT_MODIFIER_SHIFT;
    }
    flags
}

/// Reads the semantic modifier set back from `NSEvent` modifier flags.
///
/// Command maps to the semantic primary modifier — the inverse of
/// [`Modifiers::resolve`] with [`PrimaryModifier::Command`] — so declared
/// chords compare by plain equality. Caps Lock, Fn, and the numeric-pad flag
/// are ignored deliberately: they do not change a chord's meaning.
fn semantic_modifiers(flags: usize) -> Modifiers {
    Modifiers {
        primary: flags & NS_EVENT_MODIFIER_COMMAND != 0,
        control: flags & NS_EVENT_MODIFIER_CONTROL != 0,
        alt: flags & NS_EVENT_MODIFIER_OPTION != 0,
        shift: flags & NS_EVENT_MODIFIER_SHIFT != 0,
    }
}

/// Maps one key-down event onto the declarative chord vocabulary.
///
/// `characters` is the event's `charactersIgnoringModifiers`; `key_code` is
/// consulted only for the digit row, whose shifted characters are punctuation
/// on every layout. Keys outside the vocabulary return `None` so the event
/// falls through to native handling untouched.
fn chord_from_key_event(characters: &str, key_code: u16, flags: usize) -> Option<KeyChord> {
    let key = key_identity_from_characters(characters)
        .or_else(|| digit_identity_from_key_code(key_code))?;
    Some(KeyChord::new(key, semantic_modifiers(flags)))
}

fn key_identity_from_characters(characters: &str) -> Option<KeyIdentity> {
    let mut sequence = characters.chars();
    let first = sequence.next()?;
    if sequence.next().is_some() {
        return None;
    }
    if first.is_ascii_alphabetic() {
        return KeyIdentity::letter(first);
    }
    if let Some(digit) = first.to_digit(10) {
        return KeyIdentity::digit(u8::try_from(digit).expect("decimal digit fits in u8"));
    }
    let named = match first {
        '\u{1b}' => KeyIdentity::ESCAPE,
        // The Return key sends CR; the keypad Enter key sends NSEnterCharacter.
        '\r' | '\u{3}' => KeyIdentity::ENTER,
        // Shift+Tab arrives as NSBackTabCharacter; the shift flag carries the
        // distinction, so both spellings are the Tab identity.
        '\t' | '\u{19}' => KeyIdentity::TAB,
        ' ' => KeyIdentity::SPACE,
        // The backward-delete key sends DEL; menu key equivalents use BS.
        '\u{7f}' | '\u{8}' => KeyIdentity::BACKSPACE,
        '\u{f728}' => KeyIdentity::DELETE,
        '\u{f700}' => KeyIdentity::ARROW_UP,
        '\u{f701}' => KeyIdentity::ARROW_DOWN,
        '\u{f702}' => KeyIdentity::ARROW_LEFT,
        '\u{f703}' => KeyIdentity::ARROW_RIGHT,
        '\u{f729}' => KeyIdentity::HOME,
        '\u{f72b}' => KeyIdentity::END,
        '\u{f72c}' => KeyIdentity::PAGE_UP,
        '\u{f72d}' => KeyIdentity::PAGE_DOWN,
        function if (FUNCTION_KEY_BASE..FUNCTION_KEY_BASE + 24).contains(&(function as u32)) => {
            let number = function as u32 - FUNCTION_KEY_BASE + 1;
            return KeyIdentity::function(u8::try_from(number).expect("F1..=F24 fits in u8"));
        }
        _ => return None,
    };
    Some(named)
}

/// ANSI digit-row virtual key codes, consulted when Shift replaced the
/// event character with punctuation.
const fn digit_identity_from_key_code(key_code: u16) -> Option<KeyIdentity> {
    let digit = match key_code {
        29 => 0,
        18 => 1,
        19 => 2,
        20 => 3,
        21 => 4,
        23 => 5,
        22 => 6,
        26 => 7,
        28 => 8,
        25 => 9,
        _ => return None,
    };
    KeyIdentity::digit(digit)
}

/// Returns the `keyEquivalent` string a native menu item displays for a key.
///
/// Letters are lowercase with Shift expressed through the modifier mask;
/// named keys use the AppKit function-key code points and control characters
/// that NSMenuItem renders as key-glyphs.
fn key_equivalent_text(key: KeyIdentity) -> String {
    if let Some(letter) = key.as_letter() {
        return letter.to_ascii_lowercase().to_string();
    }
    if let Some(digit) = key.as_digit() {
        return digit.to_string();
    }
    if let Some(function) = key.as_function() {
        let code_point = FUNCTION_KEY_BASE + u32::from(function) - 1;
        return char::from_u32(code_point)
            .expect("AppKit function-key range is valid Unicode")
            .to_string();
    }
    let named = if key == KeyIdentity::ESCAPE {
        "\u{1b}"
    } else if key == KeyIdentity::ENTER {
        "\r"
    } else if key == KeyIdentity::TAB {
        "\t"
    } else if key == KeyIdentity::SPACE {
        " "
    } else if key == KeyIdentity::BACKSPACE {
        "\u{8}"
    } else if key == KeyIdentity::DELETE {
        "\u{f728}"
    } else if key == KeyIdentity::ARROW_UP {
        "\u{f700}"
    } else if key == KeyIdentity::ARROW_DOWN {
        "\u{f701}"
    } else if key == KeyIdentity::ARROW_LEFT {
        "\u{f702}"
    } else if key == KeyIdentity::ARROW_RIGHT {
        "\u{f703}"
    } else if key == KeyIdentity::HOME {
        "\u{f729}"
    } else if key == KeyIdentity::END {
        "\u{f72b}"
    } else if key == KeyIdentity::PAGE_UP {
        "\u{f72c}"
    } else {
        "\u{f72d}"
    };
    named.to_owned()
}

/// Applies a declared chord to a native menu item as its key equivalent.
///
/// AppKit encodes Shift for letter keys through the character case of the
/// key equivalent, not through the modifier mask: a lowercase letter with
/// `NSEventModifierFlagShift` in the mask is matched inconsistently — the
/// installed menu bar honored the mask, but a view-hosted menu (a toolbar
/// item's NSMenu) matched the same declaration with Shift ignored, so a
/// `Primary+Shift+N` toolbar entry swallowed plain `Primary+N` whenever its
/// window was key (found by the window-lifecycle probe,
/// `reports/dynamic-window-management`). Letters therefore uppercase the
/// equivalent and drop Shift from the mask — Apple's own convention — while
/// keys without a case (digits, arrows, function keys) keep Shift in the
/// mask, which AppKit honors uniformly for them.
fn apply_menu_item_chord(menu_item: &AnyObject, chord: KeyChord) {
    let mut text = key_equivalent_text(chord.key);
    let mut mask = native_modifier_flags(chord.modifiers);
    if chord.modifiers.shift && chord.key.as_letter().is_some() {
        text = text.to_ascii_uppercase();
        mask &= !NS_EVENT_MODIFIER_SHIFT;
    }
    let key_equivalent = ns_string(&text);
    // SAFETY: The receiver is a retained NSMenuItem on the main thread and
    // both properties are public AppKit API.
    unsafe {
        let _: () = msg_send![menu_item, setKeyEquivalent: key_equivalent.as_object()];
        let _: () = msg_send![menu_item, setKeyEquivalentModifierMask: mask];
    }
}

/// Native window pointers paired with their declarative identities.
type WindowIdentityRegistry = Rc<RefCell<Vec<(usize, WindowId)>>>;

/// Returns whether the window's first responder is editable native text.
///
/// A focused NSTextField or NSSearchField edits through the window's field
/// editor, an NSTextView; NSText covers the field editor and every text view.
/// Non-editable text (a selectable label's field editor) does not receive
/// typing, so accelerators stay deliverable there.
///
/// Routing decision, recorded for `reports/canvas-text-input`: a focused
/// input-accepting canvas counts as editable text for the
/// [`KeyRoutingContext`] fact, so window-scoped accelerators defer to a
/// focused terminal-style surface exactly as they defer to a focused text
/// field, and only entries declared global fire over it. A withheld chord
/// falls through to the canvas as a raw key event.
unsafe fn first_responder_is_text_input(window: &AnyObject) -> bool {
    // SAFETY: The caller supplies a live NSWindow on the main thread and the
    // responder is only inspected through public API after each
    // class-membership check.
    let responder: *mut AnyObject = unsafe { msg_send![window, firstResponder] };
    let Some(responder) = NonNull::new(responder) else {
        return false;
    };
    let is_canvas: bool =
        unsafe { msg_send![responder.as_ref(), isKindOfClass: CanvasView::class()] };
    if is_canvas {
        // SAFETY: Class membership was verified above and the reference is
        // used only within this main-thread call.
        let canvas = unsafe { &*responder.as_ptr().cast::<CanvasView>() };
        return canvas.ivars().accepts_input.get();
    }
    let is_text: bool =
        unsafe { msg_send![responder.as_ref(), isKindOfClass: objc2::class!(NSText)] };
    if !is_text {
        return false;
    }
    unsafe { msg_send![responder.as_ref(), isEditable] }
}

/// Installs the application's single key-down monitor and returns its token.
///
/// The token must stay retained for the application lifetime; the menu bar,
/// the router, and the registry are consulted live on every event, so
/// reconciliation changes entries without this connection ever being remade.
///
/// Precedence: a chord the effective menu bar claims is returned untouched —
/// AppKit's native menu key-equivalent dispatch fires the menu item exactly
/// once (shadowing any same-chord accelerator entry) — and only unclaimed
/// chords are routed through the accelerator tables.
fn install_accelerator_monitor(
    router: Rc<RefCell<AcceleratorRouter>>,
    registry: WindowIdentityRegistry,
    menu_bar: MenuBarHost,
) -> Id {
    // The probes assert routing outcomes from the log, so each routed chord
    // is reported with the focus fact it was routed under. The text-input
    // probe reads the same lines to prove a focused canvas counts as text
    // input for withholding.
    let log_outcomes = std::env::var_os("RINKA_APPKIT_ACCELERATOR_PROBE").is_some()
        || std::env::var_os("RINKA_APPKIT_TEXT_INPUT_PROBE").is_some()
        || std::env::var_os("RINKA_APPKIT_MENU_BAR_PROBE").is_some();
    let handler = block2::RcBlock::new(move |event: *mut AnyObject| -> *mut AnyObject {
        // SAFETY: AppKit invokes local monitors on the main thread with a
        // live NSEvent matching the requested key-down mask.
        unsafe {
            let flags: usize = msg_send![&*event, modifierFlags];
            let key_code: u16 = msg_send![&*event, keyCode];
            let characters: *mut AnyObject = msg_send![&*event, charactersIgnoringModifiers];
            let characters = rust_string(characters);
            let Some(chord) = chord_from_key_event(&characters, key_code, flags) else {
                return event;
            };
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let key_window: *mut AnyObject = msg_send![application, keyWindow];
            let context = KeyRoutingContext {
                key_window: registry.borrow().iter().find_map(|(pointer, id)| {
                    (*pointer == key_window as usize).then(|| id.clone())
                }),
                text_input_focused: NonNull::new(key_window)
                    .is_some_and(|window| first_responder_is_text_input(window.as_ref())),
            };
            if menu_bar.claims_chord(context.key_window.as_ref(), chord) {
                if log_outcomes {
                    eprintln!(
                        "Rinka accelerator event chord={chord} text_focus={} outcome=menu-bar-claimed",
                        context.text_input_focused
                    );
                }
                return event;
            }
            // Resolution and invocation are split so the router borrow is
            // released before the action runs: an action may open or close
            // windows, which registers and unregisters tables on this router.
            let (outcome, action) = router.borrow().resolve(chord, &context);
            if let Some(action) = action {
                action();
            }
            if log_outcomes {
                let outcome_text = match &outcome {
                    AcceleratorOutcome::Dispatched {
                        window,
                        accelerator,
                    } => format!("dispatched window={} id={accelerator}", window.as_str()),
                    AcceleratorOutcome::WithheldForTextInput {
                        window,
                        accelerator,
                    } => format!("withheld window={} id={accelerator}", window.as_str()),
                    AcceleratorOutcome::Unmatched => "unmatched".to_owned(),
                };
                eprintln!(
                    "Rinka accelerator event chord={chord} text_focus={} outcome={outcome_text}",
                    context.text_input_focused
                );
            }
            match outcome {
                AcceleratorOutcome::Dispatched { .. } => std::ptr::null_mut(),
                AcceleratorOutcome::WithheldForTextInput { .. }
                | AcceleratorOutcome::Unmatched => event,
            }
        }
    });
    // SAFETY: addLocalMonitorForEventsMatchingMask copies the handler block
    // and returns an autoreleased monitor token; the host balances the
    // borrow with a retain and keeps the token alive for the run loop.
    unsafe {
        let monitor: *mut AnyObject = msg_send![objc2::class!(NSEvent),
            addLocalMonitorForEventsMatchingMask: NS_EVENT_MASK_KEY_DOWN,
            handler: &*handler
        ];
        Id::from_borrowed(monitor)
    }
}

#[cfg(test)]
mod key_dispatch_tests {
    use super::{
        NS_EVENT_MODIFIER_COMMAND, NS_EVENT_MODIFIER_CONTROL, NS_EVENT_MODIFIER_OPTION,
        NS_EVENT_MODIFIER_SHIFT, chord_from_key_event, key_equivalent_text, native_modifier_flags,
    };
    use rinka_core::{KeyChord, KeyIdentity};

    fn chord(text: &str) -> KeyChord {
        text.parse().expect("test chord")
    }

    #[test]
    fn the_semantic_primary_modifier_resolves_to_command_flags() {
        assert_eq!(
            native_modifier_flags(chord("Primary+S").modifiers),
            NS_EVENT_MODIFIER_COMMAND
        );
        assert_eq!(
            native_modifier_flags(chord("Primary+Control+Alt+Shift+S").modifiers),
            NS_EVENT_MODIFIER_COMMAND
                | NS_EVENT_MODIFIER_CONTROL
                | NS_EVENT_MODIFIER_OPTION
                | NS_EVENT_MODIFIER_SHIFT
        );
    }

    #[test]
    fn key_events_map_back_to_declared_chords() {
        assert_eq!(
            chord_from_key_event("s", 1, NS_EVENT_MODIFIER_COMMAND),
            Some(chord("Primary+S"))
        );
        // Shift produces an uppercase letter in charactersIgnoringModifiers.
        assert_eq!(
            chord_from_key_event("H", 4, NS_EVENT_MODIFIER_COMMAND | NS_EVENT_MODIFIER_SHIFT),
            Some(chord("Primary+Shift+H"))
        );
        // Function keys arrive as AppKit function-key code points, often with
        // extra non-modifier flags that must not change the chord.
        assert_eq!(
            chord_from_key_event("\u{f708}", 96, 1 << 23),
            Some(chord("F5"))
        );
        assert_eq!(
            chord_from_key_event("\u{f700}", 126, NS_EVENT_MODIFIER_OPTION),
            Some(chord("Alt+Up"))
        );
        assert_eq!(
            chord_from_key_event("\u{1b}", 53, 0),
            Some(chord("Escape"))
        );
        assert_eq!(
            chord_from_key_event("\u{7f}", 51, NS_EVENT_MODIFIER_COMMAND),
            Some(chord("Primary+Backspace"))
        );
    }

    #[test]
    fn shifted_digit_rows_resolve_through_ansi_key_codes() {
        // Shift+2 produces '@' on a US layout; the key code recovers digit 2.
        assert_eq!(
            chord_from_key_event("@", 19, NS_EVENT_MODIFIER_COMMAND | NS_EVENT_MODIFIER_SHIFT),
            Some(chord("Primary+Shift+2"))
        );
        assert_eq!(
            chord_from_key_event("2", 19, NS_EVENT_MODIFIER_COMMAND),
            Some(chord("Primary+2"))
        );
    }

    #[test]
    fn keys_outside_the_vocabulary_fall_through() {
        assert_eq!(chord_from_key_event(";", 41, NS_EVENT_MODIFIER_COMMAND), None);
        assert_eq!(chord_from_key_event("", 255, 0), None);
    }

    #[test]
    fn menu_key_equivalents_use_lowercase_letters_and_function_code_points() {
        assert_eq!(
            key_equivalent_text(KeyIdentity::letter('N').expect("letter")),
            "n"
        );
        assert_eq!(
            key_equivalent_text(KeyIdentity::function(5).expect("function key")),
            "\u{f708}"
        );
        assert_eq!(key_equivalent_text(KeyIdentity::BACKSPACE), "\u{8}");
    }
}
