//! Stages the Windows App SDK runtime for the Fluent shell proof.

#[cfg(target_os = "windows")]
fn main() {
    windows_reactor_setup::as_self_contained();
}

#[cfg(not(target_os = "windows"))]
fn main() {}
