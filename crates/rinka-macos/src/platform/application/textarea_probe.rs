// Live text-area diagnostic driven by `RINKA_APPKIT_TEXTAREA_PROBE`.
//
// The probe operates only this process's own windows: text is typed through
// NSTextView's input-client entry points, buttons fire through their stable
// event bindings, key events travel through NSWindow sendEvent:, and IME
// composition is driven through the same setMarkedText/insertText calls a
// real input method uses. Nothing touches the user's desktop.

/// `NSNotFound`, the sentinel range location for "at the current selection".
const NS_NOT_FOUND: usize = isize::MAX as usize;

#[derive(Debug)]
struct TextAreaProbe {
    step: usize,
    attempts: usize,
    passed: bool,
    typed_at: Option<std::time::Instant>,
    baseline_revision: TextRevision,
    /// Keeps the latency-critical activity assertion alive for the probe.
    _activity: Option<Id>,
}

/// Finds the mounted node carrying one declarative key.
fn mounted_node_for_key<'tree>(
    node: &'tree MountedNode<AppKitHandle>,
    key: &str,
) -> Option<&'tree MountedNode<AppKitHandle>> {
    if node
        .element()
        .key()
        .is_some_and(|candidate| candidate.as_str() == key)
    {
        return Some(node);
    }
    node.children()
        .iter()
        .find_map(|child| mounted_node_for_key(child, key))
}

impl ApplicationDelegate {
    fn begin_text_area_probe(&self) {
        if std::env::var_os("RINKA_APPKIT_TEXTAREA_PROBE").is_none()
            || self.ivars().text_area_probe.borrow().is_some()
        {
            return;
        }
        if std::env::var_os("RINKA_APPKIT_SCENE_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_TRANSITION_PROBE").is_some()
            || std::env::var_os("RINKA_APPKIT_ACCELERATOR_PROBE").is_some()
        {
            panic!("the text-area probe must run in its own process");
        }
        // Latency measurements model a foreground editing session; without
        // this assertion App Nap throttles the deferred-emission timers of a
        // backgrounded diagnostic process by hundreds of milliseconds.
        // SAFETY: beginActivityWithOptions returns a token that NSArray-style
        // retention keeps alive inside the probe for the process lifetime.
        let activity = unsafe {
            let info: *mut AnyObject = msg_send![objc2::class!(NSProcessInfo), processInfo];
            let reason = ns_string("rinka text-area latency probe");
            // NSActivityUserInitiated | NSActivityLatencyCritical.
            let options: u64 = 0x00FF_FFFF | 0xFF_0000_0000;
            let token: *mut AnyObject = msg_send![info,
                beginActivityWithOptions: options,
                reason: reason.as_object()
            ];
            NonNull::new(token).map(|token| Id::from_borrowed(token.as_ptr()))
        };
        *self.ivars().text_area_probe.borrow_mut() = Some(TextAreaProbe {
            step: 0,
            attempts: 0,
            passed: true,
            typed_at: None,
            baseline_revision: TextRevision::default(),
            _activity: activity,
        });
        self.schedule_text_area_probe(0.2);
    }

