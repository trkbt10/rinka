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
        let window_identities: WindowIdentityRegistry = Rc::new(RefCell::new(Vec::new()));
        let menu_bar_host = MenuBarHost::new(
            mtm,
            application.name.clone(),
            application.menu_bar.clone(),
            window_identities.clone(),
        );
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
            accelerator_router: Rc::new(RefCell::new(AcceleratorRouter::new())),
            window_identities,
            menu_bar_host,
            key_monitor: RefCell::new(None),
            transition_probe: RefCell::new(None),
            scene_probe: RefCell::new(None),
            accelerator_probe: RefCell::new(None),
            clipboard_probe: RefCell::new(None),
            text_area_probe: RefCell::new(None),
            dialog_probe: RefCell::new(None),
            text_input_probe: RefCell::new(None),
            menu_bar_probe: RefCell::new(None),
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
                    // The renderer owns one stable accelerator table for the
                    // window's lifetime; registering it here connects the
                    // application key monitor exactly once per window while
                    // reconciliation keeps replacing the entries in place.
                    let accelerator_bindings = renderer
                        .with_renderer(|renderer| renderer.accelerator_bindings().clone());
                    self.ivars()
                        .accelerator_router
                        .borrow_mut()
                        .register_window(window.id.clone(), accelerator_bindings);
                    // The same discipline serves the menu bar: one stable
                    // slot per window, replaced in place on every render,
                    // and a per-window reconciled hook so the installed
                    // NSMenu reflects the freshest declaration.
                    let menu_bar_bindings =
                        renderer.with_renderer(|renderer| renderer.menu_bar_bindings().clone());
                    self.ivars()
                        .menu_bar_host
                        .register_window(window.id.clone(), menu_bar_bindings);
                    let menu_bar_host = self.ivars().menu_bar_host.clone();
                    renderer.set_reconciled_handler(move || menu_bar_host.refresh());
                    self.ivars()
                        .window_identities
                        .borrow_mut()
                        .push((native_window.as_ptr() as usize, window.id.clone()));
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
        // One application-local key monitor delivers every declared
        // accelerator; the router and the per-window tables it consults are
        // updated in place, so this connection is never remade. Chords the
        // effective menu bar claims pass through to native menu dispatch.
        let monitor = install_accelerator_monitor(
            self.ivars().accelerator_router.clone(),
            self.ivars().window_identities.clone(),
            self.ivars().menu_bar_host.clone(),
        );
        *self.ivars().key_monitor.borrow_mut() = Some(monitor);
        // Install the effective menu bar for the initial declarations; the
        // key-window delegate callback keeps it following focus afterwards.
        self.ivars().menu_bar_host.refresh();
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
            "ready" | "empty" | "busy" | "error" | "canvas" | "editor"
        ) {
            panic!("RINKA_APPKIT_SCENE_PROBE expects ready, empty, busy, error, canvas, or editor");
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

    /// Writes rendered evidence into `RINKA_APPKIT_WINDOW_CAPTURE_DIR`:
    /// each retained window's content view as `<prefix>window-<index>.png`,
    /// the element named by `RINKA_APPKIT_ELEMENT_CAPTURE` as
    /// `<prefix>element-<key>.png`, and — when
    /// `RINKA_APPKIT_VIEW_CAPTURE_KEYS` lists declarative keys — each keyed
    /// mounted view as `<prefix>view-<key>.png`.
    ///
    /// The render is in-process (`cacheDisplayInRect:toBitmapImageRep:` at
    /// the backing scale), so it needs no screen-recording permission and
    /// records the real drawing output. Vibrancy-backed surfaces (sidebar
    /// and inspector panes) do not render faithfully offscreen; keyed view
    /// captures target plain view subtrees, which do.
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
        self.capture_keyed_views_to_directory(&directory, prefix);
    }

    /// Writes each mounted view listed in `RINKA_APPKIT_VIEW_CAPTURE_KEYS`
    /// (comma-separated declarative keys) into the capture directory as
    /// `<prefix>view-<key>.png`.
    ///
    /// Keyed view captures exist because vibrancy-backed panes do not render
    /// faithfully offscreen; a plain view subtree does, so evidence targets
    /// the mounted element directly.
    fn capture_keyed_views_to_directory(&self, directory: &std::path::Path, prefix: &str) {
        let Some(keys) = std::env::var_os("RINKA_APPKIT_VIEW_CAPTURE_KEYS") else {
            return;
        };
        let keys = keys
            .into_string()
            .unwrap_or_else(|_| panic!("RINKA_APPKIT_VIEW_CAPTURE_KEYS must be valid UTF-8"));
        let renderers = self.ivars().renderers.borrow();
        for key in keys.split(',').map(str::trim).filter(|key| !key.is_empty()) {
            let captured = renderers.first().is_some_and(|runtime| {
                runtime.with_renderer(|renderer| {
                    let Some(root) = renderer.mounted() else {
                        return false;
                    };
                    let Some(handle) = mounted_handle_for_key(root, key) else {
                        return false;
                    };
                    // SAFETY: The mounted handle owns a live NSView rendered
                    // on AppKit's main thread.
                    unsafe {
                        write_view_capture(
                            handle.view(),
                            &directory.join(format!("{prefix}view-{key}.png")),
                        )
                    }
                })
            });
            if !captured {
                eprintln!("Rinka view capture failed key={key}");
            }
        }
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

    fn begin_accelerator_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_ACCELERATOR_PROBE").is_none()
            || self.ivars().accelerator_probe.borrow().is_some()
        {
            return;
        }
        if std::env::var_os("RINKA_APPKIT_SCENE_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_some()
        {
            panic!("the accelerator probe must run in its own process");
        }
        *self.ivars().accelerator_probe.borrow_mut() = Some(AcceleratorProbe {
            step: 0,
            attempts: 0,
            passed: true,
        });
        self.schedule_accelerator_probe();
    }

    fn observed_probe_scene(&self) -> Option<&'static str> {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().and_then(|runtime| {
            runtime.with_renderer(|renderer| renderer.mounted().and_then(mounted_scene))
        })
    }

    /// Returns whether the primary window is key, re-requesting activation
    /// otherwise. Cargo-launched diagnostics can lose activation to other
    /// processes starting concurrently; window-scoped chords and menu key
    /// equivalents require key status, so the probes never post before it is
    /// established. Cooperative activation refuses a plain `activate` from a
    /// background-launched process while the user's application is frontmost,
    /// so the probe escalates to the ignoring-other-apps request — confined
    /// to diagnostic runs, which own the desktop for their bounded duration.
    fn probe_window_is_key(&self) -> bool {
        let Some(window) = self.ivars().windows.borrow().first().cloned() else {
            return false;
        };
        // SAFETY: Reading key status and re-requesting activation for the
        // retained primary window are main-thread NSApplication calls.
        unsafe {
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let active: bool = msg_send![application, isActive];
            let key: *mut AnyObject = msg_send![application, keyWindow];
            if std::env::var_os("RINKA_APPKIT_PROBE_DEBUG").is_some() {
                eprintln!(
                    "Rinka probe debug active={active} key={key:?} window={:?}",
                    window.as_ptr()
                );
            }
            if active && key == window.as_ptr() {
                return true;
            }
            let _: () = msg_send![application, activate];
            let _: () = msg_send![application, activateIgnoringOtherApps: true];
            let _: () = msg_send![window.as_object(),
                makeKeyAndOrderFront: std::ptr::null::<AnyObject>()
            ];
        }
        false
    }

    fn fail_accelerator_probe_step(&self, step: &'static str, detail: &str) {
        eprintln!("Rinka accelerator probe step={step} {detail} pass=false");
        if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut() {
            probe.passed = false;
        }
        self.finish_accelerator_probe();
    }

    /// Waits until the mounted scene matches, retrying across main-loop
    /// turns; `Some(true)` reports the settled match, `Some(false)` the
    /// timeout, and `None` requests another turn.
    fn await_probe_scene(&self, step: &'static str, expected: &'static str) -> Option<bool> {
        const MAX_MAIN_LOOP_TURNS: usize = 200;
        let observed = self.observed_probe_scene();
        if observed == Some(expected) {
            eprintln!(
                "Rinka accelerator probe step={step} expected_scene={expected} observed_scene={expected} pass=true"
            );
            if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut() {
                probe.attempts = 0;
            }
            return Some(true);
        }
        let attempts = {
            let mut probe = self.ivars().accelerator_probe.borrow_mut();
            let probe = probe.as_mut()?;
            probe.attempts += 1;
            probe.attempts
        };
        if attempts < MAX_MAIN_LOOP_TURNS {
            return None;
        }
        eprintln!(
            "Rinka accelerator probe step={step} expected_scene={expected} observed_scene={} pass=false",
            observed.unwrap_or("unknown")
        );
        if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut() {
            probe.passed = false;
        }
        Some(false)
    }

    /// Posts one synthetic key-down through the real event queue so the
    /// local monitor observes it exactly like hardware input.
    fn post_probe_chord(&self, characters: &str, key_code: u16, flags: usize) {
        self.post_probe_key(characters, characters, key_code, flags, false);
    }

    /// Posts one synthetic key-down with distinct translated and
    /// modifier-free character strings and an explicit repeat flag, so the
    /// text-input probe can replay arrows, control chords, and held keys
    /// faithfully.
    fn post_probe_key(
        &self,
        characters: &str,
        characters_ignoring_modifiers: &str,
        key_code: u16,
        flags: usize,
        repeat: bool,
    ) {
        let Some(window) = self.ivars().windows.borrow().first().cloned() else {
            return;
        };
        let text = ns_string(characters);
        let unmodified = ns_string(characters_ignoring_modifiers);
        // SAFETY: The retained NSWindow supplies a live window number and the
        // synthesized NSEvent is queued on the main thread; AppKit dequeues
        // it through the normal run-loop dispatch that local monitors hook.
        unsafe {
            let window_number: isize = msg_send![window.as_object(), windowNumber];
            let event: *mut AnyObject = msg_send![objc2::class!(NSEvent),
                keyEventWithType: 10_usize,
                location: Point::default(),
                modifierFlags: flags,
                timestamp: 0.0_f64,
                windowNumber: window_number,
                context: std::ptr::null::<AnyObject>(),
                characters: text.as_object(),
                charactersIgnoringModifiers: unmodified.as_object(),
                isARepeat: repeat,
                keyCode: key_code
            ];
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let _: () = msg_send![application, postEvent: event, atStart: false];
        }
    }

    fn focus_probe_search_field(&self) -> bool {
        let Some(window) = self.ivars().windows.borrow().first().cloned() else {
            return false;
        };
        // SAFETY: The retained NSWindow owns its toolbar; the search field is
        // retained by its NSSearchToolbarItem while focus moves to it.
        unsafe {
            let toolbar: *mut AnyObject = msg_send![window.as_object(), toolbar];
            let Some(toolbar) = NonNull::new(toolbar) else {
                return false;
            };
            let items: *mut AnyObject = msg_send![toolbar.as_ref(), items];
            let count: usize = msg_send![items, count];
            for index in 0..count {
                let item: *mut AnyObject = msg_send![items, objectAtIndex: index];
                let is_search: bool =
                    msg_send![item, isKindOfClass: objc2::class!(NSSearchToolbarItem)];
                if !is_search {
                    continue;
                }
                let field: *mut AnyObject = msg_send![item, searchField];
                let accepted: bool = msg_send![window.as_object(), makeFirstResponder: field];
                return accepted && first_responder_is_text_input(window.as_object());
            }
        }
        false
    }

    fn unfocus_probe_text_input(&self) {
        let Some(window) = self.ivars().windows.borrow().first().cloned() else {
            return;
        };
        // SAFETY: Resigning first responder on the retained key window ends
        // the field editor session begun by the probe.
        unsafe {
            let _: bool =
                msg_send![window.as_object(), makeFirstResponder: std::ptr::null::<AnyObject>()];
        }
    }

    /// Returns whether the hidden `.env` row is currently mounted, the
    /// observable effect of the toggle-hidden accelerator.
    fn probe_hidden_file_visible(&self) -> bool {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().is_some_and(|runtime| {
            runtime.with_renderer(|renderer| {
                renderer.mounted().is_some_and(|root| {
                    mounted_handle_for_key(root, "file-HiddenEnvironment").is_some()
                })
            })
        })
    }

    /// Waits until the hidden-row visibility matches, retrying across
    /// main-loop turns; `Some(true)` reports the settled match, `Some(false)`
    /// the timeout, and `None` requests another turn.
    fn await_probe_hidden_file(&self, step: &'static str, expected: bool) -> Option<bool> {
        const MAX_MAIN_LOOP_TURNS: usize = 200;
        let observed = self.probe_hidden_file_visible();
        if observed == expected {
            eprintln!(
                "Rinka accelerator probe step={step} expected_hidden_visible={expected} observed_hidden_visible={observed} pass=true"
            );
            if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut() {
                probe.attempts = 0;
            }
            return Some(true);
        }
        let attempts = {
            let mut probe = self.ivars().accelerator_probe.borrow_mut();
            let probe = probe.as_mut()?;
            probe.attempts += 1;
            probe.attempts
        };
        if attempts < MAX_MAIN_LOOP_TURNS {
            return None;
        }
        eprintln!(
            "Rinka accelerator probe step={step} expected_hidden_visible={expected} observed_hidden_visible={observed} pass=false"
        );
        if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut() {
            probe.passed = false;
        }
        Some(false)
    }

    /// Reads the key equivalent of the toolbar menu action titled `Name`.
    fn probe_menu_key_equivalent(&self) -> Option<(String, usize)> {
        let window = self.ivars().windows.borrow().first().cloned()?;
        // SAFETY: The toolbar, its NSMenuToolbarItem, the menu, and its items
        // are all retained by the window; only public properties are read.
        unsafe {
            let toolbar: *mut AnyObject = msg_send![window.as_object(), toolbar];
            let toolbar = NonNull::new(toolbar)?;
            let items: *mut AnyObject = msg_send![toolbar.as_ref(), items];
            let count: usize = msg_send![items, count];
            for index in 0..count {
                let item: *mut AnyObject = msg_send![items, objectAtIndex: index];
                let is_menu: bool =
                    msg_send![item, isKindOfClass: objc2::class!(NSMenuToolbarItem)];
                if !is_menu {
                    continue;
                }
                let menu: *mut AnyObject = msg_send![item, menu];
                let Some(menu) = NonNull::new(menu) else {
                    continue;
                };
                let entries: *mut AnyObject = msg_send![menu.as_ref(), itemArray];
                let entry_count: usize = msg_send![entries, count];
                for entry_index in 0..entry_count {
                    let entry: *mut AnyObject = msg_send![entries, objectAtIndex: entry_index];
                    let title: *mut AnyObject = msg_send![entry, title];
                    if rust_string(title) != "Name" {
                        continue;
                    }
                    let key_equivalent: *mut AnyObject = msg_send![entry, keyEquivalent];
                    let mask: usize = msg_send![entry, keyEquivalentModifierMask];
                    return Some((rust_string(key_equivalent), mask));
                }
            }
        }
        None
    }

    fn advance_accelerator_probe(&self) {
        const MAX_ACTIVATION_TURNS: usize = 200;
        // Turns the withheld chord is given to prove it changes nothing.
        const WITHHELD_QUIET_TURNS: usize = 4;
        let Some((step, attempts)) = self
            .ivars()
            .accelerator_probe
            .borrow()
            .as_ref()
            .map(|probe| (probe.step, probe.attempts))
        else {
            return;
        };
        let advance = |next_attempts: usize| {
            if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut() {
                probe.step += 1;
                probe.attempts = next_attempts;
            }
            self.schedule_accelerator_probe();
        };
        match step {
            0 => {
                // Establish key status before the first chord: menu key
                // equivalents and window-scoped table entries both require
                // an active application.
                if !self.probe_window_is_key() {
                    if attempts >= MAX_ACTIVATION_TURNS {
                        self.fail_accelerator_probe_step("initial_scene", "activation_timeout");
                        return;
                    }
                    if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut() {
                        probe.attempts += 1;
                    }
                    self.schedule_accelerator_probe();
                    return;
                }
                let observed = self.observed_probe_scene();
                if observed != Some("ready") {
                    self.fail_accelerator_probe_step("initial_scene", "expected_scene=ready");
                    return;
                }
                eprintln!(
                    "Rinka accelerator probe step=initial_scene expected_scene=ready observed_scene=ready pass=true"
                );
                // Primary+2 is declared on both the View menu item and the
                // accelerator table; the menu owns it. The monitor logs the
                // deferral and native menu dispatch switches the scene —
                // exactly once, asserted from the activation log.
                self.post_probe_chord("2", 19, NS_EVENT_MODIFIER_COMMAND);
                advance(0);
            }
            1 => match self.await_probe_scene("menu_chord_dispatch", "empty") {
                None => self.schedule_accelerator_probe(),
                Some(false) => self.finish_accelerator_probe(),
                Some(true) => {
                    let focused = self.focus_probe_search_field();
                    eprintln!("Rinka accelerator probe step=focus_search_field pass={focused}");
                    if !focused {
                        if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut()
                        {
                            probe.passed = false;
                        }
                        self.finish_accelerator_probe();
                        return;
                    }
                    // Primary+1 is menu-owned too; native menu key
                    // equivalents fire over focused text input, so the scene
                    // must switch while the field owns typing.
                    self.post_probe_chord("1", 18, NS_EVENT_MODIFIER_COMMAND);
                    advance(0);
                }
            },
            2 => match self.await_probe_scene("menu_chord_over_text_input", "ready") {
                None => self.schedule_accelerator_probe(),
                Some(false) => self.finish_accelerator_probe(),
                Some(true) => {
                    // Re-assert the editing session before the withheld test;
                    // the reconciliation triggered by Primary+1 must not have
                    // moved focus, but the field is re-focused defensively.
                    let focused = self.focus_probe_search_field();
                    eprintln!("Rinka accelerator probe step=refocus_search_field pass={focused}");
                    if !focused {
                        if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut()
                        {
                            probe.passed = false;
                        }
                        self.finish_accelerator_probe();
                        return;
                    }
                    // Primary+Shift+H has no menu home, so the accelerator
                    // table's defer-to-typing policy governs it: while the
                    // search field is key the chord is withheld and the
                    // event falls through to the field editor.
                    self.post_probe_chord(
                        "H",
                        4,
                        NS_EVENT_MODIFIER_COMMAND | NS_EVENT_MODIFIER_SHIFT,
                    );
                    advance(0);
                }
            },
            3 => {
                // The withheld chord leaves no state trace; give its event
                // several turns to dispatch, then require the hidden row
                // still absent. Whether the field owned focus at dispatch
                // time is asserted from the monitor's event log line.
                if attempts < WITHHELD_QUIET_TURNS {
                    if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut() {
                        probe.attempts += 1;
                    }
                    self.schedule_accelerator_probe();
                    return;
                }
                let hidden_visible = self.probe_hidden_file_visible();
                let scene = self.observed_probe_scene();
                let passed = !hidden_visible && scene == Some("ready");
                eprintln!(
                    "Rinka accelerator probe step=text_field_precedence expected_hidden_visible=false observed_hidden_visible={hidden_visible} observed_scene={} pass={passed}",
                    scene.unwrap_or("unknown")
                );
                if !passed {
                    if let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_accelerator_probe();
                    return;
                }
                self.unfocus_probe_text_input();
                // With focus released the withheld chord fires again.
                self.post_probe_chord(
                    "H",
                    4,
                    NS_EVENT_MODIFIER_COMMAND | NS_EVENT_MODIFIER_SHIFT,
                );
                advance(0);
            }
            _ => match self.await_probe_hidden_file("chord_after_unfocus", true) {
                None => self.schedule_accelerator_probe(),
                Some(false) => self.finish_accelerator_probe(),
                Some(true) => {
                    let menu = self.probe_menu_key_equivalent();
                    let expected_mask = NS_EVENT_MODIFIER_COMMAND | NS_EVENT_MODIFIER_SHIFT;
                    let menu_passed = menu
                        .as_ref()
                        .is_some_and(|(text, mask)| text == "n" && *mask == expected_mask);
                    eprintln!(
                        "Rinka accelerator probe step=menu_key_equivalent observed={menu:?} expected=(\"n\", {expected_mask}) pass={menu_passed}"
                    );
                    if !menu_passed
                        && let Some(probe) = self.ivars().accelerator_probe.borrow_mut().as_mut()
                    {
                        probe.passed = false;
                    }
                    self.finish_accelerator_probe();
                }
            },
        }
    }

    fn schedule_accelerator_probe(&self) {
        // SAFETY: The next main-loop turn runs after the queued key event has
        // been dispatched and any resulting reconciliation has completed.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runAcceleratorProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.05_f64
            ];
        }
    }

    fn finish_accelerator_probe(&self) {
        let passed = self
            .ivars()
            .accelerator_probe
            .borrow()
            .as_ref()
            .is_some_and(|probe| probe.passed);
        eprintln!(
            "Rinka accelerator probe result={}",
            if passed { "PASS" } else { "FAIL" }
        );
        if std::env::var_os("RINKA_APPKIT_ACCELERATOR_PROBE_HOLD").is_none() {
            // SAFETY: Diagnostic completion terminates only the current test app.
            unsafe {
                let application: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let _: () = msg_send![application, terminate: std::ptr::null::<AnyObject>()];
            }
        }
    }

    fn begin_clipboard_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_CLIPBOARD_PROBE").is_none()
            || self.ivars().clipboard_probe.borrow().is_some()
        {
            return;
        }
        if std::env::var_os("RINKA_APPKIT_SCENE_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_ACCELERATOR_PROBE").is_some()
        {
            panic!("the clipboard probe must run in its own process");
        }
        *self.ivars().clipboard_probe.borrow_mut() = Some(ClipboardProbe {
            step: 0,
            attempts: 0,
            passed: true,
        });
        self.schedule_clipboard_probe();
    }

    /// Presses the mounted native button declared under `key`.
    ///
    /// The retained view is extracted before the click so every renderer
    /// borrow is released; the resulting component update may re-render
    /// freely while the click is being delivered.
    fn press_probe_button(&self, key: &str) -> bool {
        let view = {
            let renderers = self.ivars().renderers.borrow();
            let Some(runtime) = renderers.first() else {
                return false;
            };
            runtime.with_renderer(|renderer| {
                renderer.mounted().and_then(|root| {
                    mounted_handle_for_key(root, key).map(|handle| handle.0.view.clone())
                })
            })
        };
        let Some(view) = view else {
            return false;
        };
        // SAFETY: The retained view is the NSButton mounted for `key`;
        // performClick: drives its connected target/action synchronously on
        // AppKit's main thread.
        unsafe {
            let _: () = msg_send![view.as_object(), performClick: std::ptr::null::<AnyObject>()];
        }
        true
    }

    /// Reads the explorer's mounted clipboard status note, if present.
    fn probe_clipboard_note(&self) -> Option<String> {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().and_then(|runtime| {
            runtime.with_renderer(|renderer| {
                renderer
                    .mounted()
                    .and_then(|root| mounted_label_text(root, "clipboard-note"))
            })
        })
    }

    /// Reads the general pasteboard through the host's own service.
    fn probe_general_pasteboard_text(&self) -> Option<String> {
        let clipboard = rinka_core::Clipboard::new(PasteboardClipboard::general());
        let delivered = Rc::new(RefCell::new(None));
        let sink = delivered.clone();
        clipboard.read_text(move |result| *sink.borrow_mut() = Some(result));
        delivered
            .borrow_mut()
            .take()
            .and_then(Result::ok)
            .flatten()
    }

    /// Returns the toolbar's native search field, if one is installed.
    fn probe_search_field(&self) -> Option<Id> {
        let window = self.ivars().windows.borrow().first().cloned()?;
        // SAFETY: The retained NSWindow owns its toolbar; the search field is
        // retained by its NSSearchToolbarItem and this wrapper adds its own
        // balanced retain.
        unsafe {
            let toolbar: *mut AnyObject = msg_send![window.as_object(), toolbar];
            let toolbar = NonNull::new(toolbar)?;
            let items: *mut AnyObject = msg_send![toolbar.as_ref(), items];
            let count: usize = msg_send![items, count];
            for index in 0..count {
                let item: *mut AnyObject = msg_send![items, objectAtIndex: index];
                let is_search: bool =
                    msg_send![item, isKindOfClass: objc2::class!(NSSearchToolbarItem)];
                if !is_search {
                    continue;
                }
                let field: *mut AnyObject = msg_send![item, searchField];
                return NonNull::new(field).map(|field| Id::from_borrowed(field.as_ptr()));
            }
        }
        None
    }

    /// Returns the window's focused field editor, if native text has focus.
    fn probe_focused_field_editor(&self) -> Option<Id> {
        let window = self.ivars().windows.borrow().first().cloned()?;
        // SAFETY: The first responder is read on the main thread and only
        // returned once the NSText class membership check has passed.
        unsafe {
            let responder: *mut AnyObject = msg_send![window.as_object(), firstResponder];
            let responder = NonNull::new(responder)?;
            let is_text: bool =
                msg_send![responder.as_ref(), isKindOfClass: objc2::class!(NSText)];
            is_text.then(|| Id::from_borrowed(responder.as_ptr()))
        }
    }

    /// Fills the toolbar search field with `marker` and selects its content,
    /// making the field's editor the first responder — the setup for proving
    /// that the native text field's own Copy still works with no rinka code
    /// in the path.
    fn prepare_probe_search_field_selection(&self, marker: &str) -> bool {
        let Some(window) = self.ivars().windows.borrow().first().cloned() else {
            return false;
        };
        let Some(field) = self.probe_search_field() else {
            return false;
        };
        // SAFETY: selectText: begins a field editor session on the main
        // thread and selects the programmatically set content.
        unsafe {
            set_string(field.as_object(), SET_STRING_VALUE, marker);
            let _: () = msg_send![field.as_object(), selectText: std::ptr::null::<AnyObject>()];
            first_responder_is_text_input(window.as_object())
        }
    }

    /// Sends the standard Copy action to the window's focused field editor —
    /// the action a Cmd+C key equivalent resolves to for native text.
    ///
    /// Requiring no application activation keeps the probe deterministic on
    /// a busy desktop and means it never steals the user's focus; the
    /// key-routing half (an unmatched chord falls through to focused text)
    /// is the accelerator probe's landed evidence.
    fn copy_probe_search_field_selection(&self) -> bool {
        let Some(editor) = self.probe_focused_field_editor() else {
            return false;
        };
        // SAFETY: NSText's copy: writes the selection to the general
        // pasteboard on the main thread without requiring key status.
        unsafe {
            let _: () = msg_send![editor.as_object(), copy: std::ptr::null::<AnyObject>()];
        }
        true
    }

    /// Clears the search field, focuses it, and sends the standard Paste
    /// action — the action a Cmd+V key equivalent resolves to — returning
    /// the field editor's text after the paste.
    fn paste_probe_search_field(&self) -> Option<String> {
        let field = self.probe_search_field()?;
        // SAFETY: selectText: re-establishes the field editor session; the
        // editor's paste: inserts the general pasteboard's text and its
        // string is copied out on the main thread.
        unsafe {
            set_string(field.as_object(), SET_STRING_VALUE, "");
            let _: () = msg_send![field.as_object(), selectText: std::ptr::null::<AnyObject>()];
            let editor = self.probe_focused_field_editor()?;
            let _: () = msg_send![editor.as_object(), paste: std::ptr::null::<AnyObject>()];
            let value: *mut AnyObject = msg_send![editor.as_object(), string];
            Some(rust_string(value))
        }
    }

    fn fail_clipboard_probe_step(&self, step: &'static str, detail: &str) {
        eprintln!("Rinka clipboard probe step={step} {detail} pass=false");
        if let Some(probe) = self.ivars().clipboard_probe.borrow_mut().as_mut() {
            probe.passed = false;
        }
        self.finish_clipboard_probe();
    }

    /// Drives the live interop sequence: Paste Path reads the seeded text;
    /// the focused search field's standard Copy action proves the native
    /// text field's own clipboard behavior still works with no rinka code
    /// in the path; Copy Path runs last so the app-written path is what the
    /// wrapping script's final pbpaste observes. All observations are
    /// in-process (mounted props and the pasteboard service); the
    /// cross-process interop itself is asserted by the script through
    /// pbcopy and pbpaste.
    fn advance_clipboard_probe(&self) {
        const MAX_MAIN_LOOP_TURNS: usize = 200;
        /// Selected search-field content the native Copy must transfer.
        const NATIVE_FIELD_MARKER: &str = "rinka native field copy 検証";
        let Some((step, attempts)) = self
            .ivars()
            .clipboard_probe
            .borrow()
            .as_ref()
            .map(|probe| (probe.step, probe.attempts))
        else {
            return;
        };
        let retry = || {
            if let Some(probe) = self.ivars().clipboard_probe.borrow_mut().as_mut() {
                probe.attempts += 1;
            }
            self.schedule_clipboard_probe();
        };
        let advance = || {
            if let Some(probe) = self.ivars().clipboard_probe.borrow_mut().as_mut() {
                probe.step += 1;
                probe.attempts = 0;
            }
            self.schedule_clipboard_probe();
        };
        match step {
            0 => {
                if self.observed_probe_scene() != Some("ready") {
                    if attempts >= MAX_MAIN_LOOP_TURNS {
                        self.fail_clipboard_probe_step("initial_scene", "expected_scene=ready");
                        return;
                    }
                    retry();
                    return;
                }
                eprintln!("Rinka clipboard probe step=initial_scene observed_scene=ready pass=true");
                if !self.press_probe_button("paste-path") {
                    self.fail_clipboard_probe_step("press_paste", "button_not_mounted");
                    return;
                }
                advance();
            }
            1 => match self.probe_clipboard_note() {
                Some(note) => {
                    eprintln!("Rinka clipboard probe step=paste observed={note:?} pass=true");
                    advance();
                }
                None if attempts < MAX_MAIN_LOOP_TURNS => retry(),
                None => self.fail_clipboard_probe_step("paste", "note_timeout"),
            },
            2 => {
                if !self.prepare_probe_search_field_selection(NATIVE_FIELD_MARKER) {
                    self.fail_clipboard_probe_step(
                        "native_field_copy",
                        "search_field_not_focused",
                    );
                    return;
                }
                if !self.copy_probe_search_field_selection() {
                    self.fail_clipboard_probe_step(
                        "native_field_copy",
                        "field_editor_not_focused",
                    );
                    return;
                }
                advance();
            }
            3 => match self.probe_general_pasteboard_text() {
                Some(text) if text == NATIVE_FIELD_MARKER => {
                    eprintln!(
                        "Rinka clipboard probe step=native_field_copy observed={text:?} pass=true"
                    );
                    match self.paste_probe_search_field() {
                        Some(pasted) if pasted == NATIVE_FIELD_MARKER => {
                            eprintln!(
                                "Rinka clipboard probe step=native_field_paste observed={pasted:?} pass=true"
                            );
                        }
                        observed => {
                            self.fail_clipboard_probe_step(
                                "native_field_paste",
                                &format!("observed={observed:?}"),
                            );
                            return;
                        }
                    }
                    self.unfocus_probe_text_input();
                    if !self.press_probe_button("copy-path") {
                        self.fail_clipboard_probe_step("press_copy", "button_not_mounted");
                        return;
                    }
                    advance();
                }
                _ if attempts < MAX_MAIN_LOOP_TURNS => retry(),
                observed => self.fail_clipboard_probe_step(
                    "native_field_copy",
                    &format!("observed={observed:?}"),
                ),
            },
            _ => match self.probe_clipboard_note() {
                Some(note) if note.starts_with("Copied ") => {
                    eprintln!("Rinka clipboard probe step=copy observed={note:?} pass=true");
                    self.finish_clipboard_probe();
                }
                _ if attempts < MAX_MAIN_LOOP_TURNS => retry(),
                observed => self.fail_clipboard_probe_step(
                    "copy",
                    &format!("observed={observed:?}"),
                ),
            },
        }
    }

    fn schedule_clipboard_probe(&self) {
        // SAFETY: The next main-loop turn runs after the pressed button's
        // action has dispatched and any resulting reconciliation completed.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runClipboardProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.05_f64
            ];
        }
    }

    fn finish_clipboard_probe(&self) {
        let passed = self
            .ivars()
            .clipboard_probe
            .borrow()
            .as_ref()
            .is_some_and(|probe| probe.passed);
        eprintln!(
            "Rinka clipboard probe result={}",
            if passed { "PASS" } else { "FAIL" }
        );
        // SAFETY: Diagnostic completion terminates only the current test app.
        unsafe {
            let application: *mut AnyObject =
                msg_send![objc2::class!(NSApplication), sharedApplication];
            let _: () = msg_send![application, terminate: std::ptr::null::<AnyObject>()];
        }
    }

    fn begin_text_input_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_TEXT_INPUT_PROBE").is_none()
            || self.ivars().text_input_probe.borrow().is_some()
        {
            return;
        }
        for other in [
            "RINKA_APPKIT_SCENE_PROBE",
            "RINKA_APPKIT_TRANSITION_PROBE",
            "RINKA_APPKIT_ACCELERATOR_PROBE",
            "RINKA_APPKIT_CLIPBOARD_PROBE",
        ] {
            if std::env::var_os(other).is_some() {
                panic!("the text-input probe must run in its own process");
            }
        }
        *self.ivars().text_input_probe.borrow_mut() = Some(TextInputProbe {
            step: 0,
            attempts: 0,
            passed: true,
            caret_before: None,
        });
        self.schedule_text_input_probe();
    }

    /// Returns the mounted echo canvas's native view.
    fn probe_canvas_view(&self) -> Option<Id> {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().and_then(|runtime| {
            runtime.with_renderer(|renderer| {
                renderer
                    .mounted()
                    .and_then(|root| mounted_handle_for_key(root, "canvas-surface"))
                    .map(|handle| handle.0.view.clone())
            })
        })
    }

    /// Reads the explorer's mounted text-input caption, if present.
    fn probe_input_caption(&self) -> Option<String> {
        let renderers = self.ivars().renderers.borrow();
        renderers.first().and_then(|runtime| {
            runtime.with_renderer(|renderer| {
                renderer
                    .mounted()
                    .and_then(|root| mounted_label_text(root, "canvas-input-caption"))
            })
        })
    }

    /// Sends one primary click through NSWindow sendEvent: at the center of
    /// the canvas — real hit testing, so click-to-focus is exercised live.
    fn click_probe_canvas(&self) -> bool {
        let Some(view) = self.probe_canvas_view() else {
            return false;
        };
        // SAFETY: The mounted view, its window, and NSEvent construction are
        // used on AppKit's main thread; sendEvent: performs ordinary event
        // dispatch confined to this application's window.
        unsafe {
            let window: *mut AnyObject = msg_send![view.as_object(), window];
            if window.is_null() {
                return false;
            }
            let bounds: Rect = msg_send![view.as_object(), bounds];
            let center = Point {
                x: bounds.origin.x + bounds.size.width / 2.0,
                y: bounds.origin.y + bounds.size.height / 2.0,
            };
            let in_window: Point = msg_send![
                view.as_object(),
                convertPoint: center,
                toView: std::ptr::null::<AnyObject>()
            ];
            let window_number: isize = msg_send![window, windowNumber];
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
    }

    /// Calls setMarkedText:selectedRange:replacementRange: directly on the
    /// canvas — protocol-level composition evidence that stays deterministic
    /// regardless of which input source the desktop has active.
    fn set_probe_marked_text(&self, text: &str, selected_location: usize, selected_length: usize) {
        let Some(view) = self.probe_canvas_view() else {
            return;
        };
        let marked = ns_string(text);
        // SAFETY: The retained view implements NSTextInputClient; ranges use
        // UTF-16 units exactly as an input method supplies them.
        unsafe {
            let _: () = msg_send![
                view.as_object(),
                setMarkedText: marked.as_object(),
                selectedRange: NSRange::new(selected_location, selected_length),
                replacementRange: NSRange::new(NSNotFound as usize, 0)
            ];
        }
    }

    /// Calls insertText:replacementRange: directly on the canvas.
    fn insert_probe_text(&self, text: &str) {
        let Some(view) = self.probe_canvas_view() else {
            return;
        };
        let inserted = ns_string(text);
        // SAFETY: The retained view implements NSTextInputClient.
        unsafe {
            let _: () = msg_send![
                view.as_object(),
                insertText: inserted.as_object(),
                replacementRange: NSRange::new(NSNotFound as usize, 0)
            ];
        }
    }

    /// Reads the screen rectangle the canvas serves for the IME candidate
    /// window.
    fn probe_first_rect(&self) -> Option<Rect> {
        let view = self.probe_canvas_view()?;
        // SAFETY: The retained view implements NSTextInputClient; the
        // actual-range out parameter is optional and passed as null.
        unsafe {
            Some(msg_send![
                view.as_object(),
                firstRectForCharacterRange: NSRange::new(0, 0),
                actualRange: std::ptr::null_mut::<NSRange>()
            ])
        }
    }

    fn fail_text_input_probe_step(&self, step: &'static str, detail: &str) {
        eprintln!("Rinka text-input probe step={step} {detail} pass=false");
        if let Some(probe) = self.ivars().text_input_probe.borrow_mut().as_mut() {
            probe.passed = false;
        }
        self.finish_text_input_probe();
    }

    /// Waits until the input caption contains `needle`, retrying across
    /// main-loop turns; `Some(true)` reports the settled match, `Some(false)`
    /// the timeout, and `None` requests another turn.
    fn await_input_caption(&self, step: &'static str, needle: &str) -> Option<bool> {
        const MAX_MAIN_LOOP_TURNS: usize = 200;
        let caption = self.probe_input_caption();
        if caption
            .as_deref()
            .is_some_and(|caption| caption.contains(needle))
        {
            eprintln!("Rinka text-input probe step={step} expected={needle:?} pass=true");
            if let Some(probe) = self.ivars().text_input_probe.borrow_mut().as_mut() {
                probe.attempts = 0;
            }
            return Some(true);
        }
        let attempts = {
            let mut probe = self.ivars().text_input_probe.borrow_mut();
            let probe = probe.as_mut()?;
            probe.attempts += 1;
            probe.attempts
        };
        if attempts < MAX_MAIN_LOOP_TURNS {
            return None;
        }
        eprintln!(
            "Rinka text-input probe step={step} expected={needle:?} observed={caption:?} pass=false"
        );
        if let Some(probe) = self.ivars().text_input_probe.borrow_mut().as_mut() {
            probe.passed = false;
        }
        Some(false)
    }

    #[allow(clippy::too_many_lines)]
    fn advance_text_input_probe(&self) {
        const MAX_ACTIVATION_TURNS: usize = 200;
        /// Turns a withheld chord is given to prove it changes no scene.
        const WITHHELD_QUIET_TURNS: usize = 4;
        let Some((step, attempts)) = self
            .ivars()
            .text_input_probe
            .borrow()
            .as_ref()
            .map(|probe| (probe.step, probe.attempts))
        else {
            return;
        };
        let advance = |next_attempts: usize| {
            if let Some(probe) = self.ivars().text_input_probe.borrow_mut().as_mut() {
                probe.step += 1;
                probe.attempts = next_attempts;
            }
            self.schedule_text_input_probe();
        };
        let retry = || {
            if let Some(probe) = self.ivars().text_input_probe.borrow_mut().as_mut() {
                probe.attempts += 1;
            }
            self.schedule_text_input_probe();
        };
        match step {
            0 => {
                // Establish key status, the canvas scene, and click-to-focus.
                if !self.probe_window_is_key() {
                    if attempts >= MAX_ACTIVATION_TURNS {
                        self.fail_text_input_probe_step("initial_scene", "activation_timeout");
                        return;
                    }
                    retry();
                    return;
                }
                if self.observed_probe_scene() != Some("canvas") {
                    if attempts >= MAX_ACTIVATION_TURNS {
                        self.fail_text_input_probe_step("initial_scene", "expected_scene=canvas");
                        return;
                    }
                    retry();
                    return;
                }
                eprintln!(
                    "Rinka text-input probe step=initial_scene observed_scene=canvas pass=true"
                );
                // The window's key-view loop assigns the input-accepting
                // canvas as initial first responder on its own; release it
                // so the click's focusing effect is proven in isolation.
                self.unfocus_probe_text_input();
                advance(0);
            }
            1 => match self.await_input_caption("unfocus_canvas", "focused=false") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    if !self.click_probe_canvas() {
                        self.fail_text_input_probe_step("click_focus", "canvas_not_mounted");
                        return;
                    }
                    advance(0);
                }
            },
            2 => match self.await_input_caption("click_focus", "focused=true") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    let Some(view) = self.probe_canvas_view() else {
                        self.fail_text_input_probe_step("first_responder", "canvas_not_mounted");
                        return;
                    };
                    // SAFETY: Responder identity, accessibility role, and the
                    // input context's source are read on the main thread.
                    let (is_first_responder, role, source) = unsafe {
                        let window: *mut AnyObject = msg_send![view.as_object(), window];
                        let responder: *mut AnyObject = msg_send![window, firstResponder];
                        let role: *mut AnyObject = msg_send![view.as_object(), accessibilityRole];
                        let context: *mut AnyObject = msg_send![view.as_object(), inputContext];
                        let source = if context.is_null() {
                            "no-input-context".to_owned()
                        } else {
                            let source: *mut AnyObject =
                                msg_send![context, selectedKeyboardInputSource];
                            rust_string(source)
                        };
                        (
                            responder == view.as_ptr(),
                            rust_string(role),
                            source,
                        )
                    };
                    let role_passed = role == "AXTextArea";
                    eprintln!(
                        "Rinka text-input probe step=first_responder is_first_responder={is_first_responder} role={role} input_source={source} pass={}",
                        is_first_responder && role_passed
                    );
                    if !(is_first_responder && role_passed) {
                        if let Some(probe) =
                            self.ivars().text_input_probe.borrow_mut().as_mut()
                        {
                            probe.passed = false;
                        }
                        self.finish_text_input_probe();
                        return;
                    }
                    // End-to-end typing: one real key-down through the queue.
                    // The active input source decides whether it inserts text
                    // (raw path) or begins a composition (composition path);
                    // both prove keyDown → NSTextInputClient delivery.
                    self.post_probe_key("h", "h", 4, 0, false);
                    advance(0);
                }
            },
            3 => {
                const MAX_MAIN_LOOP_TURNS: usize = 200;
                let caption = self.probe_input_caption().unwrap_or_default();
                let raw_path = caption.contains("key=H");
                let composition_path = !caption.contains("preedit=\"\"");
                if !(raw_path || composition_path) {
                    if attempts >= MAX_MAIN_LOOP_TURNS {
                        self.fail_text_input_probe_step(
                            "end_to_end_key",
                            &format!("observed={caption:?}"),
                        );
                        return;
                    }
                    retry();
                    return;
                }
                eprintln!(
                    "Rinka text-input probe step=end_to_end_key path={} pass=true",
                    if raw_path { "raw" } else { "composition" }
                );
                // Reset any composition the real input source opened, then
                // continue with deterministic protocol-level text.
                self.set_probe_marked_text("", 0, 0);
                self.insert_probe_text("echo:");
                advance(0);
            }
            4 => match self.await_input_caption("protocol_insert", "echo:") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    let Some(rect) = self.probe_first_rect() else {
                        self.fail_text_input_probe_step("caret_rect", "canvas_not_mounted");
                        return;
                    };
                    if let Some(probe) = self.ivars().text_input_probe.borrow_mut().as_mut() {
                        probe.caret_before = Some(rect);
                    }
                    self.insert_probe_text("wide");
                    advance(0);
                }
            },
            5 => match self.await_input_caption("caret_rect_text", "wide") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    let before = self
                        .ivars()
                        .text_input_probe
                        .borrow()
                        .as_ref()
                        .and_then(|probe| probe.caret_before);
                    let (Some(before), Some(after)) = (before, self.probe_first_rect()) else {
                        self.fail_text_input_probe_step("caret_rect", "rect_unavailable");
                        return;
                    };
                    // The served rectangle advanced with the caret and stays
                    // inside the window on screen.
                    let advanced = after.origin.x > before.origin.x;
                    let contained = self
                        .ivars()
                        .windows
                        .borrow()
                        .first()
                        .is_some_and(|window| {
                            // SAFETY: The retained window's frame is read on
                            // the main thread; both are screen coordinates.
                            let frame: Rect = unsafe { msg_send![window.as_object(), frame] };
                            after.origin.x >= frame.origin.x
                                && after.origin.x <= frame.origin.x + frame.size.width
                                && after.origin.y >= frame.origin.y
                                && after.origin.y <= frame.origin.y + frame.size.height
                        });
                    eprintln!(
                        "Rinka text-input probe step=caret_rect before_x={:.1} after_x={:.1} advanced={advanced} contained={contained} pass={}",
                        before.origin.x,
                        after.origin.x,
                        advanced && contained
                    );
                    if !(advanced && contained) {
                        if let Some(probe) =
                            self.ivars().text_input_probe.borrow_mut().as_mut()
                        {
                            probe.passed = false;
                        }
                        self.finish_text_input_probe();
                        return;
                    }
                    // Raw keys the input method passes through untranslated.
                    self.post_probe_key("\u{f702}", "\u{f702}", 123, 0, false);
                    advance(0);
                }
            },
            6 => match self.await_input_caption("raw_arrow_key", "key=Left") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    self.post_probe_key("\u{3}", "c", 8, NS_EVENT_MODIFIER_CONTROL, false);
                    advance(0);
                }
            },
            7 => match self.await_input_caption("raw_control_chord", "key=Control+C") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    self.post_probe_key("\u{f703}", "\u{f703}", 124, 0, true);
                    advance(0);
                }
            },
            8 => match self.await_input_caption("raw_key_repeat", "key=Right repeat") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    // Scripted composition through the NSTextInputClient
                    // protocol: begin → update → commit, with the caret in
                    // UTF-16 units exactly as an input method sends it.
                    self.set_probe_marked_text("にほんご", 4, 0);
                    advance(0);
                }
            },
            9 => match self.await_input_caption("ime_preedit", "preedit=\"にほんご\"") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    self.set_probe_marked_text("にほん語", 3, 1);
                    advance(0);
                }
            },
            10 => match self.await_input_caption("ime_preedit_update", "preedit=\"にほん語\"") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    self.insert_probe_text("日本語");
                    advance(0);
                }
            },
            11 => match self.await_input_caption("ime_commit", "日本語\" preedit=\"\"") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    self.set_probe_marked_text("かな", 2, 0);
                    advance(0);
                }
            },
            12 => match self.await_input_caption("ime_cancel_preedit", "preedit=\"かな\"") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    self.set_probe_marked_text("", 0, 0);
                    advance(0);
                }
            },
            13 => match self.await_input_caption("ime_cancel", "preedit=\"\"") {
                None => self.schedule_text_input_probe(),
                Some(false) => self.finish_text_input_probe(),
                Some(true) => {
                    let echo_untouched = self
                        .probe_input_caption()
                        .is_some_and(|caption| !caption.contains("かな"));
                    eprintln!(
                        "Rinka text-input probe step=ime_cancel_left_no_trace pass={echo_untouched}"
                    );
                    if !echo_untouched {
                        if let Some(probe) =
                            self.ivars().text_input_probe.borrow_mut().as_mut()
                        {
                            probe.passed = false;
                        }
                        self.finish_text_input_probe();
                        return;
                    }
                    self.capture_windows_to_directory("text-input-");
                    // Accelerator precedence over the focused canvas: the
                    // window-scoped Primary+2 must be withheld and fall
                    // through to the canvas as a raw key.
                    self.post_probe_key("2", "2", 19, NS_EVENT_MODIFIER_COMMAND, false);
                    advance(0);
                }
            },
            14 => {
                if attempts < WITHHELD_QUIET_TURNS {
                    retry();
                    return;
                }
                let scene_unchanged = self.observed_probe_scene() == Some("canvas");
                let chord_reached_canvas = self
                    .probe_input_caption()
                    .is_some_and(|caption| caption.contains("key=Primary+2"));
                eprintln!(
                    "Rinka text-input probe step=withheld_over_canvas scene_unchanged={scene_unchanged} chord_reached_canvas={chord_reached_canvas} pass={}",
                    scene_unchanged && chord_reached_canvas
                );
                if !(scene_unchanged && chord_reached_canvas) {
                    if let Some(probe) = self.ivars().text_input_probe.borrow_mut().as_mut() {
                        probe.passed = false;
                    }
                    self.finish_text_input_probe();
                    return;
                }
                // The global entry still fires over the focused canvas.
                self.post_probe_key("1", "1", 18, NS_EVENT_MODIFIER_COMMAND, false);
                advance(0);
            }
            _ => {
                const MAX_MAIN_LOOP_TURNS: usize = 200;
                if self.observed_probe_scene() != Some("ready") {
                    if attempts >= MAX_MAIN_LOOP_TURNS {
                        self.fail_text_input_probe_step(
                            "global_over_canvas",
                            "expected_scene=ready",
                        );
                        return;
                    }
                    retry();
                    return;
                }
                eprintln!(
                    "Rinka text-input probe step=global_over_canvas observed_scene=ready pass=true"
                );
                self.finish_text_input_probe();
            }
        }
    }

    fn schedule_text_input_probe(&self) {
        // SAFETY: The next main-loop turn runs after posted events have
        // dispatched and any resulting reconciliation has completed.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runTextInputProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.05_f64
            ];
        }
    }

    fn finish_text_input_probe(&self) {
        let passed = self
            .ivars()
            .text_input_probe
            .borrow()
            .as_ref()
            .is_some_and(|probe| probe.passed);
        eprintln!(
            "Rinka text-input probe result={}",
            if passed { "PASS" } else { "FAIL" }
        );
        if std::env::var_os("RINKA_APPKIT_TEXT_INPUT_PROBE_HOLD").is_none() {
            // SAFETY: Diagnostic completion terminates only the current test app.
            unsafe {
                let application: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let _: () = msg_send![application, terminate: std::ptr::null::<AnyObject>()];
            }
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

/// Renders one live view into a PNG at the window's backing scale.
///
/// # Safety
///
/// `view` must be a live NSView used on AppKit's main thread.
unsafe fn write_view_capture(view: &AnyObject, path: &std::path::Path) -> bool {
    // SAFETY: cacheDisplayInRect draws the current layout into rep storage
    // owned by the view; the PNG data is written before anything releases it.
    unsafe {
        let bounds: Rect = msg_send![view, bounds];
        let rep: *mut AnyObject = msg_send![view, bitmapImageRepForCachingDisplayInRect: bounds];
        let Some(rep) = NonNull::new(rep) else {
            eprintln!("Rinka view capture failed: no caching rep");
            return false;
        };
        let _: () = msg_send![view, cacheDisplayInRect: bounds, toBitmapImageRep: rep.as_ref()];
        let properties: *mut AnyObject = msg_send![objc2::class!(NSDictionary), dictionary];
        // NSBitmapImageFileTypePNG = 4.
        let data: *mut AnyObject = msg_send![
            rep.as_ref(),
            representationUsingType: 4_usize,
            properties: properties
        ];
        let Some(data) = NonNull::new(data) else {
            eprintln!("Rinka view capture failed: no PNG data");
            return false;
        };
        let Some(path_text) = path.to_str() else {
            eprintln!("Rinka view capture failed: non-UTF-8 path");
            return false;
        };
        let destination = ns_string(path_text);
        let written: bool = msg_send![
            data.as_ref(),
            writeToFile: destination.as_object(),
            atomically: true
        ];
        let pixels_wide: isize = msg_send![rep.as_ref(), pixelsWide];
        let pixels_high: isize = msg_send![rep.as_ref(), pixelsHigh];
        eprintln!(
            "Rinka window capture path={path_text} pixels={pixels_wide}x{pixels_high} written={written}"
        );
        written
    }
}
