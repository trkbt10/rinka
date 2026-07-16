//! AppKit application lifecycle, window hosting, and diagnostic probes.

use super::*;

struct ApplicationDelegateIvars {
    application: RefCell<Option<ApplicationSpec>>,
    transition_sizes: Option<(Size, Size)>,
    windows: RefCell<Vec<Id>>,
    window_initial_sizes: RefCell<Vec<Size>>,
    window_initial_extent_constraints: RefCell<Vec<Vec<Id>>>,
    split_window_frames: RefCell<Vec<Option<Rect>>>,
    renderers: RefCell<Vec<WindowRuntime<AppKitBackend>>>,
    list_registries: RefCell<Vec<ListRegistry>>,
    split_resize_epoch: Cell<u64>,
    split_restore_pending: Rc<Cell<bool>>,
    toolbar_delegates: RefCell<Vec<Retained<ToolbarDelegate>>>,
    transition_probe: RefCell<Option<TransitionProbe>>,
    scene_probe: RefCell<Option<SceneProbe>>,
}

#[derive(Clone, Copy, Debug)]
struct TransitionProbe {
    step: usize,
    phase: TransitionProbePhase,
    baseline: Rect,
    wide_size: Size,
    minimum_size: Size,
    attempts: usize,
    observed_split_epoch: u64,
    quiet_turns: usize,
    passed: bool,
}

