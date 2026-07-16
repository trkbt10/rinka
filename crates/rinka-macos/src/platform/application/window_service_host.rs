// Runtime window lifecycle backed by the application delegate.
//
// The service holds only a weak Objective-C reference: the delegate retains
// every window runtime, each runtime retains its content's service registry,
// and the registry holds this service — a strong reference here would cycle
// the entire application graph. A request arriving after the delegate is
// gone (application teardown) answers with a typed host error instead of
// dereferencing a dead object.
//
// Every method runs on AppKit's main thread by construction: window services
// are reached only from component updates, and the delegate drives every
// component update from the main run loop.

/// [`WindowService`] implementation forwarding to the application delegate.
struct AppKitWindowService {
    delegate: ObjcWeak<ApplicationDelegate>,
}

impl AppKitWindowService {
    fn new(delegate: &ApplicationDelegate) -> Self {
        Self {
            delegate: ObjcWeak::new(delegate),
        }
    }

    fn with_delegate<R>(
        &self,
        request: impl FnOnce(&ApplicationDelegate) -> Result<R, WindowError>,
    ) -> Result<R, WindowError> {
        match self.delegate.load() {
            Some(delegate) => request(&delegate),
            None => Err(WindowError::Host {
                reason: "the AppKit application delegate is no longer alive".to_owned(),
            }),
        }
    }
}

impl WindowService for AppKitWindowService {
    fn open(&self, window: rinka_core::WindowSpec) -> Result<(), WindowError> {
        self.with_delegate(move |delegate| delegate.open_runtime_window(window))
    }

    fn close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.with_delegate(|delegate| delegate.close_runtime_window(id))
    }

    fn focus(&self, id: &WindowId) -> Result<(), WindowError> {
        self.with_delegate(|delegate| delegate.focus_runtime_window(id))
    }

    fn set_content_size(&self, id: &WindowId, size: LogicalSize) -> Result<(), WindowError> {
        self.with_delegate(|delegate| delegate.set_runtime_window_content_size(id, size))
    }

    fn set_position(&self, id: &WindowId, position: WindowPosition) -> Result<(), WindowError> {
        self.with_delegate(|delegate| delegate.set_runtime_window_position(id, position))
    }

    fn confirm_close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.with_delegate(|delegate| delegate.confirm_runtime_window_close(id))
    }

    fn veto_close(&self, id: &WindowId) -> Result<(), WindowError> {
        self.with_delegate(|delegate| delegate.veto_runtime_window_close(id))
    }
}

/// Maps the declared last-window policy onto AppKit's terminate decision.
///
/// The AppKit convention keeps an application running (menu bar, Dock) after
/// its windows close, so [`LastWindowClosedPolicy::PlatformDefault`] and
/// [`LastWindowClosedPolicy::StayRunning`] both answer `false`; only an
/// explicit [`LastWindowClosedPolicy::Exit`] terminates.
const fn terminate_after_last_window(policy: LastWindowClosedPolicy) -> bool {
    matches!(policy, LastWindowClosedPolicy::Exit)
}
