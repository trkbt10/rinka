impl ApplicationDelegate {
    fn new(mtm: MainThreadMarker, application: ApplicationSpec) -> Retained<Self> {
        let transition_sizes = application.windows.first().map(|spec| {
            (
                Size {
                    width: spec.initial_size.width,
                    height: spec.initial_size.height,
                },
                Size {
                    width: spec.minimum_size.width,
                    height: spec.minimum_size.height,
                },
            )
        });
        let object = Self::alloc(mtm).set_ivars(ApplicationDelegateIvars {
            application: RefCell::new(Some(application)),
            transition_sizes,
            windows: RefCell::new(Vec::new()),
            window_initial_sizes: RefCell::new(Vec::new()),
            window_initial_extent_constraints: RefCell::new(Vec::new()),
            split_window_frames: RefCell::new(Vec::new()),
            renderers: RefCell::new(Vec::new()),
            list_registries: RefCell::new(Vec::new()),
            split_resize_epoch: Cell::new(0),
            split_restore_pending: Rc::new(Cell::new(false)),
            toolbar_delegates: RefCell::new(Vec::new()),
            transition_probe: RefCell::new(None),
            scene_probe: RefCell::new(None),
        });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }

    fn window_index(&self, window: &AnyObject) -> Option<usize> {
        self.ivars()
            .windows
            .borrow()
            .iter()
            .position(|candidate| std::ptr::eq(candidate.as_object(), window))
    }

    unsafe fn set_window_content_extent(&self, window: &AnyObject, content_size: Size) {
        let Some(index) = self.window_index(window) else {
            return;
        };
        if let Some(constraints) = self
            .ivars()
            .window_initial_extent_constraints
            .borrow()
            .get(index)
        {
            if let Some(width) = constraints.first() {
                let _: () =
                    unsafe { msg_send![width.as_object(), setConstant: content_size.width] };
            }
            if let Some(height) = constraints.get(1) {
                let _: () =
                    unsafe { msg_send![height.as_object(), setConstant: content_size.height] };
            }
        }
        let controller: *mut AnyObject = unsafe { msg_send![window, contentViewController] };
        if let Some(controller) = NonNull::new(controller) {
            let _: () =
                unsafe { msg_send![controller.as_ref(), setPreferredContentSize: content_size] };
        }
    }

    fn show_initial_windows(&self) {
        let Some(application) = self.ivars().application.borrow_mut().take() else {
            return;
        };
        // SAFETY: The host enters here only from the process main thread.
        let app: *mut AnyObject =
            unsafe { msg_send![objc2::class!(NSApplication), sharedApplication] };
        // SAFETY: NSApplication setters are main-thread calls. Cargo-launched
        // diagnostics reach this point without a LaunchServices registration;
        // establishing the policy after AppKit launch supports both that path
        // and normal application-bundle launches.
        unsafe {
            let _: bool = msg_send![app, setActivationPolicy: 0_isize];
        }
        // SAFETY: NSApplication implements NSAppearanceCustomization. This
        // diagnostic override is applied before constructing any native view.
        let diagnostic_appearance = unsafe { configure_diagnostic_appearance(&*app) };

        let mut key_window = None;
        for window in application.windows {
            let is_primary = matches!(window.kind, WindowKind::Main | WindowKind::Preferences);
            let initial_size = Size {
                width: window.initial_size.width,
                height: window.initial_size.height,
            };
            match build_window(
                self.mtm(),
                &window,
                self.ivars().split_restore_pending.clone(),
            ) {
                Ok((
                    native_window,
                    renderer,
                    toolbar_delegate,
                    list_registry,
                    initial_extent_constraints,
                )) => {
                    if let Some(requested) = diagnostic_appearance {
                        // SAFETY: The retained object is the NSWindow or NSPanel
                        // just built on AppKit's main thread.
                        unsafe {
                            assert_diagnostic_appearance(native_window.as_object(), requested);
                        }
                    }
                    // SAFETY: NSWindow's delegate is weak. The application
                    // retains this delegate for the complete event loop.
                    unsafe {
                        let _: () = msg_send![native_window.as_object(), setDelegate: &**self];
                    }
                    if is_primary && key_window.is_none() {
                        key_window = Some(native_window.clone());
                    }
                    self.ivars()
                        .list_registries
                        .borrow_mut()
                        .push(list_registry);
                    self.ivars()
                        .toolbar_delegates
                        .borrow_mut()
                        .push(toolbar_delegate);
                    self.ivars().renderers.borrow_mut().push(renderer);
                    self.ivars()
                        .window_initial_sizes
                        .borrow_mut()
                        .push(initial_size);
                    self.ivars()
                        .window_initial_extent_constraints
                        .borrow_mut()
                        .push(initial_extent_constraints);
                    self.ivars().split_window_frames.borrow_mut().push(None);
                    self.ivars().windows.borrow_mut().push(native_window);
                }
                Err(error) => eprintln!("AppKit host error: {error}"),
            }
        }
        // SAFETY: The application delegate lives for the AppKit run loop.
        // Observing native split resize boundaries keeps controlled outline
        // state isolated from pane animation and window-size layout traffic.
        unsafe {
            let center: *mut AnyObject =
                msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![center,
                addObserver: self,
                selector: sel!(splitViewWillResizeSubviews:),
                name: SPLIT_VIEW_WILL_RESIZE_NOTIFICATION,
                object: std::ptr::null::<AnyObject>()
            ];
            let _: () = msg_send![center,
                addObserver: self,
                selector: sel!(splitViewDidResizeSubviews:),
                name: SPLIT_VIEW_DID_RESIZE_NOTIFICATION,
                object: std::ptr::null::<AnyObject>()
            ];
        }
        // SAFETY: Required for a Cargo-launched, unbundled AppKit process.
        unsafe {
            let _: () = msg_send![app, activate];
            if let Some(window) = key_window {
                let _: () = msg_send![window.as_object(), makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
            }
            let _: () = msg_send![self,
                performSelector: sel!(refreshInitialLayout:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.0_f64
            ];
        }
    }

    fn begin_scene_probe(&self) {
        let Some(expected_scene) = std::env::var_os("RINKA_APPKIT_SCENE_PROBE") else {
            return;
        };
        if std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_some() {
            panic!("scene and transition probes must run in separate processes");
        }
        if self.ivars().scene_probe.borrow().is_some() {
            return;
        }
        let expected_scene = expected_scene
            .into_string()
            .unwrap_or_else(|_| panic!("RINKA_APPKIT_SCENE_PROBE must be valid UTF-8"));
        if !matches!(
            expected_scene.as_str(),
            "ready" | "empty" | "busy" | "error" | "canvas"
        ) {
            panic!("RINKA_APPKIT_SCENE_PROBE expects ready, empty, busy, error, or canvas");
        }

        let observed_scene = {
            let renderers = self.ivars().renderers.borrow();
            renderers.first().and_then(|runtime| {
                runtime.with_renderer(|renderer| renderer.mounted().and_then(mounted_scene))
            })
        };
        let windows = self.ivars().windows.borrow();
        let expected_window_count = if expected_scene == "busy" { 2 } else { 1 };
        let scene_passed = observed_scene == Some(expected_scene.as_str());
        let window_count_passed = windows.len() == expected_window_count;
        // SAFETY: The probe only reads retained AppKit windows and views on
        // the main thread after initial layout has completed.
        let geometry_passed = unsafe {
            windows
                .iter()
                .all(|window| window_geometry_is_valid(window.as_object()))
        };
        let visibility_passed = unsafe {
            windows.iter().all(|window| {
                let visible: bool = msg_send![window.as_object(), isVisible];
                let screen: *mut AnyObject = msg_send![window.as_object(), screen];
                visible && !screen.is_null()
            })
        };
        let visibility_required = std::env::var_os("RINKA_APPKIT_WINDOW_LIVE_PROBE").is_some();
        let passed = scene_passed
            && window_count_passed
            && geometry_passed
            && (!visibility_required || visibility_passed);
        eprintln!(
            "Rinka scene probe expected={} observed={} scene_pass={} windows={} expected_windows={} window_count_pass={} geometry_pass={} visibility_required={} visibility_pass={} pass={}",
            expected_scene,
            observed_scene.unwrap_or("unknown"),
            scene_passed,
            windows.len(),
            expected_window_count,
            window_count_passed,
            geometry_passed,
            visibility_required,
            visibility_passed,
            passed
        );

        *self.ivars().scene_probe.borrow_mut() = Some(SceneProbe {
            expected_scene: expected_scene.clone(),
            phase: SceneProbePhase::AwaitingMainWindow,
            attempts: 0,
            requires_live_panel: std::env::var_os("RINKA_APPKIT_PANEL_LIVE_PROBE").is_some(),
            passed,
        });
        if expected_scene == "busy" {
            let requires_live_panel = self
                .ivars()
                .scene_probe
                .borrow()
                .as_ref()
                .is_some_and(|probe| probe.requires_live_panel);
            if !requires_live_panel {
                let panel_contract = windows
                    .get(1)
                    .is_some_and(|panel| unsafe { panel_contract_is_valid(panel.as_object()) });
                drop(windows);
                let stop_reached = self.perform_panel_stop_action();
                eprintln!(
                    "Rinka scene probe panel_static contract={panel_contract} stop_reached={stop_reached} pass={}",
                    panel_contract && stop_reached
                );
                if let Some(probe) = self.ivars().scene_probe.borrow_mut().as_mut() {
                    probe.passed &= panel_contract && stop_reached;
                }
                self.finish_scene_probe();
                return;
            }
            drop(windows);
            self.schedule_scene_probe();
        } else {
            drop(windows);
            self.finish_scene_probe();
        }
    }

    fn advance_scene_probe(&self) {
        let windows = self.ivars().windows.borrow();
        let Some(main) = windows.first() else {
            return;
        };
        let Some(panel) = windows.get(1) else {
            return;
        };
        let phase = self
            .ivars()
            .scene_probe
            .borrow()
            .as_ref()
            .map(|probe| probe.phase);
        if phase == Some(SceneProbePhase::AwaitingMainWindow) {
            // SAFETY: Busy declares one main NSWindow followed by one
            // keyboard-capable floating NSPanel. State is read after at least
            // one main-loop turn so activation can settle without time guesses.
            let (application_active, key_is_main, main_is_main, panel_contract) = unsafe {
                let application: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let application_active: bool = msg_send![application, isActive];
                let key: *mut AnyObject = msg_send![application, keyWindow];
                let main_window: *mut AnyObject = msg_send![application, mainWindow];
                (
                    application_active,
                    key == main.as_ptr(),
                    main_window == main.as_ptr(),
                    panel_contract_is_valid(panel.as_object()),
                )
            };
            if !application_active || !key_is_main || !main_is_main {
                // SAFETY: Activation is idempotent and the retained primary
                // window is the intended key/main window for this phase.
                unsafe {
                    let application: *mut AnyObject =
                        msg_send![objc2::class!(NSApplication), sharedApplication];
                    let _: () = msg_send![application, activate];
                    let _: () = msg_send![main.as_object(),
                        makeKeyAndOrderFront: std::ptr::null::<AnyObject>()
                    ];
                }
                drop(windows);
                if self.retry_scene_probe() {
                    self.schedule_scene_probe();
                } else {
                    self.finish_scene_probe();
                }
                return;
            }
            let initial_passed = panel_contract;
            eprintln!(
                "Rinka scene probe panel_initial application_active={application_active} key_is_main={key_is_main} main_is_main={main_is_main} panel_contract={panel_contract} pass={initial_passed}"
            );
            if let Some(probe) = self.ivars().scene_probe.borrow_mut().as_mut() {
                probe.passed &= initial_passed;
                probe.phase = SceneProbePhase::AwaitingPanelWindow;
                probe.attempts = 0;
            }
            // SAFETY: The retained panel accepts keyboard focus by contract.
            unsafe {
                let _: () = msg_send![panel.as_object(),
                    makeKeyAndOrderFront: std::ptr::null::<AnyObject>()
                ];
            }
            drop(windows);
            self.schedule_scene_probe();
            return;
        }
        // SAFETY: The panel was made key on the preceding main-loop turn.
        let (key_is_panel, main_remains_main, panel_reports_key, main_reports_main) = unsafe {
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let key: *mut AnyObject = msg_send![application, keyWindow];
            let main_window: *mut AnyObject = msg_send![application, mainWindow];
            let panel_is_key: bool = msg_send![panel.as_object(), isKeyWindow];
            let main_is_main: bool = msg_send![main.as_object(), isMainWindow];
            (
                key == panel.as_ptr(),
                main_window == main.as_ptr(),
                panel_is_key,
                main_is_main,
            )
        };
        let panel_passed =
            key_is_panel && main_remains_main && panel_reports_key && main_reports_main;
        if !panel_passed {
            drop(windows);
            if self.retry_scene_probe() {
                self.schedule_scene_probe();
            } else {
                self.finish_scene_probe();
            }
            return;
        }
        eprintln!(
            "Rinka scene probe panel_key key_is_panel={key_is_panel} main_remains_main={main_remains_main} panel_reports_key={panel_reports_key} main_reports_main={main_reports_main} pass={panel_passed}"
        );
        drop(windows);

        let stop_reached = self.perform_panel_stop_action();
        eprintln!("Rinka scene probe panel_stop_reached={stop_reached}");
        if let Some(probe) = self.ivars().scene_probe.borrow_mut().as_mut() {
            probe.passed &= panel_passed && stop_reached;
        }
        self.finish_scene_probe();
    }

    fn perform_panel_stop_action(&self) -> bool {
        {
            let renderers = self.ivars().renderers.borrow();
            renderers.get(1).is_some_and(|runtime| {
                runtime.with_renderer(|renderer| {
                    let Some(root) = renderer.mounted() else {
                        return false;
                    };
                    let Some(handle) = mounted_handle_for_key(root, "cancel-transfer") else {
                        return false;
                    };
                    // SAFETY: The key identifies the native Stop NSButton.
                    unsafe {
                        let _: () = msg_send![handle.view(),
                            performClick: std::ptr::null::<AnyObject>()
                        ];
                    }
                    true
                })
            })
        }
    }

    fn schedule_scene_probe(&self) {
        // SAFETY: The next main-loop turn observes key/main status after AppKit
        // completes panel activation.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runSceneProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.02_f64
            ];
        }
    }

    fn retry_scene_probe(&self) -> bool {
        const MAX_MAIN_LOOP_TURNS: usize = 100;
        let mut probe = self.ivars().scene_probe.borrow_mut();
        let Some(probe) = probe.as_mut() else {
            return false;
        };
        probe.attempts += 1;
        if probe.attempts < MAX_MAIN_LOOP_TURNS {
            return true;
        }
        probe.passed = false;
        eprintln!(
            "Rinka scene probe activation_timeout phase={:?} turns={}",
            probe.phase, probe.attempts
        );
        false
    }

    fn finish_scene_probe(&self) {
        {
            let probe = self.ivars().scene_probe.borrow();
            let Some(probe) = probe.as_ref() else {
                return;
            };
            eprintln!(
                "Rinka scene probe scene={} result={}",
                probe.expected_scene,
                if probe.passed { "PASS" } else { "FAIL" }
            );
        }
        self.capture_windows_to_directory("");
        self.run_pointer_probe();
        if std::env::var_os("RINKA_APPKIT_SCENE_PROBE_HOLD").is_none() {
            // SAFETY: Diagnostic completion terminates only the current test app.
            unsafe {
                let application: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let _: () = msg_send![application,
                    terminate: std::ptr::null::<AnyObject>()
                ];
            }
        }
    }

    /// Writes each retained window's content view into
    /// `RINKA_APPKIT_WINDOW_CAPTURE_DIR` as `<prefix>window-<index>.png`.
    ///
    /// The capture renders the live view hierarchy into a backing-scale
    /// bitmap through AppKit itself, so it needs no screen-recording
    /// permission and records the real drawing output.
    fn capture_windows_to_directory(&self, prefix: &str) {
        let Some(directory) = std::env::var_os("RINKA_APPKIT_WINDOW_CAPTURE_DIR") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        for (index, window) in self.ivars().windows.borrow().iter().enumerate() {
            let path = directory.join(format!("{prefix}window-{index}.png"));
            // SAFETY: The retained NSWindow's content view renders itself on
            // AppKit's main thread inside this call.
            let written = unsafe { write_window_content_png(window.as_object(), &path) };
            eprintln!(
                "Rinka window capture index={index} prefix={prefix} written={written} path={}",
                path.display()
            );
        }
        self.capture_element_to_directory(&directory, prefix);
    }

    /// Writes the mounted element named by `RINKA_APPKIT_ELEMENT_CAPTURE`
    /// into the capture directory as `<prefix>element-<key>.png`.
    fn capture_element_to_directory(&self, directory: &std::path::Path, prefix: &str) {
        let Some(target_key) = std::env::var_os("RINKA_APPKIT_ELEMENT_CAPTURE") else {
            return;
        };
        let target_key = target_key
            .into_string()
            .unwrap_or_else(|_| panic!("RINKA_APPKIT_ELEMENT_CAPTURE must be valid UTF-8"));
        let handle = {
            let renderers = self.ivars().renderers.borrow();
            renderers.first().and_then(|runtime| {
                runtime.with_renderer(|renderer| {
                    renderer
                        .mounted()
                        .and_then(|mounted| mounted_handle_for_key(mounted, &target_key))
                        .cloned()
                })
            })
        };
        let Some(handle) = handle else {
            eprintln!("Rinka element capture key={target_key} written=false reason=not-mounted");
            return;
        };
        let path = directory.join(format!("{prefix}element-{target_key}.png"));
        // SAFETY: The mounted element's view renders itself on AppKit's main
        // thread inside this call.
        let written = unsafe { write_view_png(handle.view(), &path) };
        eprintln!(
            "Rinka element capture key={target_key} prefix={prefix} written={written} path={}",
            path.display()
        );
    }

    /// Sends one synthetic primary-button click through the AppKit event
    /// pipeline at the center of the mounted element named by
    /// `RINKA_APPKIT_POINTER_PROBE`, then captures the windows again with
    /// the `after-pointer-` prefix.
    ///
    /// The events travel through `NSWindow sendEvent:` — real hit testing
    /// and responder dispatch — without touching the user's desktop, screen
    /// pointer, or any process other than this one.
    fn run_pointer_probe(&self) {
        let Some(target_key) = std::env::var_os("RINKA_APPKIT_POINTER_PROBE") else {
            return;
        };
        let target_key = target_key
            .into_string()
            .unwrap_or_else(|_| panic!("RINKA_APPKIT_POINTER_PROBE must be valid UTF-8"));
        let handle = {
            let renderers = self.ivars().renderers.borrow();
            renderers.first().and_then(|runtime| {
                runtime.with_renderer(|renderer| {
                    renderer
                        .mounted()
                        .and_then(|mounted| mounted_handle_for_key(mounted, &target_key))
                        .cloned()
                })
            })
        };
        let Some(handle) = handle else {
            eprintln!("Rinka pointer probe key={target_key} result=FAIL reason=element-not-mounted");
            return;
        };
        // SAFETY: The mounted view, its window, and NSEvent construction are
        // used on AppKit's main thread; sendEvent: performs ordinary event
        // dispatch confined to this application's window.
        let delivered = unsafe {
            let view = handle.view();
            let window: *mut AnyObject = msg_send![view, window];
            if window.is_null() {
                false
            } else {
                let bounds: Rect = msg_send![view, bounds];
                let center = Point {
                    x: bounds.origin.x + bounds.size.width / 2.0,
                    y: bounds.origin.y + bounds.size.height / 2.0,
                };
                let in_window: Point = msg_send![
                    view,
                    convertPoint: center,
                    toView: std::ptr::null::<AnyObject>()
                ];
                let window_number: isize = msg_send![window, windowNumber];
                // NSEventTypeLeftMouseDown = 1, NSEventTypeLeftMouseUp = 2.
                for event_type in [1_usize, 2_usize] {
                    let event: *mut AnyObject = msg_send![objc2::class!(NSEvent),
                        mouseEventWithType: event_type,
                        location: in_window,
                        modifierFlags: 0_usize,
                        timestamp: 0.0_f64,
                        windowNumber: window_number,
                        context: std::ptr::null::<AnyObject>(),
                        eventNumber: 0_isize,
                        clickCount: 1_isize,
                        pressure: 1.0_f32
                    ];
                    let _: () = msg_send![window, sendEvent: event];
                }
                true
            }
        };
        eprintln!(
            "Rinka pointer probe key={target_key} result={}",
            if delivered { "PASS" } else { "FAIL" }
        );
        if delivered {
            self.capture_windows_to_directory("after-pointer-");
        }
    }

    fn begin_transition_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_none()
            || self.ivars().transition_probe.borrow().is_some()
        {
            return;
        }
        let Some(window) = self.ivars().windows.borrow().first().cloned() else {
            return;
        };
        let Some((wide_size, minimum_size)) = self.ivars().transition_sizes else {
            return;
        };
        // SAFETY: The first application window has completed its initial
        // layout and owns the promoted NSSplitViewController.
        unsafe {
            let _: () = msg_send![window.as_object(), setContentSize: wide_size];
            let controller: *mut AnyObject = msg_send![window.as_object(), contentViewController];
            set_split_item_collapsed(controller, 0, false);
            set_split_item_collapsed(controller, 2, false);
            let frame: Rect = msg_send![window.as_object(), frame];
            *self.ivars().transition_probe.borrow_mut() = Some(TransitionProbe {
                step: 0,
                phase: TransitionProbePhase::WidePending,
                baseline: frame,
                wide_size,
                minimum_size,
                attempts: 0,
                observed_split_epoch: self.ivars().split_resize_epoch.get(),
                quiet_turns: 0,
                passed: true,
            });
            self.schedule_transition_probe();
        }
    }

    fn advance_transition_probe(&self) {
        const MAX_SETTLING_TURNS: usize = 200;
        const REQUIRED_QUIET_TURNS: usize = 2;
        let Some(window) = self.ivars().windows.borrow().first().cloned() else {
            return;
        };
        // SAFETY: The probe runs on AppKit's main thread and advances only
        // after the requested split state has actually settled.
        unsafe {
            let controller: *mut AnyObject = msg_send![window.as_object(), contentViewController];
            let frame: Rect = msg_send![window.as_object(), frame];
            let split_resize_epoch = self.ivars().split_resize_epoch.get();
            let split_restore_pending = self.ivars().split_restore_pending.get();
            let outline_state_settled =
                registered_outline_state_is_settled(&self.ivars().list_registries);
            let source_widths = registered_visible_source_widths(&self.ivars().list_registries);
            let action = {
                let mut probe_ref = self.ivars().transition_probe.borrow_mut();
                let Some(probe) = probe_ref.as_mut() else {
                    return;
                };
                if matches!(
                    probe.phase,
                    TransitionProbePhase::WidePending | TransitionProbePhase::MinimumPending
                ) {
                    let target_size = if probe.phase == TransitionProbePhase::WidePending {
                        probe.wide_size
                    } else {
                        probe.minimum_size
                    };
                    let size_matches = rect_size_matches(frame, target_size);
                    let sidebar_collapsed = split_item_collapsed(controller, 0);
                    let inspector_collapsed = split_item_collapsed(controller, 2);
                    let split_is_quiet = probe.observed_split_epoch == split_resize_epoch;
                    if !size_matches
                        || sidebar_collapsed
                        || inspector_collapsed
                        || split_restore_pending
                        || !outline_state_settled
                        || !source_widths.all_widths_resolved
                        || !split_is_quiet
                        || probe.quiet_turns < REQUIRED_QUIET_TURNS
                    {
                        probe.attempts += 1;
                        if split_is_quiet
                            && !split_restore_pending
                            && outline_state_settled
                            && source_widths.all_widths_resolved
                            && size_matches
                            && !sidebar_collapsed
                            && !inspector_collapsed
                        {
                            probe.quiet_turns += 1;
                        } else {
                            probe.observed_split_epoch = split_resize_epoch;
                            probe.quiet_turns = 0;
                        }
                        if probe.attempts >= MAX_SETTLING_TURNS {
                            probe.passed = false;
                            eprintln!(
                                "Rinka transition probe phase={} settlement_timeout frame={frame:?} target_size={target_size:?} sidebar_collapsed={sidebar_collapsed} inspector_collapsed={inspector_collapsed} source_rows_fit={} source_width_resolved={} source_width_capped={}",
                                probe.phase.label(),
                                source_widths.all_rows_fit,
                                source_widths.all_widths_resolved,
                                source_widths.any_width_capped,
                            );
                            TransitionProbeAction::Fail
                        } else if sidebar_collapsed || inspector_collapsed {
                            TransitionProbeAction::RestorePanes
                        } else {
                            TransitionProbeAction::Wait
                        }
                    } else {
                        probe.baseline = frame;
                        probe.step = 0;
                        probe.attempts = 0;
                        probe.quiet_turns = 0;
                        probe.phase = if probe.phase == TransitionProbePhase::WidePending {
                            TransitionProbePhase::Wide
                        } else {
                            TransitionProbePhase::Minimum
                        };
                        eprintln!(
                            "Rinka transition probe phase={} baseline={frame:?} sidebar_collapsed={sidebar_collapsed} inspector_collapsed={inspector_collapsed} pass=true",
                            probe.phase.label()
                        );
                        TransitionProbeAction::ToggleSidebar
                    }
                } else {
                    let Some(expectation) = TRANSITION_PROBE_STEPS.get(probe.step).copied() else {
                        return;
                    };
                    let frame_matches = rect_matches(frame, probe.baseline);
                    let sidebar_collapsed = split_item_collapsed(controller, 0);
                    let inspector_collapsed = split_item_collapsed(controller, 2);
                    let expected_inspector_collapsed = expectation.inspector_collapsed;
                    let state_matches = sidebar_collapsed == expectation.sidebar_collapsed
                        && inspector_collapsed == expected_inspector_collapsed;
                    if !frame_matches {
                        probe.passed = false;
                    }
                    let split_is_quiet = probe.observed_split_epoch == split_resize_epoch;
                    if !state_matches
                        || split_restore_pending
                        || !outline_state_settled
                        || !source_widths.all_widths_resolved
                        || !split_is_quiet
                        || probe.quiet_turns < REQUIRED_QUIET_TURNS
                    {
                        probe.attempts += 1;
                        if split_is_quiet
                            && !split_restore_pending
                            && outline_state_settled
                            && source_widths.all_widths_resolved
                            && state_matches
                        {
                            probe.quiet_turns += 1;
                        } else {
                            probe.observed_split_epoch = split_resize_epoch;
                            probe.quiet_turns = 0;
                        }
                        if probe.attempts >= MAX_SETTLING_TURNS {
                            probe.passed = false;
                            eprintln!(
                                "Rinka transition probe phase={} step={} state={} settlement_timeout expected_sidebar_collapsed={} sidebar_collapsed={} expected_inspector_collapsed={} inspector_collapsed={} frame={frame:?} frame_matches={frame_matches} source_rows_fit={} source_width_resolved={} source_width_capped={}",
                                probe.phase.label(),
                                probe.step,
                                expectation.label,
                                expectation.sidebar_collapsed,
                                sidebar_collapsed,
                                expected_inspector_collapsed,
                                inspector_collapsed,
                                source_widths.all_rows_fit,
                                source_widths.all_widths_resolved,
                                source_widths.any_width_capped,
                            );
                            TransitionProbeAction::Fail
                        } else {
                            TransitionProbeAction::Wait
                        }
                    } else {
                        let step_passed = frame_matches;
                        probe.passed &= step_passed;
                        probe.attempts = 0;
                        probe.quiet_turns = 0;
                        eprintln!(
                            "Rinka transition probe phase={} step={} state={} expected_sidebar_collapsed={} sidebar_collapsed={} expected_inspector_collapsed={} inspector_collapsed={} frame={frame:?} frame_matches={frame_matches} pass={step_passed}",
                            probe.phase.label(),
                            probe.step,
                            expectation.label,
                            expectation.sidebar_collapsed,
                            sidebar_collapsed,
                            expected_inspector_collapsed,
                            inspector_collapsed,
                        );
                        match (probe.phase, expectation.action) {
                            (TransitionProbePhase::Wide, TransitionProbeAction::CompletePhase) => {
                                probe.phase = TransitionProbePhase::MinimumPending;
                                probe.step = 0;
                                TransitionProbeAction::ResizeToMinimum
                            }
                            (
                                TransitionProbePhase::Minimum,
                                TransitionProbeAction::CompletePhase,
                            ) => TransitionProbeAction::Finish,
                            (_, action) => {
                                probe.step += 1;
                                action
                            }
                        }
                    }
                }
            };
            match action {
                TransitionProbeAction::ToggleSidebar => {
                    let _: () = msg_send![controller, toggleSidebar: std::ptr::null::<AnyObject>()];
                    self.schedule_transition_probe();
                }
                TransitionProbeAction::ToggleInspector => {
                    let _: () =
                        msg_send![controller, toggleInspector: std::ptr::null::<AnyObject>()];
                    self.schedule_transition_probe();
                }
                TransitionProbeAction::RestorePanes => {
                    set_split_item_collapsed(controller, 0, false);
                    set_split_item_collapsed(controller, 2, false);
                    self.schedule_transition_probe();
                }
                TransitionProbeAction::ResizeToMinimum => {
                    let minimum = self
                        .ivars()
                        .transition_probe
                        .borrow()
                        .as_ref()
                        .map_or(Size::default(), |probe| probe.minimum_size);
                    self.set_window_content_extent(window.as_object(), minimum);
                    let _: () = msg_send![window.as_object(), setContentSize: minimum];
                    self.schedule_transition_probe();
                }
                TransitionProbeAction::CompletePhase => unreachable!(
                    "phase completion is resolved while the transition probe is borrowed"
                ),
                TransitionProbeAction::Finish | TransitionProbeAction::Fail => {
                    let passed = self
                        .ivars()
                        .transition_probe
                        .borrow()
                        .is_some_and(|probe| probe.passed);
                    eprintln!(
                        "Rinka transition probe result={}",
                        if passed { "PASS" } else { "FAIL" }
                    );
                    if std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE_HOLD").is_none() {
                        let application: *mut AnyObject =
                            msg_send![objc2::class!(NSApplication), sharedApplication];
                        let _: () =
                            msg_send![application, terminate: std::ptr::null::<AnyObject>()];
                    }
                }
                TransitionProbeAction::Wait => self.schedule_transition_probe(),
            }
        }
    }

    fn schedule_transition_probe(&self) {
        // SAFETY: Short main-loop polling observes AppKit's actual native
        // split state instead of assuming an animation duration.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runTransitionProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.02_f64
            ];
        }
    }
}
