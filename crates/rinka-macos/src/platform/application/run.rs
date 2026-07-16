/// Runs the application on AppKit's main loop.
pub fn run(application: ApplicationSpec) {
    autoreleasepool(|_| {
        let mtm = MainThreadMarker::new().expect("AppKit must start on the process main thread");
        // SAFETY: sharedApplication is the AppKit singleton on the main thread.
        let app: *mut AnyObject =
            unsafe { msg_send![objc2::class!(NSApplication), sharedApplication] };
        let delegate = ApplicationDelegate::new(mtm, application);
        // SAFETY: NSApplication keeps a non-owning delegate pointer; the retained
        // local lives until run returns. AppKit must complete launch and state
        // restoration before application-owned windows are constructed.
        unsafe {
            let _: () = msg_send![app, setDelegate: &*delegate];
            let _: () = msg_send![app, finishLaunching];
        }
        if let Err(exception) = objc2::exception::catch(AssertUnwindSafe(|| {
            delegate.show_initial_windows();
        })) {
            panic!("AppKit rejected the native view tree: {exception:?}");
        }
        unsafe {
            let _: () = msg_send![app, run];
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        DiagnosticAppearance, TRANSITION_PROBE_STEPS, TransitionProbeAction,
        parse_diagnostic_appearance, should_install_toolbar,
    };
    use rinka_core::{PanelBehavior, WindowKind};

    #[test]
    fn workspace_split_controls_install_a_toolbar_without_custom_items() {
        assert!(should_install_toolbar(WindowKind::Main, 0, true));
        assert!(!should_install_toolbar(WindowKind::Main, 0, false));
    }

    #[test]
    fn panels_do_not_install_a_toolbar() {
        assert!(!should_install_toolbar(
            WindowKind::Panel(PanelBehavior {
                floating: true,
                hides_when_inactive: false,
                accepts_keyboard: true,
            }),
            1,
            true,
        ));
    }

    #[test]
    fn transition_probe_covers_three_combined_cycles_per_size_phase() {
        assert_eq!(TRANSITION_PROBE_STEPS.len(), 24);
        assert_eq!(
            TRANSITION_PROBE_STEPS
                .iter()
                .filter(|step| step.sidebar_collapsed && step.inspector_collapsed)
                .count(),
            3
        );
        assert_eq!(
            TRANSITION_PROBE_STEPS.last().unwrap().action,
            TransitionProbeAction::CompletePhase
        );
        assert!(
            TRANSITION_PROBE_STEPS
                .last()
                .is_some_and(|step| { !step.sidebar_collapsed && !step.inspector_collapsed })
        );
    }

    #[test]
    fn diagnostic_appearance_accepts_only_explicit_matrix_values() {
        assert_eq!(parse_diagnostic_appearance(None).unwrap(), None);
        assert_eq!(
            parse_diagnostic_appearance(Some("light")).unwrap(),
            Some(DiagnosticAppearance::Light)
        );
        assert_eq!(
            parse_diagnostic_appearance(Some("dark")).unwrap(),
            Some(DiagnosticAppearance::Dark)
        );
        assert!(parse_diagnostic_appearance(Some("system")).is_err());
    }
}
