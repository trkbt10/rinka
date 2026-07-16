//! AppKit application lifecycle, window hosting, and diagnostic probes.

use super::*;

include!("application/probes.rs");
include!("application/context_menu_probe.rs");
include!("application/delegate.rs");
include!("application/textarea_probe.rs");
include!("application/window_host.rs");
include!("application/run.rs");
