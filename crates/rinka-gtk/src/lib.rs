//! GTK 4 and libadwaita native host.

/// Identifies this platform adapter in diagnostics.
pub const PLATFORM_NAME: &str = "GTK 4 + libadwaita";

#[cfg(target_os = "linux")]
mod platform;

#[cfg(target_os = "linux")]
pub use platform::run;

#[cfg(not(target_os = "linux"))]
/// Runs a native GTK application and returns its process status.
pub fn run(_application: rinka_core::ApplicationSpec) -> i32 {
    panic!("the GTK host can run only on Linux");
}
