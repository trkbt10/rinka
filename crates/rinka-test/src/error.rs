//! Typed harness diagnostics.

use crate::query::Locator;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

/// Diagnostic raised by the consumer test harness.
///
/// Every driver failure is a typed value in the `TreeError`/`RenderError`
/// style: an unsupported platform, a missing window-server session, and a
/// settlement timeout are each distinguishable by the consumer, never a
/// silent no-op or an untyped panic.
#[derive(Debug)]
pub enum HarnessError {
    /// The live driver verb is not implemented for this platform's adapter
    /// yet. The headless driver remains available everywhere.
    UnsupportedPlatform {
        /// The platform whose adapter lacks the live driver.
        platform: &'static str,
        /// The driver verb that was requested.
        verb: &'static str,
    },
    /// The process has no window-server session, so the platform cannot
    /// host live windows (headless CI, SSH daemon contexts).
    ///
    /// This is the typed skip reason the gate policy requires: a runner
    /// logs it and skips, never reporting the skip as proof.
    NoWindowServerSession,
    /// No mounted element matched the locator.
    NotFound {
        /// The key or accessibility label that failed to match.
        locator: Locator,
    },
    /// The located element cannot serve the requested verb.
    WrongRole {
        /// The locator that resolved the element.
        locator: Locator,
        /// The requested driver verb.
        verb: &'static str,
        /// The element kind actually found.
        found: rinka_core::ElementKind,
    },
    /// The settlement wait exhausted its bounded turns; `unmet` names every
    /// condition still failing on the final turn.
    SettlementTimeout {
        /// Number of main-loop turns granted before giving up.
        turns: usize,
        /// The named conditions that never settled.
        unmet: Vec<String>,
    },
    /// Reconciliation reported an asynchronous error while the harness was
    /// driving the application.
    Render(String),
    /// The platform adapter rejected an operation.
    Platform(String),
    /// A capture could not be written or decoded.
    Capture {
        /// The requested output path.
        path: PathBuf,
        /// Why the capture failed.
        reason: String,
    },
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform { platform, verb } => write!(
                formatter,
                "the live '{verb}' driver is not implemented for the {platform} adapter yet"
            ),
            Self::NoWindowServerSession => formatter.write_str(
                "no window-server session is available; live AppKit tests require a \
                 windowed login session (typed skip, not proof)",
            ),
            Self::NotFound { locator } => {
                write!(formatter, "no mounted element matches {locator}")
            }
            Self::WrongRole {
                locator,
                verb,
                found,
            } => write!(
                formatter,
                "{locator} resolved a {found:?} element, which cannot serve '{verb}'"
            ),
            Self::SettlementTimeout { turns, unmet } => write!(
                formatter,
                "settlement timed out after {turns} turns; unmet conditions: {}",
                unmet.join(", ")
            ),
            Self::Render(reason) => write!(formatter, "reconciliation failed: {reason}"),
            Self::Platform(reason) => write!(formatter, "platform adapter error: {reason}"),
            Self::Capture { path, reason } => {
                write!(formatter, "capture failed for {}: {reason}", path.display())
            }
        }
    }
}

impl Error for HarnessError {}
