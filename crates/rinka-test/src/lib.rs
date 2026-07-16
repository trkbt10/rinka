//! Consumer test harness for rinka applications.
//!
//! `rinka-test` lets a consumer put "the app actually works when operated"
//! into its own gate: mount the real UI, find widgets by accessibility
//! label, click and type, assert the resulting state, and capture real
//! pixels — the workflow egui_kittest-style suites rely on, served by the
//! real platform adapter instead of a simulated one.
//!
//! # The two drivers
//!
//! - [`HeadlessHost`] mounts window content over the deterministic
//!   `rinka-headless` backend. It runs on every platform with no window
//!   server and gives the find/act/settle verbs deterministic semantics.
//! - [`TestHost`] mounts a full [`rinka_core::ApplicationSpec`] against the
//!   live platform adapter **in-process**. On macOS this is the real AppKit
//!   adapter; on GTK and WinUI the live driver is not implemented yet and
//!   every verb returns the typed
//!   [`HarnessError::UnsupportedPlatform`] diagnostic — never a silent
//!   no-op.
//!
//! # How a consumer (for example Overshell) writes a live test
//!
//! Live AppKit tests must own the process main thread, so they live in an
//! integration-test binary with `harness = false`:
//!
//! ```toml
//! [[test]]
//! name = "live_ui"
//! harness = false
//!
//! [dev-dependencies]
//! rinka-test = { path = "../rinka-test" }
//! ```
//!
//! ```no_run
//! use rinka_test::{HarnessError, TestHost};
//!
//! fn main() {
//!     match TestHost::mount(application()) {
//!         Err(HarnessError::NoWindowServerSession) => {
//!             // The typed CI skip: log it, never report it as proof.
//!             eprintln!("rinka-test skip reason=no-window-server-session");
//!         }
//!         Err(error) => panic!("mount failed: {error}"),
//!         Ok(host) => {
//!             // Find by accessibility label (in-process, no TCC grant),
//!             // act through the real native control, settle, and assert.
//!             let row = host.find_by_label("Home").expect("sidebar row");
//!             host.select_row(&row).expect("row selection dispatches");
//!             host.settle_until("home listing is shown", |host| {
//!                 host.exists_by_label("Files in Home")
//!             })
//!             .expect("the table follows the sidebar selection");
//!
//!             let field = host.find_by_label("Filter files").expect("field");
//!             host.type_text(&field, "Cargo").expect("typing");
//!             host.commit_text(&field).expect("commit");
//!             assert_eq!(host.read_value(&field).expect("value"), "Cargo");
//!
//!             // Real pixels, verified decodable and non-trivial.
//!             let (width, height) = host
//!                 .capture_window_png(0, std::path::Path::new("/tmp/w.png"))
//!                 .expect("capture");
//!             assert!(width > 0 && height > 0);
//!         }
//!     }
//! }
//! # fn application() -> rinka_core::ApplicationSpec { unimplemented!() }
//! ```
//!
//! # CI windowed-session policy
//!
//! Headless CI cannot host AppKit. [`TestHost::mount`] therefore checks for
//! a window-server session first and fails with the typed
//! [`HarnessError::NoWindowServerSession`] when there is none. A gate
//! runner must log that reason and skip — a skip is visible and typed,
//! and it is never proof that the UI works. Everything the live driver
//! does stays inside the test process: synthetic events are dispatched
//! through the application's own windows, finding walks the mounted
//! element tree, and captures render in-process, so no screen-recording or
//! accessibility (TCC) permission is requested and no other application on
//! the desktop is touched.
//!
//! # Settlement
//!
//! No verb sleeps for an arbitrary duration. [`TestHost::settle`] pumps the
//! real run loop in bounded turns over the adapter's named conditions
//! (split-restore idle, controlled outlines settled, source widths
//! resolved, split epoch quiet), and [`TestHost::settle_until`] does the
//! same for a named consumer condition; on timeout the
//! [`HarnessError::SettlementTimeout`] failure names every condition still
//! unmet.

mod capture;
mod error;
mod headless;
mod live;
mod query;

pub use capture::png_dimensions;
pub use error::HarnessError;
pub use headless::{HeadlessElement, HeadlessHost};
pub use live::{ChordModifiers, ElementHandle, TestHost};
pub use query::{Locator, find_node, tree_snapshot};
