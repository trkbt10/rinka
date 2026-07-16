//! AppKit application lifecycle, window hosting, and diagnostic probes.

use super::*;

include!("application/probes.rs");
include!("application/delegate.rs");
include!("application/window_host.rs");
include!("application/run.rs");