#[derive(Debug)]
struct SceneProbe {
    expected_scene: String,
    phase: SceneProbePhase,
    attempts: usize,
    requires_live_panel: bool,
    passed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SceneProbePhase {
    AwaitingMainWindow,
    AwaitingPanelWindow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransitionProbePhase {
    WidePending,
    Wide,
    MinimumPending,
    Minimum,
}

impl TransitionProbePhase {
    const fn label(self) -> &'static str {
        match self {
            Self::WidePending => "wide-pending",
            Self::Wide => "wide",
            Self::MinimumPending => "minimum-pending",
            Self::Minimum => "minimum",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransitionProbeAction {
    ToggleSidebar,
    ToggleInspector,
    RestorePanes,
    ResizeToMinimum,
    CompletePhase,
    Finish,
    Wait,
    Fail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TransitionProbeStep {
    label: &'static str,
    sidebar_collapsed: bool,
    inspector_collapsed: bool,
    action: TransitionProbeAction,
}

const TRANSITION_PROBE_STEPS: [TransitionProbeStep; 24] = [
    TransitionProbeStep {
        label: "sidebar-cycle-1-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "sidebar-cycle-1-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "sidebar-cycle-2-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "sidebar-cycle-2-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "sidebar-cycle-3-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "sidebar-cycle-3-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-1-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-1-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-2-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-2-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-3-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "inspector-cycle-3-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-1-sidebar-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-1-both-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-1-inspector-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-1-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-2-sidebar-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-2-both-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-2-inspector-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-2-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-3-sidebar-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: false,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-3-both-hidden",
        sidebar_collapsed: true,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleSidebar,
    },
    TransitionProbeStep {
        label: "combined-cycle-3-inspector-hidden",
        sidebar_collapsed: false,
        inspector_collapsed: true,
        action: TransitionProbeAction::ToggleInspector,
    },
    TransitionProbeStep {
        label: "combined-cycle-3-restored",
        sidebar_collapsed: false,
        inspector_collapsed: false,
        action: TransitionProbeAction::CompletePhase,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiagnosticAppearance {
    Light,
    Dark,
}

impl DiagnosticAppearance {
    const fn native_name(self) -> &'static str {
        match self {
            Self::Light => "NSAppearanceNameAqua",
            Self::Dark => "NSAppearanceNameDarkAqua",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

fn parse_diagnostic_appearance(
    value: Option<&str>,
) -> Result<Option<DiagnosticAppearance>, String> {
    match value {
        None => Ok(None),
        Some("light") => Ok(Some(DiagnosticAppearance::Light)),
        Some("dark") => Ok(Some(DiagnosticAppearance::Dark)),
        Some(value) => Err(format!(
            "RINKA_APPKIT_APPEARANCE expects 'light' or 'dark', received '{value}'"
        )),
    }
}

unsafe fn configure_diagnostic_appearance(application: &AnyObject) -> Option<DiagnosticAppearance> {
    let requested = match std::env::var("RINKA_APPKIT_APPEARANCE") {
        Ok(value) => parse_diagnostic_appearance(Some(&value)),
        Err(std::env::VarError::NotPresent) => parse_diagnostic_appearance(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("RINKA_APPKIT_APPEARANCE must be valid UTF-8".to_owned())
        }
    }
    .unwrap_or_else(|error| panic!("{error}"));
    let requested = requested?;
    let name = ns_string(requested.native_name());
    let appearance: *mut AnyObject =
        unsafe { msg_send![objc2::class!(NSAppearance), appearanceNamed: name.as_object()] };
    let Some(appearance) = NonNull::new(appearance) else {
        panic!(
            "AppKit did not provide the requested {} appearance",
            requested.label()
        );
    };
    unsafe {
        let _: () = msg_send![application, setAppearance: appearance.as_ref()];
    }
    Some(requested)
}

unsafe fn assert_diagnostic_appearance(window: &AnyObject, requested: DiagnosticAppearance) {
    let effective: *mut AnyObject = unsafe { msg_send![window, effectiveAppearance] };
    let light = ns_string(DiagnosticAppearance::Light.native_name());
    let dark = ns_string(DiagnosticAppearance::Dark.native_name());
    let choices = ns_array(&[light, dark]);
    let matched: *mut AnyObject =
        unsafe { msg_send![effective, bestMatchFromAppearancesWithNames: choices.as_object()] };
    let actual = rust_string(matched);
    if actual != requested.native_name() {
        panic!(
            "AppKit effective appearance mismatch: requested {}, received {actual}",
            requested.label()
        );
    }
    eprintln!(
        "Rinka AppKit appearance requested={} effective={} pass=true",
        requested.label(),
        actual
    );
}

impl fmt::Debug for ApplicationDelegateIvars {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApplicationDelegateIvars")
            .field("has_application", &self.application.borrow().is_some())
            .field("window_count", &self.windows.borrow().len())
            .field(
                "window_initial_size_count",
                &self.window_initial_sizes.borrow().len(),
            )
            .field(
                "window_initial_extent_constraint_count",
                &self.window_initial_extent_constraints.borrow().len(),
            )
            .field(
                "split_window_frame_count",
                &self.split_window_frames.borrow().len(),
            )
            .field("renderer_count", &self.renderers.borrow().len())
            .finish_non_exhaustive()
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ApplicationDelegateIvars]
    struct ApplicationDelegate;

    // SAFETY: NSObjectProtocol adds no invariants beyond the NSObject superclass.
    unsafe impl NSObjectProtocol for ApplicationDelegate {}

    impl ApplicationDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &AnyObject) {
            self.show_initial_windows();
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window(&self, _application: &AnyObject) -> bool {
            true
        }

        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(
            &self,
            _application: &AnyObject,
            _has_visible_windows: bool,
        ) -> bool {
            // The windows are retained by this delegate and AppKit brings them
            // forward as part of its standard reopen processing.
            true
        }

        #[unsafe(method(refreshInitialLayout:))]
        fn refresh_initial_layout(&self, _sender: *mut AnyObject) {
            let trace = std::env::var_os("RINKA_APPKIT_TRACE").is_some();
            for runtime in self.ivars().renderers.borrow().iter() {
                runtime.with_renderer(|renderer| {
                    if let Some(root) = renderer.mounted() {
                        refresh_mounted_stacks(root);
                    }
                });
            }
            for window in self.ivars().windows.borrow().iter() {
                // SAFETY: The retained objects are NSWindow instances created
                // by this delegate and the delayed selector runs on main.
                unsafe {
                    let content: *mut AnyObject = msg_send![window.as_object(), contentView];
                    if let Some(content) = NonNull::new(content) {
                        let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
                        layout_scroll_documents(content.as_ref(), trace);
                        let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
                        if trace {
                            trace_window_contract(window.as_object());
                            trace_view_tree(content.as_ref(), 0);
                        }
                    }
                }
            }
            // Apply declarative window sizes after the initial native layout
            // has propagated through every content-view controller.
            unsafe {
                let _: () = msg_send![self,
                    performSelector: sel!(restoreInitialWindowSizes:),
                    withObject: std::ptr::null::<AnyObject>(),
                    afterDelay: 0.0_f64
                ];
            }
        }

        #[unsafe(method(restoreInitialWindowSizes:))]
        fn restore_initial_window_sizes(&self, _sender: *mut AnyObject) {
            let trace = std::env::var_os("RINKA_APPKIT_TRACE").is_some();
            for (index, window) in self.ivars().windows.borrow().iter().enumerate() {
                let Some(initial_size) = self
                    .ivars()
                    .window_initial_sizes
                    .borrow()
                    .get(index)
                    .copied()
                else {
                    continue;
                };
                // SAFETY: Retained NSWindow instances are resized on AppKit's
                // main thread after their content controllers have settled.
                unsafe {
                    self.set_window_content_extent(window.as_object(), initial_size);
                    let _: () = msg_send![window.as_object(), setContentSize: initial_size];
                    if trace {
                        let frame: Rect = msg_send![window.as_object(), frame];
                        eprintln!(
                            "Rinka AppKit post-initial-layout size={initial_size:?} frame={frame:?}"
                        );
                    }
                }
            }
            self.begin_transition_probe();
            self.begin_scene_probe();
        }

        #[unsafe(method(runTransitionProbe:))]
        fn run_transition_probe(&self, _sender: *mut AnyObject) {
            self.advance_transition_probe();
        }

        #[unsafe(method(runSceneProbe:))]
        fn run_scene_probe(&self, _sender: *mut AnyObject) {
            self.advance_scene_probe();
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, notification: &AnyObject) {
            // SAFETY: NSWindowDidResizeNotification carries the resized
            // NSWindow as its object and is delivered on the main thread.
            unsafe {
                let window: *mut AnyObject = msg_send![notification, object];
                let Some(window) = NonNull::new(window) else {
                    return;
                };
                let trace = std::env::var_os("RINKA_APPKIT_TRACE").is_some();
                if trace {
                    let frame: Rect = msg_send![window.as_ref(), frame];
                    eprintln!("Rinka AppKit windowDidResize begin frame={frame:?}");
                }
                let content: *mut AnyObject = msg_send![window.as_ref(), contentView];
                let Some(content) = NonNull::new(content) else {
                    return;
                };
                let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
                layout_scroll_documents(content.as_ref(), false);
                let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
            }
            let list_handles = registered_list_handles(&self.ivars().list_registries);
            refresh_all_semantic_sidebar_content_fit(&list_handles);
            if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
                unsafe {
                    let window: *mut AnyObject = msg_send![notification, object];
                    let frame: Rect = msg_send![window, frame];
                    eprintln!("Rinka AppKit windowDidResize end frame={frame:?}");
                    if let Some(index) = self.window_index(&*window)
                        && let Some(constraints) = self
                            .ivars()
                            .window_initial_extent_constraints
                            .borrow()
                            .get(index)
                    {
                        for (axis, constraint) in constraints.iter().enumerate() {
                            let active: bool = msg_send![constraint.as_object(), isActive];
                            let constant: f64 = msg_send![constraint.as_object(), constant];
                            eprintln!(
                                "Rinka AppKit window extent constraint axis={axis} active={active} constant={constant}"
                            );
                        }
                    }
                }
            }
        }

        #[unsafe(method(windowWillResize:toSize:))]
        fn window_will_resize(&self, sender: &AnyObject, frame_size: Size) -> Size {
            // NSWindow proposes an outer-frame size here. Keep the retained
            // content-extent constraints synchronized with the corresponding
            // content rect during an interactive edge drag. AppKit also calls
            // this delegate method for fitting-size changes initiated by a
            // content-view controller; those must not replace the declarative
            // window extent.
            unsafe {
                let in_live_resize: bool = msg_send![sender, inLiveResize];
                if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
                    let current: Rect = msg_send![sender, frame];
                    eprintln!(
                        "Rinka AppKit windowWillResize live={in_live_resize} current={current:?} proposed={frame_size:?}"
                    );
                }
                if !in_live_resize {
                    return frame_size;
                }
                let frame: Rect = msg_send![sender, frame];
                let proposed_frame = Rect {
                    origin: frame.origin,
                    size: frame_size,
                };
                let proposed_content: Rect = msg_send![sender, contentRectForFrameRect: proposed_frame];
                self.set_window_content_extent(sender, proposed_content.size);
            }
            frame_size
        }

        #[unsafe(method(splitViewWillResizeSubviews:))]
        fn split_view_will_resize_subviews(&self, notification: &AnyObject) {
            // SAFETY: A new transaction supersedes any restore queued for the
            // previous native resize sequence.
            unsafe {
                let _: () = msg_send![objc2::class!(NSObject),
                    cancelPreviousPerformRequestsWithTarget: self,
                    selector: sel!(restoreSplitOutlineState:),
                    object: std::ptr::null::<AnyObject>()
                ];
            }
            // Native sidebar and inspector transitions are allowed to resize
            // their sibling items, but never the owning window. Capture the
            // exact outer frame once for the whole coalesced transition.
            unsafe {
                let split_view: *mut AnyObject = msg_send![notification, object];
                if let Some(split_view) = NonNull::new(split_view) {
                    let window: *mut AnyObject = msg_send![split_view.as_ref(), window];
                    if let Some(window) = NonNull::new(window)
                        && let Some(index) = self.window_index(window.as_ref())
                    {
                        let mut frames = self.ivars().split_window_frames.borrow_mut();
                        if let Some(saved) = frames.get_mut(index)
                            && saved.is_none()
                        {
                            *saved = Some(msg_send![window.as_ref(), frame]);
                        }
                    }
                }
            }
            self.ivars().split_restore_pending.set(true);
            self.ivars()
                .split_resize_epoch
                .set(self.ivars().split_resize_epoch.get().wrapping_add(1));
            for handle in registered_list_handles(&self.ivars().list_registries) {
                if let Some(delegate) = handle.0.table_delegate.borrow().as_ref() {
                    if !matches!(
                        *delegate.ivars().style.borrow(),
                        ListStyle::Source | ListStyle::Table
                    ) {
                        continue;
                    }
                    // SAFETY: A newer split transaction owns suppression from
                    // this point, so a deferred clear from an older transaction
                    // must not release it.
                    unsafe {
                        let _: () = msg_send![objc2::class!(NSObject),
                            cancelPreviousPerformRequestsWithTarget: &**delegate,
                            selector: sel!(clearSplitExpansionSuppression),
                            object: std::ptr::null::<AnyObject>()
                        ];
                    }
                    *delegate.ivars().suppress_split_expansion.borrow_mut() = true;
                }
            }
        }

        #[unsafe(method(splitViewDidResizeSubviews:))]
        fn split_view_did_resize_subviews(&self, _notification: &AnyObject) {
            self.ivars().split_restore_pending.set(true);
            self.ivars()
                .split_resize_epoch
                .set(self.ivars().split_resize_epoch.get().wrapping_add(1));
            // SAFETY: Coalescing repeated resize notifications lets the last
            // animation frame restore every controlled outline exactly once.
            unsafe {
                let _: () = msg_send![objc2::class!(NSObject),
                    cancelPreviousPerformRequestsWithTarget: self,
                    selector: sel!(restoreSplitOutlineState:),
                    object: std::ptr::null::<AnyObject>()
                ];
                let _: () = msg_send![self,
                    performSelector: sel!(restoreSplitOutlineState:),
                    withObject: std::ptr::null::<AnyObject>(),
                    afterDelay: 0.08_f64
                ];
            }
        }

        #[unsafe(method(restoreSplitOutlineState:))]
        fn restore_split_outline_state(&self, _sender: *mut AnyObject) {
            let list_handles = registered_list_handles(&self.ivars().list_registries);
            for handle in &list_handles {
                let delegate = handle.0.table_delegate.borrow();
                let Some(delegate) = delegate.as_ref() else {
                    continue;
                };
                if !matches!(
                    *delegate.ivars().style.borrow(),
                    ListStyle::Source | ListStyle::Table
                ) {
                    continue;
                }
                // The mounted tree may have replaced this outline after Will.
                // Claim suppression again immediately before applying state so
                // every live delegate belongs to the current restore sequence.
                unsafe {
                    let _: () = msg_send![objc2::class!(NSObject),
                        cancelPreviousPerformRequestsWithTarget: &**delegate,
                        selector: sel!(clearSplitExpansionSuppression),
                        object: std::ptr::null::<AnyObject>()
                    ];
                }
                *delegate.ivars().suppress_split_expansion.borrow_mut() = true;
                // SAFETY: The retained handle is a mounted NSOutlineView.
                // Reapplying only expansion preserves selection, scrolling,
                // sorting, and cell identity in every unaffected list.
                unsafe {
                    apply_outline_expansion(handle.host_view(), &delegate.ivars().rows.borrow());
                    let _: () = msg_send![&**delegate,
                        performSelector: sel!(clearSplitExpansionSuppression),
                        withObject: std::ptr::null::<AnyObject>(),
                        afterDelay: 0.0_f64
                    ];
                }
            }
            refresh_all_semantic_sidebar_content_fit(&list_handles);
            let frames = {
                let mut saved = self.ivars().split_window_frames.borrow_mut();
                saved
                    .iter_mut()
                    .map(Option::take)
                    .collect::<Vec<Option<Rect>>>()
            };
            for (index, frame) in frames.into_iter().enumerate() {
                let Some(frame) = frame else {
                    continue;
                };
                let Some(window) = self.ivars().windows.borrow().get(index).cloned() else {
                    continue;
                };
                // Restore both the origin and size. AppKit may otherwise keep
                // the content width while shifting the outer frame as a split
                // item expands near a screen edge.
                unsafe {
                    let content: Rect = msg_send![window.as_object(), contentRectForFrameRect: frame];
                    self.set_window_content_extent(window.as_object(), content.size);
                    let _: () = msg_send![window.as_object(), setFrame: frame, display: true];
                }
            }
            self.ivars().split_restore_pending.set(false);
        }
    }
);

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
            "ready" | "empty" | "busy" | "error"
        ) {
            panic!("RINKA_APPKIT_SCENE_PROBE expects ready, empty, busy, or error");
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
        let probe = self.ivars().scene_probe.borrow();
        let Some(probe) = probe.as_ref() else {
            return;
        };
        eprintln!(
            "Rinka scene probe scene={} result={}",
            probe.expected_scene,
            if probe.passed { "PASS" } else { "FAIL" }
        );
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

fn rect_matches(left: Rect, right: Rect) -> bool {
    const TOLERANCE: f64 = 0.01;
    (left.origin.x - right.origin.x).abs() <= TOLERANCE
        && (left.origin.y - right.origin.y).abs() <= TOLERANCE
        && (left.size.width - right.size.width).abs() <= TOLERANCE
        && (left.size.height - right.size.height).abs() <= TOLERANCE
}

fn rect_size_matches(rect: Rect, size: Size) -> bool {
    const TOLERANCE: f64 = 0.01;
    (rect.size.width - size.width).abs() <= TOLERANCE
        && (rect.size.height - size.height).abs() <= TOLERANCE
}

unsafe fn split_item_collapsed(controller: *mut AnyObject, index: usize) -> bool {
    // SAFETY: The probe is enabled only for the three-item Workspace fixture.
    let items: *mut AnyObject = unsafe { msg_send![controller, splitViewItems] };
    let item: *mut AnyObject = unsafe { msg_send![items, objectAtIndex: index] };
    unsafe { msg_send![item, isCollapsed] }
}

unsafe fn set_split_item_collapsed(controller: *mut AnyObject, index: usize, collapsed: bool) {
    // SAFETY: The probe is enabled only for the three-item Workspace fixture.
    let items: *mut AnyObject = unsafe { msg_send![controller, splitViewItems] };
    let item: *mut AnyObject = unsafe { msg_send![items, objectAtIndex: index] };
    let _: () = unsafe { msg_send![item, setCollapsed: collapsed] };
}

type BuiltWindow = (
    Id,
    WindowRuntime<AppKitBackend>,
    Retained<ToolbarDelegate>,
    ListRegistry,
    Vec<Id>,
);

fn build_window(
    mtm: MainThreadMarker,
    spec: &WindowSpec,
    split_restore_pending: Rc<Cell<bool>>,
) -> Result<BuiltWindow, AppKitError> {
    let frame = Rect {
        origin: Point::default(),
        size: Size {
            width: spec.initial_size.width,
            height: spec.initial_size.height,
        },
    };
    let class = match spec.kind {
        WindowKind::Panel(_) => objc2::class!(NSPanel),
        WindowKind::Main | WindowKind::Preferences => objc2::class!(NSWindow),
    };
    let full_height_content = !matches!(spec.kind, WindowKind::Panel(_));
    let style_mask =
        1_usize | 2_usize | 4_usize | 8_usize | if full_height_content { 32768_usize } else { 0 };
    // SAFETY: initWithContentRect is the designated NSWindow/NSPanel initializer.
    let window = unsafe {
        let allocated: *mut AnyObject = msg_send![class, alloc];
        let pointer: *mut AnyObject = msg_send![allocated,
            initWithContentRect: frame,
            styleMask: style_mask,
            backing: 2_usize,
            defer: false
        ];
        Id::from_owned(pointer)
    };
    set_string(window.as_object(), "setTitle:", &spec.title);
    // SAFETY: Window geometry and Tahoe titlebar properties are public AppKit API.
    unsafe {
        let _: () = msg_send![window.as_object(), setReleasedWhenClosed: false];
        let _: () = msg_send![window.as_object(), setContentMinSize: Size {
            width: spec.minimum_size.width,
            height: spec.minimum_size.height,
        }];
        let _: () =
            msg_send![window.as_object(), setTitlebarAppearsTransparent: full_height_content];
        if full_height_content {
            let _: () = msg_send![window.as_object(), setToolbarStyle: 3_isize];
        }
    }

    if let WindowKind::Panel(behavior) = spec.kind {
        configure_panel(window.as_object(), behavior);
    }

    // SAFETY: Every NSWindow created above has a content view.
    let content: *mut AnyObject = unsafe { msg_send![window.as_object(), contentView] };
    // SAFETY: contentView is retained by its window; the backend takes another retain.
    let content = unsafe { Id::from_borrowed(content) };
    let list_registry = Rc::new(RefCell::new(Vec::new()));
    let renderer = Renderer::new(AppKitBackend::new(
        content.clone(),
        mtm,
        list_registry.clone(),
        split_restore_pending,
    ));
    let runtime = WindowRuntime::mount(renderer, spec.content.clone())
        .map_err(|error| AppKitError(error.to_string()))?;
    runtime.with_renderer(|renderer| {
        if let Some(root) = renderer.mounted() {
            refresh_mounted_stacks(root);
        }
    });
    let initial_content_size = Size {
        width: spec.initial_size.width,
        height: spec.initial_size.height,
    };
    let toolbar_delegate =
        runtime.with_renderer(|renderer| install_toolbar(window.as_object(), spec, mtm, renderer));
    let initial_extent_constraints = runtime.with_renderer(|renderer| {
        install_root_content_controller(window.as_object(), renderer, initial_content_size)
    })?;
    // Installing the retained content-view controller and toolbar allows
    // AppKit to resolve their native fitting sizes. Reassert the declarative
    // content size after that ownership graph is complete so Ready, Empty,
    // Busy, and Error cannot acquire different top-level window widths from
    // their scene-specific fitting content.
    unsafe {
        if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
            let frame: Rect = msg_send![window.as_object(), frame];
            let content: *mut AnyObject = msg_send![window.as_object(), contentView];
            let content_frame: Rect = msg_send![content, frame];
            eprintln!(
                "Rinka AppKit toolbar installed frame={frame:?} content_frame={content_frame:?}"
            );
        }
        let _: () = msg_send![window.as_object(), setContentSize: Size {
            width: spec.initial_size.width,
            height: spec.initial_size.height,
        }];
        if std::env::var_os("RINKA_APPKIT_TRACE").is_some() {
            let frame: Rect = msg_send![window.as_object(), frame];
            let content: *mut AnyObject = msg_send![window.as_object(), contentView];
            let content_frame: Rect = msg_send![content, frame];
            eprintln!(
                "Rinka AppKit toolbar size restored frame={frame:?} content_frame={content_frame:?}"
            );
        }
    }
    // The content view may have changed when the retained native controller
    // became the window's content-view controller.
    let content: *mut AnyObject = unsafe { msg_send![window.as_object(), contentView] };
    let content = unsafe { Id::from_borrowed(content) };

    // SAFETY: Show and place the fully-rendered native window. The application
    // delegate assigns key status after every auxiliary panel is ordered.
    unsafe {
        let _: () = msg_send![window.as_object(), center];
        let _: () = msg_send![window.as_object(), orderFront: std::ptr::null::<AnyObject>()];
        let _: () = msg_send![content.as_object(), layoutSubtreeIfNeeded];
        layout_scroll_documents(
            content.as_object(),
            std::env::var_os("RINKA_APPKIT_TRACE").is_some(),
        );
        let _: () = msg_send![content.as_object(), layoutSubtreeIfNeeded];
    }
    runtime.with_renderer(|renderer| {
        let root = renderer
            .mounted()
            .ok_or_else(|| AppKitError("window renderer has no mounted root".to_owned()))?;
        reapply_mounted_native_list_state(root)
    })?;
    let mounted_lists = list_registry_handles(&list_registry);
    refresh_all_semantic_sidebar_content_fit(&mounted_lists);
    unsafe {
        let _: () = msg_send![content.as_object(), layoutSubtreeIfNeeded];
        // Semantic Source fitting can move a native split divider after the
        // controller is installed. That transaction must consume the existing
        // content extent rather than adopting a scene-specific fitting width.
        let _: () = msg_send![window.as_object(), setContentSize: Size {
            width: spec.initial_size.width,
            height: spec.initial_size.height,
        }];
        let _: () = msg_send![content.as_object(), layoutSubtreeIfNeeded];
    }
    Ok((
        window,
        runtime,
        toolbar_delegate,
        list_registry,
        initial_extent_constraints,
    ))
}

fn install_root_content_controller(
    window: &AnyObject,
    renderer: &Renderer<AppKitBackend>,
    initial_content_size: Size,
) -> Result<Vec<Id>, AppKitError> {
    let root = renderer
        .mounted()
        .ok_or_else(|| AppKitError("window renderer has no mounted root".to_owned()))?;
    let handle = root.handle();
    // SAFETY: The temporary renderer host owns the mounted root only until a
    // native content-view controller takes over below.
    unsafe {
        let _: () = msg_send![handle.view(), removeFromSuperview];
    }
    let controller = if matches!(
        handle.element_kind(),
        Some(ElementKind::Split | ElementKind::Workspace)
    ) {
        handle
            .0
            .auxiliaries
            .first()
            .cloned()
            .ok_or_else(|| AppKitError("root split has no native controller".to_owned()))?
    } else {
        let controller = new_object(objc2::class!(NSViewController));
        let pane = create_safe_area_pane(handle.view());
        configure_growth(pane.as_object(), true, true);
        // SAFETY: The mounted root is retained inside a native container. The
        // controller owns that container while its child retains Rinka's
        // declarative extent independently from NSWindow's contentView frame.
        unsafe {
            let _: () = msg_send![controller.as_object(), setView: pane.as_object()];
        }
        controller
    };
    // SAFETY: NSWindow retains its content-view controller. Removing the root
    // from the temporary renderer host prevents dual view ownership. Declaring
    // the controller's intended content extent before attachment prevents
    // AppKit from deriving the top-level window size from scene-specific
    // intrinsic content during the ownership transfer.
    unsafe {
        let trace = std::env::var_os("RINKA_APPKIT_TRACE").is_some();
        if trace {
            let root_frame: Rect = msg_send![handle.view(), frame];
            let window_frame: Rect = msg_send![window, frame];
            eprintln!(
                "Rinka AppKit root attach begin root_frame={root_frame:?} window_frame={window_frame:?}"
            );
        }
        let _: () =
            msg_send![controller.as_object(), setPreferredContentSize: initial_content_size];
        let _: () = msg_send![handle.view(), setFrameSize: initial_content_size];
        if trace {
            let root_frame: Rect = msg_send![handle.view(), frame];
            eprintln!("Rinka AppKit root attach sized root_frame={root_frame:?}");
        }
        let _: () = msg_send![window, setContentViewController: controller.as_object()];
        let _: () = msg_send![window, setContentSize: initial_content_size];
        let content: *mut AnyObject = msg_send![window, contentView];
        let _: () = msg_send![content, setFrameSize: initial_content_size];
        // AppKit replaces the root ownership graph while assigning the
        // content-view controller and deactivates constraints attached to the
        // previous graph. Create the retained extent constraints only after
        // that transfer is complete.
        let sizing_view: *mut AnyObject = if matches!(
            handle.element_kind(),
            Some(ElementKind::Split | ElementKind::Workspace)
        ) {
            msg_send![controller.as_object(), splitView]
        } else {
            handle.0.view.as_ptr()
        };
        let initial_extent_constraints = vec![
            dimension_constant_constraint(
                msg_send![sizing_view, widthAnchor],
                initial_content_size.width,
                1000.0,
            ),
            dimension_constant_constraint(
                msg_send![sizing_view, heightAnchor],
                initial_content_size.height,
                1000.0,
            ),
        ];
        if trace {
            let root_frame: Rect = msg_send![handle.view(), frame];
            let translates: bool =
                msg_send![handle.view(), translatesAutoresizingMaskIntoConstraints];
            eprintln!(
                "Rinka AppKit root attach before-layout root_frame={root_frame:?} translates={translates}"
            );
        }
        if trace {
            let root_frame: Rect = msg_send![handle.view(), frame];
            let window_frame: Rect = msg_send![window, frame];
            eprintln!(
                "Rinka AppKit root attach end root_frame={root_frame:?} window_frame={window_frame:?}"
            );
        }
        finalize_split_mount(handle);
        Ok(initial_extent_constraints)
    }
}

fn finalize_split_mount(handle: &AppKitHandle) {
    let Some(configuration) = *handle.0.split_configuration.borrow() else {
        return;
    };
    let presentations = handle.0.presentations.borrow();
    // SAFETY: Items are retained by the mounted NSSplitViewController. The
    // sidebar's automatic resize collapse is enabled only after the controller
    // has received its real window extent.
    unsafe {
        for (index, presentation) in presentations.iter().enumerate() {
            let Some(item) = &presentation.owner else {
                continue;
            };
            match (configuration, index) {
                (
                    SplitConfiguration::Pair {
                        role: SplitRole::Navigation,
                        collapsible,
                    },
                    0,
                ) => {
                    let _: () = msg_send![item.as_object(), setCollapsed: false];
                    let _: () =
                        msg_send![item.as_object(), setCanCollapseFromWindowResize: collapsible];
                }
                (
                    SplitConfiguration::Workspace {
                        sidebar_collapsible,
                        ..
                    },
                    0,
                ) => {
                    let _: () = msg_send![item.as_object(), setCollapsed: false];
                    let _: () = msg_send![item.as_object(), setCanCollapseFromWindowResize: sidebar_collapsible];
                }
                (
                    SplitConfiguration::Pair {
                        role: SplitRole::Utility,
                        ..
                    },
                    1,
                )
                | (SplitConfiguration::Workspace { .. }, 2) => {
                    let _: () = msg_send![item.as_object(), setCollapsed: false];
                }
                _ => {}
            }
        }
        let _: () = msg_send![handle.view(), layoutSubtreeIfNeeded];
    }
}

unsafe fn trace_view_tree(view: &AnyObject, depth: usize) {
    if depth > 9 {
        return;
    }
    // SAFETY: Diagnostics only query NSView layout state on the main thread.
    let class_name: *mut AnyObject = unsafe { msg_send![view, className] };
    let frame: Rect = unsafe { msg_send![view, frame] };
    let fitting: Size = unsafe { msg_send![view, fittingSize] };
    let ambiguous: bool = unsafe { msg_send![view, hasAmbiguousLayout] };
    let horizontal_hugging: f32 =
        unsafe { msg_send![view, contentHuggingPriorityForOrientation: 0_isize] };
    let vertical_hugging: f32 =
        unsafe { msg_send![view, contentHuggingPriorityForOrientation: 1_isize] };
    eprintln!(
        "Rinka AppKit view depth={depth} class={} frame={frame:?} fitting={fitting:?} ambiguous={ambiguous} hugging=({horizontal_hugging},{vertical_hugging})",
        rust_string(class_name)
    );
    let is_table: bool = unsafe { msg_send![view, isKindOfClass: objc2::class!(NSTableView)] };
    if is_table {
        let columns: *mut AnyObject = unsafe { msg_send![view, tableColumns] };
        let column_count: usize = unsafe { msg_send![columns, count] };
        let row_count: isize = unsafe { msg_send![view, numberOfRows] };
        let row_height: f64 = unsafe { msg_send![view, rowHeight] };
        let row_size_style: isize = unsafe { msg_send![view, rowSizeStyle] };
        let effective_row_size_style: isize = unsafe { msg_send![view, effectiveRowSizeStyle] };
        let style: isize = unsafe { msg_send![view, style] };
        let effective_style: isize = unsafe { msg_send![view, effectiveStyle] };
        let intercell: Size = unsafe { msg_send![view, intercellSpacing] };
        eprintln!(
            "Rinka AppKit table columns={column_count} rows={row_count} row_height={row_height} row_size_style={row_size_style} effective_row_size_style={effective_row_size_style} style={style} effective_style={effective_style} intercell={intercell:?}"
        );
        for row in 0..row_count.min(8) {
            let row_rect: Rect = unsafe { msg_send![view, rectOfRow: row] };
            eprintln!("Rinka AppKit table row={row} rect={row_rect:?}");
        }
        for index in 0..column_count {
            let column: *mut AnyObject = unsafe { msg_send![columns, objectAtIndex: index] };
            let title: *mut AnyObject = unsafe { msg_send![column, title] };
            let width: f64 = unsafe { msg_send![column, width] };
            let minimum: f64 = unsafe { msg_send![column, minWidth] };
            eprintln!(
                "Rinka AppKit table column={index} title={} width={width} minimum={minimum}",
                rust_string(title)
            );
        }
    }
    let subviews: *mut AnyObject = unsafe { msg_send![view, subviews] };
    let count: usize = unsafe { msg_send![subviews, count] };
    for index in 0..count {
        let child: *mut AnyObject = unsafe { msg_send![subviews, objectAtIndex: index] };
        if let Some(child) = NonNull::new(child) {
            unsafe { trace_view_tree(child.as_ref(), depth + 1) };
        }
    }
}

unsafe fn trace_window_contract(window: &AnyObject) {
    // SAFETY: Diagnostics query public NSWindow, view-controller, split-item,
    // and toolbar properties on the AppKit main thread.
    let frame: Rect = unsafe { msg_send![window, frame] };
    let min_size: Size = unsafe { msg_send![window, minSize] };
    let max_size: Size = unsafe { msg_send![window, maxSize] };
    let content_min_size: Size = unsafe { msg_send![window, contentMinSize] };
    let content_max_size: Size = unsafe { msg_send![window, contentMaxSize] };
    eprintln!(
        "Rinka AppKit contract frame={frame:?} min_size={min_size:?} max_size={max_size:?} content_min_size={content_min_size:?} content_max_size={content_max_size:?}"
    );
    let controller: *mut AnyObject = unsafe { msg_send![window, contentViewController] };
    let Some(controller) = NonNull::new(controller) else {
        eprintln!("Rinka AppKit contract content_controller=nil");
        return;
    };
    let controller_class: *mut AnyObject = unsafe { msg_send![controller.as_ref(), className] };
    let is_split: bool = unsafe {
        msg_send![controller.as_ref(), isKindOfClass: objc2::class!(NSSplitViewController)]
    };
    eprintln!(
        "Rinka AppKit contract content_controller={} split={is_split}",
        rust_string(controller_class)
    );
    if is_split {
        let items: *mut AnyObject = unsafe { msg_send![controller.as_ref(), splitViewItems] };
        let count: usize = unsafe { msg_send![items, count] };
        eprintln!("Rinka AppKit contract split_items={count}");
        for index in 0..count {
            let item: *mut AnyObject = unsafe { msg_send![items, objectAtIndex: index] };
            let behavior: isize = unsafe { msg_send![item, behavior] };
            let collapsed: bool = unsafe { msg_send![item, isCollapsed] };
            let can_collapse: bool = unsafe { msg_send![item, canCollapse] };
            let resize_collapse: bool = unsafe { msg_send![item, canCollapseFromWindowResize] };
            let automatic_safe_area: bool =
                unsafe { msg_send![item, automaticallyAdjustsSafeAreaInsets] };
            let item_controller: *mut AnyObject = unsafe { msg_send![item, viewController] };
            let parent: *mut AnyObject =
                unsafe { msg_send![item_controller, parentViewController] };
            let view: *mut AnyObject = unsafe { msg_send![item_controller, view] };
            let frame: Rect = unsafe { msg_send![view, frame] };
            eprintln!(
                "Rinka AppKit contract split_item={index} behavior={behavior} collapsed={collapsed} can_collapse={can_collapse} resize_collapse={resize_collapse} automatic_safe_area={automatic_safe_area} parent_is_root={} frame={frame:?}",
                parent == controller.as_ptr()
            );
        }
    }
    let toolbar: *mut AnyObject = unsafe { msg_send![window, toolbar] };
    let Some(toolbar) = NonNull::new(toolbar) else {
        eprintln!("Rinka AppKit contract toolbar=nil");
        return;
    };
    let items: *mut AnyObject = unsafe { msg_send![toolbar.as_ref(), items] };
    let count: usize = unsafe { msg_send![items, count] };
    eprintln!("Rinka AppKit contract toolbar_items={count}");
    for index in 0..count {
        let item: *mut AnyObject = unsafe { msg_send![items, objectAtIndex: index] };
        let identifier: *mut AnyObject = unsafe { msg_send![item, itemIdentifier] };
        let target: *mut AnyObject = unsafe { msg_send![item, target] };
        let item_class: *mut AnyObject = unsafe { msg_send![item, className] };
        eprintln!(
            "Rinka AppKit contract toolbar_item={index} class={} identifier={} target_nil={}",
            rust_string(item_class),
            rust_string(identifier),
            target.is_null()
        );
    }
}

fn configure_panel(panel: &AnyObject, behavior: PanelBehavior) {
    // SAFETY: The receiver is an NSPanel and the values come from PanelBehavior.
    unsafe {
        let _: () = msg_send![panel, setFloatingPanel: behavior.floating];
        let _: () = msg_send![panel, setHidesOnDeactivate: behavior.hides_when_inactive];
        let _: () = msg_send![panel, setBecomesKeyOnlyIfNeeded: !behavior.accepts_keyboard];
    }
}

unsafe fn panel_contract_is_valid(panel: &AnyObject) -> bool {
    // SAFETY: The caller supplies the retained auxiliary window on AppKit's
    // main thread and reads only public NSPanel/NSWindow properties.
    let is_panel: bool = unsafe { msg_send![panel, isKindOfClass: objc2::class!(NSPanel)] };
    let can_become_key: bool = unsafe { msg_send![panel, canBecomeKeyWindow] };
    let floating: bool = unsafe { msg_send![panel, isFloatingPanel] };
    let key_only_if_needed: bool = unsafe { msg_send![panel, becomesKeyOnlyIfNeeded] };
    let hides_on_deactivate: bool = unsafe { msg_send![panel, hidesOnDeactivate] };
    is_panel && can_become_key && floating && !key_only_if_needed && !hides_on_deactivate
}

fn install_toolbar(
    window: &AnyObject,
    spec: &WindowSpec,
    mtm: MainThreadMarker,
    renderer: &Renderer<AppKitBackend>,
) -> Retained<ToolbarDelegate> {
    let sidebar_controller = renderer
        .mounted()
        .and_then(|root| split_controller_for(root, SplitRole::Navigation));
    let inspector_controller = renderer
        .mounted()
        .and_then(|root| split_controller_for(root, SplitRole::Utility));
    let delegate = ToolbarDelegate::new(
        mtm,
        spec.toolbar.clone(),
        sidebar_controller,
        inspector_controller,
    );
    let has_split_controls = delegate.ivars().sidebar_controller.is_some()
        || delegate.ivars().inspector_controller.is_some();
    if !should_install_toolbar(spec.kind, spec.toolbar.len(), has_split_controls) {
        return delegate;
    }
    let identifier = ns_string(&format!("jp.bunko.rinka.{}", spec.id.as_str()));
    // SAFETY: The delegate supplies native items for custom identifiers.
    // NSToolbar owns its items and NSWindow owns the toolbar; the host retains
    // the toolbar's weak delegate for the lifetime of the window.
    unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSToolbar), alloc];
        let toolbar: *mut AnyObject =
            msg_send![allocated, initWithIdentifier: identifier.as_object()];
        let _: () = msg_send![toolbar, setDelegate: &*delegate];
        let _: () = msg_send![toolbar, setAllowsUserCustomization: false];
        let _: () = msg_send![toolbar, setAutosavesConfiguration: false];
        let _: () = msg_send![toolbar,
            setDisplayMode: native_toolbar_display(spec.toolbar_display)
        ];
        let centered_identifiers = spec
            .toolbar
            .iter()
            .filter(|item| item.placement == ToolbarPlacement::Center)
            .map(|item| ns_string(&toolbar_identifier(&item.id)))
            .collect::<Vec<_>>();
        if !centered_identifiers.is_empty() {
            let identifiers = ns_array(&centered_identifiers);
            let set: *mut AnyObject = msg_send![objc2::class!(NSSet),
                setWithArray: identifiers.as_object()
            ];
            let _: () = msg_send![toolbar, setCenteredItemIdentifiers: set];
        }
        let _: () = msg_send![window, setToolbar: toolbar];
        let _: () = msg_send![toolbar, release];
    }
    delegate
}

fn should_install_toolbar(
    kind: WindowKind,
    custom_item_count: usize,
    has_split_controls: bool,
) -> bool {
    !matches!(kind, WindowKind::Panel(_)) && (custom_item_count > 0 || has_split_controls)
}

const fn native_toolbar_display(display: ToolbarDisplay) -> isize {
    match display {
        ToolbarDisplay::Automatic => 0,
        ToolbarDisplay::IconAndLabel => 1,
        ToolbarDisplay::IconOnly => 2,
        ToolbarDisplay::LabelOnly => 3,
    }
}

fn split_controller_for(node: &MountedNode<AppKitHandle>, role: SplitRole) -> Option<Id> {
    if node.handle().element_kind() == Some(ElementKind::Workspace)
        || node.handle().0.split_role == Some(role)
    {
        return node.handle().0.auxiliaries.first().cloned();
    }
    node.children()
        .iter()
        .find_map(|child| split_controller_for(child, role))
}

fn mounted_handle_for_key<'a>(
    node: &'a MountedNode<AppKitHandle>,
    key: &str,
) -> Option<&'a AppKitHandle> {
    if node
        .element()
        .key()
        .is_some_and(|candidate| candidate.as_str() == key)
    {
        return Some(node.handle());
    }
    node.children()
        .iter()
        .find_map(|child| mounted_handle_for_key(child, key))
}

