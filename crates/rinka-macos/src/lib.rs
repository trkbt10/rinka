//! AppKit native host.

/// Identifies this platform adapter in diagnostics.
pub const PLATFORM_NAME: &str = "macOS AppKit";

#[cfg(target_os = "macos")]
mod platform;

#[cfg(target_os = "macos")]
pub use platform::run;

#[cfg(target_os = "macos")]
pub use platform::{
    AppKitError, AppKitHandle, AppKitTestHost, RealizedControl, SettleObservation,
    realized_control, window_server_session_available,
};

#[cfg(not(target_os = "macos"))]
/// Reports a programming error when invoked on another operating system.
pub fn run(_application: rinka_core::ApplicationSpec) {
    panic!("the AppKit host can run only on macOS");
}
