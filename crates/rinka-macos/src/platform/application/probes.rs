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
    accelerator_router: Rc<RefCell<AcceleratorRouter>>,
    window_identities: WindowIdentityRegistry,
    key_monitor: RefCell<Option<Id>>,
    transition_probe: RefCell<Option<TransitionProbe>>,
    scene_probe: RefCell<Option<SceneProbe>>,
    accelerator_probe: RefCell<Option<AcceleratorProbe>>,
}

#[derive(Clone, Copy, Debug)]
struct AcceleratorProbe {
    step: usize,
    attempts: usize,
    passed: bool,
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

const fn transition_step(
    label: &'static str,
    sidebar_collapsed: bool,
    inspector_collapsed: bool,
    action: TransitionProbeAction,
) -> TransitionProbeStep {
    TransitionProbeStep {
        label,
        sidebar_collapsed,
        inspector_collapsed,
        action,
    }
}

use TransitionProbeAction::{CompletePhase, ToggleInspector, ToggleSidebar};

const TRANSITION_PROBE_STEPS: [TransitionProbeStep; 24] = [
    transition_step("sidebar-cycle-1-hidden", true, false, ToggleSidebar),
    transition_step("sidebar-cycle-1-restored", false, false, ToggleSidebar),
    transition_step("sidebar-cycle-2-hidden", true, false, ToggleSidebar),
    transition_step("sidebar-cycle-2-restored", false, false, ToggleSidebar),
    transition_step("sidebar-cycle-3-hidden", true, false, ToggleSidebar),
    transition_step("sidebar-cycle-3-restored", false, false, ToggleInspector),
    transition_step("inspector-cycle-1-hidden", false, true, ToggleInspector),
    transition_step("inspector-cycle-1-restored", false, false, ToggleInspector),
    transition_step("inspector-cycle-2-hidden", false, true, ToggleInspector),
    transition_step("inspector-cycle-2-restored", false, false, ToggleInspector),
    transition_step("inspector-cycle-3-hidden", false, true, ToggleInspector),
    transition_step("inspector-cycle-3-restored", false, false, ToggleSidebar),
    transition_step("combined-cycle-1-sidebar-hidden", true, false, ToggleInspector),
    transition_step("combined-cycle-1-both-hidden", true, true, ToggleSidebar),
    transition_step("combined-cycle-1-inspector-hidden", false, true, ToggleInspector),
    transition_step("combined-cycle-1-restored", false, false, ToggleSidebar),
    transition_step("combined-cycle-2-sidebar-hidden", true, false, ToggleInspector),
    transition_step("combined-cycle-2-both-hidden", true, true, ToggleSidebar),
    transition_step("combined-cycle-2-inspector-hidden", false, true, ToggleInspector),
    transition_step("combined-cycle-2-restored", false, false, ToggleSidebar),
    transition_step("combined-cycle-3-sidebar-hidden", true, false, ToggleInspector),
    transition_step("combined-cycle-3-both-hidden", true, true, ToggleSidebar),
    transition_step("combined-cycle-3-inspector-hidden", false, true, ToggleInspector),
    transition_step("combined-cycle-3-restored", false, false, CompletePhase),
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
                        layout_scroll_documents(content.as_ref());
                        let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
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
                }
            }
            self.begin_transition_probe();
            self.begin_scene_probe();
            self.begin_accelerator_probe();
            self.begin_context_menu_probe();
        }

        #[unsafe(method(runTransitionProbe:))]
        fn run_transition_probe(&self, _sender: *mut AnyObject) {
            self.advance_transition_probe();
        }

        #[unsafe(method(runSceneProbe:))]
        fn run_scene_probe(&self, _sender: *mut AnyObject) {
            self.advance_scene_probe();
        }

        #[unsafe(method(runAcceleratorProbe:))]
        fn run_accelerator_probe(&self, _sender: *mut AnyObject) {
            self.advance_accelerator_probe();
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
                let content: *mut AnyObject = msg_send![window.as_ref(), contentView];
                let Some(content) = NonNull::new(content) else {
                    return;
                };
                let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
                layout_scroll_documents(content.as_ref());
                let _: () = msg_send![content.as_ref(), layoutSubtreeIfNeeded];
            }
            let list_handles = registered_list_handles(&self.ivars().list_registries);
            refresh_all_semantic_sidebar_content_fit(&list_handles);
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
                        *delegate.ivars().pattern.borrow(),
                        CollectionPattern::NavigationSidebar | CollectionPattern::Outline | CollectionPattern::DataTable
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
                    *delegate.ivars().pattern.borrow(),
                    CollectionPattern::NavigationSidebar | CollectionPattern::Outline | CollectionPattern::DataTable
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
