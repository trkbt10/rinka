//! AppKit application lifecycle, window hosting, and diagnostic probes.

use super::*;

include!("application/probes.rs");
include!("application/context_menu_probe.rs");
include!("application/dock_probe.rs");
include!("application/drag_drop_probe.rs");
include!("application/menu_bar_probe.rs");
include!("application/delegate.rs");
include!("application/textarea_probe.rs");
include!("application/window_host.rs");
include!("application/window_service_host.rs");
include!("application/dialog_host.rs");
include!("application/dialog_probe.rs");
include!("application/window_lifecycle_probe.rs");
include!("application/run.rs");
include!("application/testing.rs");