fn mounted_scene(node: &MountedNode<AppKitHandle>) -> Option<&'static str> {
    [
        ("file-list", "ready"),
        ("directory-empty", "empty"),
        ("directory-busy", "busy"),
        ("directory-error", "error"),
    ]
    .into_iter()
    .find_map(|(key, scene)| mounted_handle_for_key(node, key).map(|_| scene))
}

unsafe fn window_geometry_is_valid(window: &AnyObject) -> bool {
    // SAFETY: The caller supplies an NSWindow retained by the application
    // delegate and invokes this helper on AppKit's main thread.
    let frame: Rect = unsafe { msg_send![window, frame] };
    if !rect_is_finite(frame) || frame.size.width <= 0.0 || frame.size.height <= 0.0 {
        eprintln!("Rinka geometry invalid window frame={frame:?}");
        return false;
    }
    let content: *mut AnyObject = unsafe { msg_send![window, contentView] };
    NonNull::new(content).is_some_and(|content| unsafe { view_geometry_is_valid(content.as_ref()) })
}

unsafe fn view_geometry_is_valid(view: &AnyObject) -> bool {
    // SAFETY: The traversal follows retained NSView subviews on AppKit's main
    // thread and performs read-only geometry and Auto Layout queries.
    let frame: Rect = unsafe { msg_send![view, frame] };
    let ambiguous: bool = unsafe { msg_send![view, hasAmbiguousLayout] };
    let translates: bool = unsafe { msg_send![view, translatesAutoresizingMaskIntoConstraints] };
    if !rect_is_finite(frame)
        || frame.size.width < 0.0
        || frame.size.height < 0.0
        || (ambiguous && !translates)
    {
        let class_name: *mut AnyObject = unsafe { msg_send![view, className] };
        eprintln!(
            "Rinka geometry invalid view_class={} frame={frame:?} ambiguous={ambiguous} translates={translates}",
            rust_string(class_name)
        );
        return false;
    }
    let subviews: *mut AnyObject = unsafe { msg_send![view, subviews] };
    let Some(subviews) = NonNull::new(subviews) else {
        return true;
    };
    let count: usize = unsafe { msg_send![subviews.as_ref(), count] };
    (0..count).all(|index| {
        let child: *mut AnyObject = unsafe { msg_send![subviews.as_ref(), objectAtIndex: index] };
        NonNull::new(child).is_some_and(|child| unsafe { view_geometry_is_valid(child.as_ref()) })
    })
}

fn rect_is_finite(rect: Rect) -> bool {
    rect.origin.x.is_finite()
        && rect.origin.y.is_finite()
        && rect.size.width.is_finite()
        && rect.size.height.is_finite()
}

fn refresh_mounted_stacks(node: &MountedNode<AppKitHandle>) {
    for child in node.children() {
        refresh_mounted_stacks(child);
    }
    if node.handle().element_kind() == Some(ElementKind::Stack) {
        refresh_stack_container_constraints(node.handle());
        refresh_stack_constraints(node.handle());
    }
}

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
