//! Headless class assertion for the AppKit input controls.
//!
//! Proves the adapter realizes `InputKind::Secure` as a concealing
//! `NSSecureTextField`, `InputKind::Text` as a plain `NSTextField`, and
//! `InputKind::Search` as an `NSSearchField`, and that the secure control's
//! controlled value round-trips through the native control.
//!
//! This binary declares `harness = false`: it realizes real AppKit controls,
//! which AppKit forbids off the process main thread, so it owns `main`. It
//! mounts no window and needs no window-server session, so it runs in the
//! ordinary `make test` gate even on a locked or headless desktop. A
//! non-macOS build logs a typed skip and exits successfully.

use std::process::ExitCode;

#[cfg(target_os = "macos")]
fn main() -> ExitCode {
    use rinka_core::{InputKind, input};
    use rinka_macos::realized_control;

    let class_cases: [(&str, InputKind, &str); 3] = [
        ("secure", InputKind::Secure, "NSSecureTextField"),
        ("text", InputKind::Text, "NSTextField"),
        ("search", InputKind::Search, "NSSearchField"),
    ];

    for (name, kind, expected_class) in class_cases {
        let control = match realized_control(&input(
            "hunter2",
            "Password",
            kind,
            "Password field",
            |_| {},
        )) {
            Ok(control) => control,
            Err(error) => {
                eprintln!("rinka-macos secure-input gate result=FAIL case={name} error={error}");
                return ExitCode::FAILURE;
            }
        };
        if control.class_name != expected_class {
            eprintln!(
                "rinka-macos secure-input gate result=FAIL case={name} \
                 expected_class={expected_class} realized_class={}",
                control.class_name
            );
            return ExitCode::FAILURE;
        }
        eprintln!(
            "rinka-macos secure-input gate case={name} realized_class={} pass=true",
            control.class_name
        );
    }

    // The concealed control's controlled value must round-trip through the
    // native NSSecureTextField (setStringValue: / stringValue), proving the
    // get/set path carries to the subclass — concealment is a display property
    // only, so the real value is still readable.
    let secure = match realized_control(&input(
        "hunter2",
        "Password",
        InputKind::Secure,
        "Password field",
        |_| {},
    )) {
        Ok(control) => control,
        Err(error) => {
            eprintln!(
                "rinka-macos secure-input gate result=FAIL case=value-roundtrip error={error}"
            );
            return ExitCode::FAILURE;
        }
    };
    if secure.string_value.as_deref() != Some("hunter2") {
        eprintln!(
            "rinka-macos secure-input gate result=FAIL case=value-roundtrip \
             expected_value=hunter2 realized_value={:?}",
            secure.string_value
        );
        return ExitCode::FAILURE;
    }
    eprintln!("rinka-macos secure-input gate case=value-roundtrip pass=true");

    eprintln!("rinka-macos secure-input gate result=PASS cases=4");
    ExitCode::SUCCESS
}

#[cfg(not(target_os = "macos"))]
fn main() -> ExitCode {
    eprintln!("rinka-macos secure-input gate skip reason=not-macos");
    eprintln!("rinka-macos secure-input gate result=SKIP");
    ExitCode::SUCCESS
}