    fn schedule_text_area_probe(&self, delay: f64) {
        // SAFETY: The delayed selector runs on the main run loop with self
        // retained by the perform request.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(runTextAreaProbe:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: delay
            ];
        }
    }

    /// Returns the mounted editor text area's retained pieces, cloned out of
    /// the renderer borrow so probe actions can re-enter reconciliation.
    fn probe_text_area(&self) -> Option<(Retained<TextAreaDelegate>, Id)> {
        let renderers = self.ivars().renderers.borrow();
        let runtime = renderers.first()?;
        runtime.with_renderer(|renderer| {
            let root = renderer.mounted()?;
            let handle = mounted_handle_for_key(root, "editor-textarea")?;
            let delegate = handle.0.text_delegate.borrow().clone()?;
            let view = delegate.text_view()?;
            Some((delegate, view))
        })
    }

    /// Reads one value from the mounted editor text area's descriptor.
    fn probe_text_area_descriptor<R>(
        &self,
        read: impl FnOnce(&TextContent, Option<TextSelection>, usize) -> R,
    ) -> Option<R> {
        let renderers = self.ivars().renderers.borrow();
        let runtime = renderers.first()?;
        runtime.with_renderer(|renderer| {
            let root = renderer.mounted()?;
            let node = mounted_node_for_key(root, "editor-textarea")?;
            match node.element().props() {
                Props::TextArea {
                    content,
                    selection,
                    spans,
                    ..
                } => Some(read(content, *selection, spans.spans().len())),
                _ => None,
            }
        })
    }

    /// Clones the stable event bindings of one mounted element so they can
    /// be fired after the renderer borrow is released.
    fn probe_bindings(&self, key: &str) -> Option<EventBindings> {
        let renderers = self.ivars().renderers.borrow();
        let runtime = renderers.first()?;
        runtime.with_renderer(|renderer| {
            let root = renderer.mounted()?;
            mounted_node_for_key(root, key).map(|node| node.events().clone())
        })
    }

    fn text_area_probe_failure(&self, reason: &str) {
        eprintln!("Rinka textarea probe step failed: {reason}");
        if let Some(probe) = self.ivars().text_area_probe.borrow_mut().as_mut() {
            probe.passed = false;
        }
        self.finish_text_area_probe();
    }

    fn finish_text_area_probe(&self) {
        {
            let probe = self.ivars().text_area_probe.borrow();
            let Some(probe) = probe.as_ref() else {
                return;
            };
            eprintln!(
                "Rinka textarea probe result={} steps={}",
                if probe.passed { "PASS" } else { "FAIL" },
                probe.step
            );
        }
        self.capture_windows_to_directory("textarea-final-");
        if std::env::var_os("RINKA_APPKIT_TEXTAREA_PROBE_HOLD").is_none() {
            // SAFETY: Diagnostic completion terminates only this test app.
            unsafe {
                let application: *mut AnyObject =
                    msg_send![objc2::class!(NSApplication), sharedApplication];
                let _: () = msg_send![application, terminate: std::ptr::null::<AnyObject>()];
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn advance_text_area_probe(&self) {
        let Some((step, typed_at, baseline_revision)) = self
            .ivars()
            .text_area_probe
            .borrow()
            .as_ref()
            .map(|probe| (probe.step, probe.typed_at, probe.baseline_revision))
        else {
            return;
        };
        let Some((delegate, view)) = self.probe_text_area() else {
            self.text_area_probe_failure("editor text area is not mounted");
            return;
        };
        let advance = |next: usize, delay: f64| {
            if let Some(probe) = self.ivars().text_area_probe.borrow_mut().as_mut() {
                probe.step = next;
                probe.attempts = 0;
            }
            self.schedule_text_area_probe(delay);
        };
        let retry = |label: &str| {
            let mut probe = self.ivars().text_area_probe.borrow_mut();
            let Some(probe) = probe.as_mut() else {
                return false;
            };
            probe.attempts += 1;
            if probe.attempts < 200 {
                true
            } else {
                eprintln!("Rinka textarea probe timeout step={label}");
                false
            }
        };
        match step {
            // Type one character through the real input path and stamp the
            // clock for the change-event round trip.
            0 => {
                let baseline = delegate.ivars().revision.get();
                // SAFETY: First-responder transfer and text insertion address
                // this process's own live window and text view.
                unsafe {
                    let window: *mut AnyObject = msg_send![view.as_object(), window];
                    if !window.is_null() {
                        let _: bool =
                            msg_send![window, makeFirstResponder: view.as_object()];
                    }
                    // The freshly opened document owes TextKit one full layout
                    // pass; a keystroke measured while that debt is pending
                    // would charge document-open cost to the edit round trip.
                    // Paying it here measures the steady editing state.
                    let layout: *mut AnyObject =
                        msg_send![view.as_object(), layoutManager];
                    let container: *mut AnyObject =
                        msg_send![view.as_object(), textContainer];
                    if !layout.is_null() && !container.is_null() {
                        let full_layout_started = std::time::Instant::now();
                        let _: NSRange =
                            msg_send![layout, glyphRangeForTextContainer: container];
                        eprintln!(
                            "Rinka textarea probe initial-full-layout micros={}",
                            full_layout_started.elapsed().as_micros()
                        );
                    }
                    delegate
                        .ivars()
                        .probe_typed_at
                        .set(Some(std::time::Instant::now()));
                    let text = ns_string("x");
                    let _: () = msg_send![view.as_object(),
                        insertText: text.as_object(),
                        replacementRange: NSRange { location: NS_NOT_FOUND, length: 0 }
                    ];
                }
                if let Some(probe) = self.ivars().text_area_probe.borrow_mut().as_mut() {
                    probe.typed_at = Some(std::time::Instant::now());
                    probe.baseline_revision = baseline;
                }
                advance(1, 0.0);
            }
            // Await the echo: the application received the delta, applied it,
            // and its re-render was recognized without touching the buffer.
            1 => {
                let expected = baseline_revision.next_edit();
                let echoed = delegate.ivars().revision.get() == expected
                    && self
                        .probe_text_area_descriptor(|content, _, _| content.revision())
                        .is_some_and(|revision| revision == expected);
                if echoed {
                    // The causal round-trip number is printed by the drain;
                    // this step only sequences (its own polling latency is
                    // not part of the round trip). Record the observation
                    // lag and document size for context.
                    let elapsed = typed_at.map_or(0, |at| at.elapsed().as_micros());
                    let chars = self
                        .probe_text_area_descriptor(|content, _, _| content.char_len())
                        .unwrap_or(0);
                    eprintln!(
                        "Rinka textarea probe echo observed micros={elapsed} chars={chars}"
                    );
                    advance(2, 0.05);
                } else if retry("echo") {
                    self.schedule_text_area_probe(0.0);
                } else {
                    self.text_area_probe_failure("typed character never echoed");
                }
            }
            // Recompute and adopt the full document highlight.
            2 => {
                let Some(bindings) = self.probe_bindings("editor-rehighlight") else {
                    self.text_area_probe_failure("rehighlight button is not mounted");
                    return;
                };
                let started = std::time::Instant::now();
                bindings.emit_activate();
                let spans = self
                    .probe_text_area_descriptor(|_, _, span_count| span_count)
                    .unwrap_or(0);
                eprintln!(
                    "Rinka textarea probe rehighlight micros={} spans={spans}",
                    started.elapsed().as_micros()
                );
                advance(3, 0.05);
            }
            // Replace the whole document programmatically.
            3 => {
                let Some(bindings) = self.probe_bindings("editor-reload") else {
                    self.text_area_probe_failure("reload button is not mounted");
                    return;
                };
                let started = std::time::Instant::now();
                bindings.emit_activate();
                eprintln!(
                    "Rinka textarea probe reload micros={}",
                    started.elapsed().as_micros()
                );
                advance(4, 0.05);
            }
            // Programmatic selection set: jump to the end scrolls the caret.
            4 => {
                let Some(bindings) = self.probe_bindings("editor-jump-end") else {
                    self.text_area_probe_failure("jump button is not mounted");
                    return;
                };
                bindings.emit_activate();
                // SAFETY: Selection and length queries on the live text view.
                let (selected, length) = unsafe {
                    let selected: NSRange = msg_send![view.as_object(), selectedRange];
                    let string: *mut AnyObject = msg_send![view.as_object(), string];
                    let length: usize = msg_send![string, length];
                    (selected, length)
                };
                let passed = selected.location == length && selected.length == 0;
                eprintln!(
                    "Rinka textarea probe selection-set result={}",
                    if passed { "PASS" } else { "FAIL" }
                );
                if !passed {
                    self.text_area_probe_failure("caret did not reach the document end");
                    return;
                }
                advance(5, 0.05);
            }
            // Native selection get: a user-side selection change must reach
            // the application's controlled state.
            5 => {
                // SAFETY: setSelectedRange addresses the live text view; the
                // resulting native notification drives the reactive round trip.
                unsafe {
                    let _: () = msg_send![view.as_object(), setSelectedRange: NSRange {
                        location: 5,
                        length: 10,
                    }];
                }
                advance(6, 0.05);
            }
            6 => {
                let stored = self
                    .probe_text_area_descriptor(|_, selection, _| selection)
                    .flatten();
                let passed = stored == Some(TextSelection::new(5, 15));
                eprintln!(
                    "Rinka textarea probe selection-get result={}",
                    if passed { "PASS" } else { "FAIL" }
                );
                if !passed {
                    self.text_area_probe_failure("native selection never reached the application");
                    return;
                }
                advance(7, 0.05);
            }
            // Read-only rejects a real key event while selection stays alive.
            7 => {
                let Some(bindings) = self.probe_bindings("editor-readonly") else {
                    self.text_area_probe_failure("read-only toggle is not mounted");
                    return;
                };
                bindings.emit_toggle(true);
                let editable: bool = {
                    // SAFETY: isEditable is a main-thread query on the view.
                    unsafe { msg_send![view.as_object(), isEditable] }
                };
                if editable {
                    self.text_area_probe_failure("read-only did not disable editing");
                    return;
                }
                let baseline = delegate.ivars().revision.get();
                if let Some(probe) = self.ivars().text_area_probe.borrow_mut().as_mut() {
                    probe.baseline_revision = baseline;
                }
                // SAFETY: The synthetic key event is dispatched through this
                // process's own window; no global event posting occurs.
                unsafe {
                    let window: *mut AnyObject = msg_send![view.as_object(), window];
                    if !window.is_null() {
                        let _: bool = msg_send![window, makeFirstResponder: view.as_object()];
                        let window_number: isize = msg_send![window, windowNumber];
                        let characters = ns_string("a");
                        // NSEventTypeKeyDown = 10.
                        let event: *mut AnyObject = msg_send![objc2::class!(NSEvent),
                            keyEventWithType: 10_usize,
                            location: Point::default(),
                            modifierFlags: 0_usize,
                            timestamp: 0.0_f64,
                            windowNumber: window_number,
                            context: std::ptr::null::<AnyObject>(),
                            characters: characters.as_object(),
                            charactersIgnoringModifiers: characters.as_object(),
                            isARepeat: false,
                            keyCode: 0_u16
                        ];
                        let _: () = msg_send![window, sendEvent: event];
                    }
                }
                advance(8, 0.05);
            }
            8 => {
                let unchanged = delegate.ivars().revision.get() == baseline_revision;
                // Selection must stay available while edits are rejected.
                // SAFETY: Selection calls address the live text view.
                let selectable = unsafe {
                    let _: () = msg_send![view.as_object(), setSelectedRange: NSRange {
                        location: 0,
                        length: 4,
                    }];
                    let selected: NSRange = msg_send![view.as_object(), selectedRange];
                    selected.location == 0 && selected.length == 4
                };
                let passed = unchanged && selectable;
                eprintln!(
                    "Rinka textarea probe read-only result={} edits_rejected={unchanged} selection_alive={selectable}",
                    if passed { "PASS" } else { "FAIL" }
                );
                if !passed {
                    self.text_area_probe_failure("read-only broke editing rejection or selection");
                    return;
                }
                if let Some(bindings) = self.probe_bindings("editor-readonly") {
                    bindings.emit_toggle(false);
                }
                advance(9, 0.05);
            }
            // IME composition: mark Japanese preedit text through the same
            // NSTextInputClient entry point a real input method uses.
            9 => {
                // SAFETY: Marked-text calls address the live text view.
                unsafe {
                    let window: *mut AnyObject = msg_send![view.as_object(), window];
                    if !window.is_null() {
                        let _: bool = msg_send![window, makeFirstResponder: view.as_object()];
                    }
                    let preedit = ns_string("にほんご");
                    let _: () = msg_send![view.as_object(),
                        setMarkedText: preedit.as_object(),
                        selectedRange: NSRange { location: 4, length: 0 },
                        replacementRange: NSRange { location: NS_NOT_FOUND, length: 0 }
                    ];
                }
                if !delegate.has_marked_text() {
                    self.text_area_probe_failure("marked text was not established");
                    return;
                }
                eprintln!("Rinka textarea probe ime-marked result=PASS");
                self.capture_windows_to_directory("textarea-ime-marked-");
                advance(10, 0.1);
            }
            // The composing delta echoed and re-rendered; the composition
            // must still be alive.
            10 => {
                let preserved = delegate.has_marked_text();
                eprintln!(
                    "Rinka textarea probe ime-echo-preserves-composition result={}",
                    if preserved { "PASS" } else { "FAIL" }
                );
                if !preserved {
                    self.text_area_probe_failure("a re-render clobbered the IME composition");
                    return;
                }
                // SAFETY: insertText commits the composition on the live view.
                unsafe {
                    let committed = ns_string("日本語");
                    let _: () = msg_send![view.as_object(),
                        insertText: committed.as_object(),
                        replacementRange: NSRange { location: NS_NOT_FOUND, length: 0 }
                    ];
                }
                advance(11, 0.1);
            }
            11 => {
                let composition_closed = !delegate.has_marked_text();
                let committed = self
                    .probe_text_area_descriptor(|content, _, _| {
                        content.text().contains("日本語")
                    })
                    .unwrap_or(false);
                let passed = composition_closed && committed;
                eprintln!(
                    "Rinka textarea probe ime-commit result={}",
                    if passed { "PASS" } else { "FAIL" }
                );
                self.capture_windows_to_directory("textarea-ime-committed-");
                if !passed {
                    self.text_area_probe_failure(
                        "committed text never reached the application state",
                    );
                    return;
                }
                advance(12, 0.05);
            }
            _ => {
                self.finish_text_area_probe();
            }
        }
    }
}
