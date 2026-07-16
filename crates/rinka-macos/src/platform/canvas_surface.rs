// Owned-drawing canvas realized as an NSView subclass painting in drawRect:.
//
// The canvas is reserved for inherently graphical content (terminal cell
// grids, meters, widget faces). It never imitates a native control; the
// adapter replays the recorded platform-neutral scene and nothing else.

unsafe extern "C" {
    #[link_name = "NSFontAttributeName"]
    static FONT_ATTRIBUTE_NAME: *mut AnyObject;
    #[link_name = "NSForegroundColorAttributeName"]
    static FOREGROUND_COLOR_ATTRIBUTE_NAME: *mut AnyObject;
    #[link_name = "NSAccessibilityImageRole"]
    static ACCESSIBILITY_IMAGE_ROLE: *mut AnyObject;
    #[link_name = "NSAccessibilityTextAreaRole"]
    static ACCESSIBILITY_TEXT_AREA_ROLE: *mut AnyObject;
}

/// `NSTrackingMouseMoved | NSTrackingActiveInKeyWindow | NSTrackingInVisibleRect`.
/// The visible-rect option keeps the tracking region synchronized with the
/// view automatically, so no updateTrackingAreas override is required.
const CANVAS_TRACKING_OPTIONS: usize = 0x02 | 0x20 | 0x200;

const MODIFIER_FLAG_SHIFT: usize = 1 << 17;
const MODIFIER_FLAG_CONTROL: usize = 1 << 18;
const MODIFIER_FLAG_OPTION: usize = 1 << 19;
const MODIFIER_FLAG_COMMAND: usize = 1 << 20;

objc2::extern_class!(
    /// AppKit event-handling superclass in the NSView inheritance chain.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct NSResponder;
);

objc2::extern_class!(
    /// AppKit drawing surface superclass of the canvas view.
    #[unsafe(super(NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct NSView;
);

struct CanvasViewIvars {
    size: Cell<CanvasSize>,
    scene: RefCell<DrawScene>,
    events: EventBindings,
    /// Whether the canvas participates in keyboard focus and text input.
    accepts_input: Cell<bool>,
    /// App-declared candidate-window anchor served to the input method.
    ime_caret: Cell<Option<CanvasRect>>,
    /// Current preedit text of the active composition, empty outside one.
    marked_text: RefCell<String>,
    /// Present only while one keyDown: is being interpreted; captures the
    /// text the input context inserted for it and whether a composition
    /// consumed it.
    pending_key: RefCell<Option<PendingCanvasKey>>,
}

/// Raw facts of one key-down being interpreted by the input context.
struct PendingCanvasKey {
    key: Option<KeyIdentity>,
    modifiers: Modifiers,
    repeat: bool,
    /// Text the input context inserted for this key press, outside any
    /// composition.
    text: Option<String>,
    /// Whether this key press began, updated, or committed a composition.
    composition: bool,
}

/// Holds the protocol declaration so the lint scope is explicit: the trait
/// docs do carry a `# Safety` section, but `extern_protocol!` re-attaches
/// them during expansion where clippy no longer recognizes it.
#[allow(clippy::missing_safety_doc)]
mod text_input_protocol {
    objc2::extern_protocol!(
        /// AppKit's text-input-client protocol.
        ///
        /// Adopting it (the `unsafe impl` block inside `define_class!` of
        /// the canvas view) is what makes `NSView.inputContext` return a
        /// live `NSTextInputContext` for the canvas, routing every focused
        /// key-down through the active operating-system input method. The
        /// trait declares no Rust methods because rinka only implements the
        /// protocol, never messages it.
        ///
        /// # Safety
        ///
        /// An implementing type must be an Objective-C object that satisfies
        /// every required `NSTextInputClient` selector; `CanvasView`
        /// implements them all in its `define_class!` protocol block.
        // SAFETY: `NSTextInputClient` is an existing AppKit protocol.
        pub(crate) unsafe trait NSTextInputClient {}
    );
}

use text_input_protocol::NSTextInputClient;

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = CanvasViewIvars]
    struct CanvasView;

    impl CanvasView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            // Scene coordinates are top-left origin with y increasing down.
            true
        }

        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size(&self) -> Size {
            let size = self.ivars().size.get();
            Size {
                width: size.width,
                height: size.height,
            }
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: *mut AnyObject) -> bool {
            // Graphical content reacts to the click that focuses the window.
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: Rect) {
            self.replay_scene();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &AnyObject) {
            if self.ivars().accepts_input.get() {
                // Clicking an input-accepting canvas moves keyboard focus to
                // it, matching native text-field behavior.
                // SAFETY: The window is queried and messaged on the main
                // thread; makeFirstResponder: accepts any responder.
                unsafe {
                    let window: *mut AnyObject = msg_send![self, window];
                    if !window.is_null() {
                        let _: bool = msg_send![window, makeFirstResponder: self];
                    }
                }
            }
            self.emit_pointer(event, PointerPhase::Down, PointerButton::Primary);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Up, PointerButton::Primary);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Drag, PointerButton::Primary);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Down, PointerButton::Secondary);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Up, PointerButton::Secondary);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Drag, PointerButton::Secondary);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Down, PointerButton::Middle);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Up, PointerButton::Middle);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Drag, PointerButton::Middle);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Move, PointerButton::None);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Scroll, PointerButton::None);
        }

        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, info: *mut AnyObject) -> usize {
            drop_session_operation(&self.ivars().events, info)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, info: *mut AnyObject) -> usize {
            drop_session_operation(&self.ivars().events, info)
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_for_drag_operation(&self, info: *mut AnyObject) -> bool {
            drop_session_operation(&self.ivars().events, info) != DRAG_OPERATION_NONE
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, info: *mut AnyObject) -> bool {
            let events = self.ivars().events.clone();
            deliver_view_drop(self, &events, info)
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            // Input acceptance also enrolls the canvas in the window's
            // key-view loop; a purely graphical canvas stays out of it.
            self.ivars().accepts_input.get()
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            // SAFETY: NSView's own implementation completes the responder
            // transition before the component observes it.
            let accepted: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if accepted {
                self.ivars().events.emit_focus(true);
            }
            accepted
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            // SAFETY: NSView's own implementation completes the responder
            // transition before the component observes it.
            let resigned: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            if resigned {
                self.abandon_composition();
                self.ivars().events.emit_focus(false);
            }
            resigned
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &AnyObject) {
            if !self.ivars().accepts_input.get() {
                // SAFETY: Forwarding preserves NSResponder's default routing
                // for canvases that do not host text input.
                unsafe {
                    let _: () = msg_send![super(self), keyDown: event];
                }
                return;
            }
            self.interpret_key_event(event);
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, _selector: objc2::runtime::Sel) {
            // The input context routes editing commands (arrows, Return,
            // deletes) here when no composition consumes them; the raw
            // KeyEvent emitted by keyDown: already reports the key, and
            // NSResponder's default implementation would beep.
        }
    }

    // The selector implementations below satisfy every required
    // NSTextInputClient method; adopting the protocol is what makes
    // `NSView.inputContext` non-nil so the OS input method drives them.
    unsafe impl NSTextInputClient for CanvasView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text_in_replacement_range(&self, string: &AnyObject, _range: NSRange) {
            let text = plain_text_argument(string);
            let had_marked = !self.ivars().marked_text.borrow().is_empty();
            if had_marked {
                // Committing ends the composition; the preedit is implicitly
                // cleared on the application side.
                self.ivars().marked_text.borrow_mut().clear();
                if let Some(pending) = self.ivars().pending_key.borrow_mut().as_mut() {
                    pending.composition = true;
                }
                self.ivars().events.emit_ime(ImeEvent::Commit { text });
                return;
            }
            {
                let mut pending = self.ivars().pending_key.borrow_mut();
                if let Some(pending) = pending.as_mut() {
                    // Plain typing: the text rides on the raw KeyEvent the
                    // surrounding keyDown: emits.
                    match &mut pending.text {
                        Some(existing) => existing.push_str(&text),
                        slot @ None => *slot = Some(text),
                    }
                    return;
                }
            }
            // Insertion outside any key-down: the input method committed
            // text on its own (a candidate chosen with the mouse).
            self.ivars().events.emit_ime(ImeEvent::Commit { text });
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, selected: NSRange, _range: NSRange) {
            let text = plain_text_argument(string);
            if let Some(pending) = self.ivars().pending_key.borrow_mut().as_mut() {
                pending.composition = true;
            }
            if text.is_empty() {
                let was_marked = !self.ivars().marked_text.borrow().is_empty();
                self.ivars().marked_text.borrow_mut().clear();
                if was_marked {
                    self.ivars().events.emit_ime(ImeEvent::Cancel);
                }
                return;
            }
            let caret = utf16_range_to_preedit_caret(&text, selected);
            self.ivars().marked_text.borrow_mut().clone_from(&text);
            self.ivars().events.emit_ime(ImeEvent::Preedit { text, caret });
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            // AppKit defines unmarkText as accepting the current marked
            // text, so it commits rather than cancels.
            let text = std::mem::take(&mut *self.ivars().marked_text.borrow_mut());
            if let Some(pending) = self.ivars().pending_key.borrow_mut().as_mut() {
                pending.composition = true;
            }
            if !text.is_empty() {
                self.ivars().events.emit_ime(ImeEvent::Commit { text });
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            !self.ivars().marked_text.borrow().is_empty()
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let marked = self.ivars().marked_text.borrow();
            if marked.is_empty() {
                empty_text_range()
            } else {
                NSRange::new(0, marked.encode_utf16().count())
            }
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            // The canvas exposes no selection model; the insertion point is
            // wherever the application's caret rectangle says it is.
            empty_text_range()
        }

        #[unsafe(method(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring_for_proposed_range(
            &self,
            _range: NSRange,
            _actual: *mut NSRange,
        ) -> *mut AnyObject {
            // The application owns its text storage; the protocol allows nil.
            std::ptr::null_mut()
        }

        #[unsafe(method(validAttributesForMarkedText))]
        fn valid_attributes_for_marked_text(&self) -> *mut AnyObject {
            // SAFETY: The class factory returns a live autoreleased empty
            // array; the canvas renders the preedit itself, unstyled.
            unsafe { msg_send![objc2::class!(NSArray), array] }
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect_for_character_range(
            &self,
            _range: NSRange,
            _actual: *mut NSRange,
        ) -> Rect {
            // The candidate window anchors at the app-declared caret
            // rectangle, converted from element-local to screen coordinates.
            let caret = self
                .ivars()
                .ime_caret
                .get()
                .unwrap_or(CanvasRect::new(0.0, 0.0, 0.0, 0.0));
            // SAFETY: Geometry conversion happens on the main thread; the
            // window is checked for teardown.
            unsafe {
                let in_window: Rect = msg_send![
                    self,
                    convertRect: canvas_rect(caret),
                    toView: std::ptr::null::<AnyObject>()
                ];
                let window: *mut AnyObject = msg_send![self, window];
                if window.is_null() {
                    Rect::default()
                } else {
                    msg_send![window, convertRectToScreen: in_window]
                }
            }
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, _point: Point) -> usize {
            // The canvas exposes no character geometry to hit-test into.
            0
        }
    }
);

impl CanvasView {
    fn new(
        mtm: MainThreadMarker,
        size: CanvasSize,
        scene: DrawScene,
        accepts_input: bool,
        ime_caret: Option<CanvasRect>,
        events: EventBindings,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(CanvasViewIvars {
            size: Cell::new(size),
            scene: RefCell::new(scene),
            events,
            accepts_input: Cell::new(accepts_input),
            ime_caret: Cell::new(ime_caret),
            marked_text: RefCell::new(String::new()),
            pending_key: RefCell::new(None),
        });
        // SAFETY: initWithFrame: is NSView's designated initializer and the
        // ivars were initialized above on the main thread.
        let view: Retained<Self> = unsafe { msg_send![super(object), initWithFrame: Rect::default()] };
        // SAFETY: The tracking area is created owned, the view retains it in
        // addTrackingArea:, and the Id wrapper releases the creation retain.
        unsafe {
            let bounds: Rect = msg_send![&*view, bounds];
            let allocated: *mut AnyObject = msg_send![objc2::class!(NSTrackingArea), alloc];
            let area = Id::from_owned(msg_send![allocated,
                initWithRect: bounds,
                options: CANVAS_TRACKING_OPTIONS,
                owner: &*view,
                userInfo: std::ptr::null::<AnyObject>()
            ]);
            let _: () = msg_send![&*view, addTrackingArea: area.as_object()];
        }
        view
    }

    /// Replaces the retained scene and coalesces the change into one native
    /// redraw: AppKit collapses any number of setNeedsDisplay: marks issued
    /// before the next display pass into a single drawRect: invocation.
    fn apply_content(
        &self,
        size: CanvasSize,
        scene: &DrawScene,
        accepts_input: bool,
        ime_caret: Option<CanvasRect>,
    ) {
        let size_changed = self.ivars().size.get() != size;
        self.ivars().size.set(size);
        self.ivars().scene.borrow_mut().clone_from(scene);
        let input_changed = self.ivars().accepts_input.get() != accepts_input;
        self.ivars().accepts_input.set(accepts_input);
        let caret_changed = self.ivars().ime_caret.get() != ime_caret;
        self.ivars().ime_caret.set(ime_caret);
        if input_changed {
            self.apply_accessibility_role();
            if !accepts_input {
                self.surrender_first_responder();
            }
        }
        if caret_changed {
            // SAFETY: The input context re-queries firstRectForCharacterRange
            // so the OS candidate window follows the declared caret.
            unsafe {
                let context: *mut AnyObject = msg_send![self, inputContext];
                if !context.is_null() {
                    let _: () = msg_send![context, invalidateCharacterCoordinates];
                }
            }
        }
        // SAFETY: The receiver is a live NSView on the main thread.
        unsafe {
            if size_changed {
                let _: () = msg_send![self, invalidateIntrinsicContentSize];
            }
            let _: () = msg_send![self, setNeedsDisplay: true];
        }
    }

    /// Reflects input acceptance in the accessibility tree: an
    /// input-accepting canvas reads as a text area (the terminal contract),
    /// a purely graphical one as an image.
    fn apply_accessibility_role(&self) {
        // SAFETY: NSView exposes the NSAccessibility role setter and both
        // role constants are live static NSStrings.
        unsafe {
            let role = if self.ivars().accepts_input.get() {
                ACCESSIBILITY_TEXT_AREA_ROLE
            } else {
                ACCESSIBILITY_IMAGE_ROLE
            };
            let _: () = msg_send![self, setAccessibilityRole: role];
        }
    }

    /// Returns keyboard focus to the window when the canvas holds it while
    /// losing its input acceptance.
    fn surrender_first_responder(&self) {
        // SAFETY: The window and its first responder are read and mutated on
        // the main thread.
        unsafe {
            let window: *mut AnyObject = msg_send![self, window];
            if window.is_null() {
                return;
            }
            let responder: *mut AnyObject = msg_send![window, firstResponder];
            if std::ptr::eq(responder.cast_const(), (self as *const Self).cast()) {
                let _: bool =
                    msg_send![window, makeFirstResponder: std::ptr::null::<AnyObject>()];
            }
        }
    }

    /// Ends an active composition without committing, discarding the input
    /// method's session state alongside the client state.
    fn abandon_composition(&self) {
        let text = std::mem::take(&mut *self.ivars().marked_text.borrow_mut());
        if text.is_empty() {
            return;
        }
        // SAFETY: discardMarkedText resets the input method's session so
        // stale composition state cannot leak into the next focused view.
        unsafe {
            let context: *mut AnyObject = msg_send![self, inputContext];
            if !context.is_null() {
                let _: () = msg_send![context, discardMarkedText];
            }
        }
        self.ivars().events.emit_ime(ImeEvent::Cancel);
    }

    /// Interprets one key-down through the input context and emits exactly
    /// one raw [`KeyEvent`] unless a composition consumed the keystroke.
    ///
    /// Delivery decision, recorded for `reports/canvas-text-input`: a
    /// key-down that begins, updates, or commits a composition produces only
    /// [`ImeEvent`]s — otherwise the Return that commits a Japanese
    /// composition would also arrive as a raw Enter and a terminal would
    /// forward a spurious newline. Every other key-down produces one
    /// [`KeyEvent`] whose `text` is whatever the input context inserted for
    /// it, so dead-key results (Option-E then E yielding é) arrive already
    /// translated through the marked-text path.
    fn interpret_key_event(&self, event: &AnyObject) {
        let had_marked = !self.ivars().marked_text.borrow().is_empty();
        // SAFETY: The argument is a live NSEvent key-down on the main thread.
        let (characters, key_code, flags, repeat) = unsafe {
            let characters: *mut AnyObject = msg_send![event, charactersIgnoringModifiers];
            let key_code: u16 = msg_send![event, keyCode];
            let flags: usize = msg_send![event, modifierFlags];
            let repeat: bool = msg_send![event, isARepeat];
            (rust_string(characters), key_code, flags, repeat)
        };
        *self.ivars().pending_key.borrow_mut() = Some(PendingCanvasKey {
            key: key_identity_from_characters(&characters)
                .or_else(|| digit_identity_from_key_code(key_code)),
            modifiers: semantic_modifiers(flags),
            repeat,
            text: None,
            composition: false,
        });
        // SAFETY: The view's own NSTextInputContext interprets the event on
        // the main thread; it calls back into the NSTextInputClient methods
        // above, which record into the pending key.
        let context_missing = unsafe {
            let context: *mut AnyObject = msg_send![self, inputContext];
            if context.is_null() {
                true
            } else {
                let _handled: bool = msg_send![context, handleEvent: event];
                false
            }
        };
        let Some(mut pending) = self.ivars().pending_key.borrow_mut().take() else {
            return;
        };
        if context_missing {
            // Without an input context there is no translation service; the
            // event's own characters are the produced text.
            pending.text = printable_event_text(event);
        }
        let composing =
            had_marked || pending.composition || !self.ivars().marked_text.borrow().is_empty();
        if composing {
            // The composition consumed this keystroke; its IME events
            // already carried it to the component.
            return;
        }
        self.ivars().events.emit_key(KeyEvent {
            key: pending.key,
            modifiers: pending.modifiers,
            text: pending.text,
            repeat: pending.repeat,
        });
    }

    fn backing_scale(&self) -> f64 {
        // SAFETY: window is a nullable NSWindow queried on the main thread.
        let window: *mut AnyObject = unsafe { msg_send![self, window] };
        if window.is_null() {
            return 1.0;
        }
        // SAFETY: The receiver is the live NSWindow returned above.
        let scale: f64 = unsafe { msg_send![window, backingScaleFactor] };
        if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        }
    }

    fn replay_scene(&self) {
        let scale = self.backing_scale();
        let scene = self.ivars().scene.borrow();
        let mut clip_depth = 0_usize;
        for command in scene.commands() {
            replay_command(command, scale, &mut clip_depth);
        }
        // Validation guarantees balance; restore defensively so a future
        // regression cannot corrupt the shared graphics-context stack.
        for _ in 0..clip_depth {
            // SAFETY: Balances a saveGraphicsState issued by replay_command.
            unsafe {
                let _: () = msg_send![objc2::class!(NSGraphicsContext), restoreGraphicsState];
            }
        }
    }

    fn emit_pointer(&self, event: &AnyObject, phase: PointerPhase, button: PointerButton) {
        // SAFETY: The argument is a live NSEvent delivered by AppKit and the
        // receiver converts window coordinates on the main thread; the view
        // is flipped, so the local point is already top-left based.
        let (position, flags, scroll_delta) = unsafe {
            let location: Point = msg_send![event, locationInWindow];
            let local: Point = msg_send![
                self,
                convertPoint: location,
                fromView: std::ptr::null::<AnyObject>()
            ];
            let flags: usize = msg_send![event, modifierFlags];
            let scroll_delta = if phase == PointerPhase::Scroll {
                let dx: f64 = msg_send![event, scrollingDeltaX];
                let dy: f64 = msg_send![event, scrollingDeltaY];
                CanvasVector::new(dx, dy)
            } else {
                CanvasVector::default()
            };
            (CanvasPoint::new(local.x, local.y), flags, scroll_delta)
        };
        self.ivars().events.emit_pointer(PointerEvent {
            phase,
            position,
            button,
            modifiers: PointerModifiers {
                shift: flags & MODIFIER_FLAG_SHIFT != 0,
                control: flags & MODIFIER_FLAG_CONTROL != 0,
                option: flags & MODIFIER_FLAG_OPTION != 0,
                command: flags & MODIFIER_FLAG_COMMAND != 0,
            },
            scroll_delta,
        });
    }
}

/// The empty text range NSTextInputClient reports outside any marked text.
const fn empty_text_range() -> NSRange {
    NSRange {
        location: NSNotFound as usize,
        length: 0,
    }
}

/// Extracts the plain text of an NSTextInputClient string argument, which is
/// either an NSString or an NSAttributedString.
fn plain_text_argument(string: &AnyObject) -> String {
    // SAFETY: The argument is a live NSString or NSAttributedString supplied
    // by the input context on the main thread.
    unsafe {
        let is_attributed: bool =
            msg_send![string, isKindOfClass: objc2::class!(NSAttributedString)];
        let plain: *mut AnyObject = if is_attributed {
            msg_send![string, string]
        } else {
            (string as *const AnyObject).cast_mut()
        };
        rust_string(plain)
    }
}

/// Converts one UTF-16 offset into the index of the scalar containing it,
/// rounding an offset inside a surrogate pair up to the next scalar.
fn utf16_offset_to_char_index(text: &str, target: usize) -> Option<usize> {
    let mut utf16 = 0_usize;
    for (index, character) in text.chars().enumerate() {
        if utf16 >= target {
            return Some(index);
        }
        utf16 += character.len_utf16();
    }
    (utf16 >= target).then(|| text.chars().count())
}

/// Converts the input method's UTF-16 selection within the marked text into
/// the platform-neutral scalar-offset caret span.
fn utf16_range_to_preedit_caret(text: &str, selected: NSRange) -> Option<PreeditCaret> {
    if selected.location == NSNotFound as usize {
        return None;
    }
    let start = utf16_offset_to_char_index(text, selected.location)?;
    let end = utf16_offset_to_char_index(text, selected.location.checked_add(selected.length)?)?;
    Some(PreeditCaret::new(start, end))
}

/// Returns the text a key event itself produced, filtering control and
/// AppKit function-key code points that are not text.
fn printable_event_text(event: &AnyObject) -> Option<String> {
    // SAFETY: characters is a live NSString property of the key event read
    // on the main thread.
    let text = unsafe {
        let characters: *mut AnyObject = msg_send![event, characters];
        rust_string(characters)
    };
    let is_text = !text.is_empty()
        && !text
            .chars()
            .any(|character| character.is_control() || ('\u{f700}'..='\u{f8ff}').contains(&character));
    is_text.then_some(text)
}

fn create_canvas(
    mtm: MainThreadMarker,
    size: CanvasSize,
    scene: &DrawScene,
    accepts_input: bool,
    ime_caret: Option<CanvasRect>,
    accessibility_label: &str,
    events: EventBindings,
) -> AppKitHandle {
    let canvas = CanvasView::new(mtm, size, scene.clone(), accepts_input, ime_caret, events);
    // SAFETY: Retained keeps the object alive across this borrow.
    let view = unsafe {
        Id::from_borrowed(Retained::as_ptr(&canvas) as *mut AnyObject)
    };
    // SAFETY: NSView exposes the NSAccessibility configuration setters.
    unsafe {
        let _: () = msg_send![view.as_object(), setAccessibilityElement: true];
    }
    canvas.apply_accessibility_role();
    set_string(
        view.as_object(),
        SET_ACCESSIBILITY_LABEL,
        accessibility_label,
    );
    // The canvas retains its declared content size; parents may still
    // stretch it with required constraints.
    configure_growth(view.as_object(), false, false);
    let handle = AppKitHandle::new(
        view,
        HostKind::Element(ElementKind::Canvas),
        None,
        Vec::new(),
    );
    *handle.0.canvas_view.borrow_mut() = Some(canvas);
    handle
}

fn replay_command(command: &DrawCommand, scale: f64, clip_depth: &mut usize) {
    match command {
        DrawCommand::FillRect { rect, color } => {
            set_fill_color(*color);
            // SAFETY: NSBezierPath class factories return live autoreleased
            // paths; drawing happens inside drawRect: on the main thread.
            unsafe {
                let path: *mut AnyObject = msg_send![
                    objc2::class!(NSBezierPath),
                    bezierPathWithRect: canvas_rect(*rect)
                ];
                let _: () = msg_send![path, fill];
            }
        }
        DrawCommand::StrokeRect { rect, width, color } => {
            set_stroke_color(*color);
            let rect = match width {
                LineWidth::Hairline => snap_rect_to_device_pixels(*rect, scale),
                LineWidth::Points(_) => *rect,
            };
            // SAFETY: See FillRect.
            unsafe {
                let path: *mut AnyObject = msg_send![
                    objc2::class!(NSBezierPath),
                    bezierPathWithRect: canvas_rect(rect)
                ];
                let _: () = msg_send![path, setLineWidth: stroke_width(*width, scale)];
                let _: () = msg_send![path, stroke];
            }
        }
        DrawCommand::Line {
            from,
            to,
            width,
            color,
        } => {
            set_stroke_color(*color);
            let (from, to) = match width {
                LineWidth::Hairline => (
                    snap_point_to_device_pixels(*from, scale),
                    snap_point_to_device_pixels(*to, scale),
                ),
                LineWidth::Points(_) => (*from, *to),
            };
            // SAFETY: See FillRect.
            unsafe {
                let path: *mut AnyObject = msg_send![objc2::class!(NSBezierPath), bezierPath];
                let _: () = msg_send![path, moveToPoint: canvas_point(from)];
                let _: () = msg_send![path, lineToPoint: canvas_point(to)];
                let _: () = msg_send![path, setLineWidth: stroke_width(*width, scale)];
                let _: () = msg_send![path, stroke];
            }
        }
        DrawCommand::FillCircle {
            center,
            radius,
            color,
        } => {
            set_fill_color(*color);
            // SAFETY: See FillRect.
            unsafe {
                let path: *mut AnyObject = msg_send![
                    objc2::class!(NSBezierPath),
                    bezierPathWithOvalInRect: circle_bounds(*center, *radius)
                ];
                let _: () = msg_send![path, fill];
            }
        }
        DrawCommand::StrokeCircle {
            center,
            radius,
            width,
            color,
        } => {
            set_stroke_color(*color);
            // SAFETY: See FillRect.
            unsafe {
                let path: *mut AnyObject = msg_send![
                    objc2::class!(NSBezierPath),
                    bezierPathWithOvalInRect: circle_bounds(*center, *radius)
                ];
                let _: () = msg_send![path, setLineWidth: stroke_width(*width, scale)];
                let _: () = msg_send![path, stroke];
            }
        }
        DrawCommand::StrokeArc {
            center,
            radius,
            start_angle,
            end_angle,
            width,
            color,
        } => {
            set_stroke_color(*color);
            // The common contract measures angles toward positive y. In this
            // flipped view increasing angles already sweep toward positive y,
            // so clockwise:NO follows the declared direction.
            // SAFETY: See FillRect.
            unsafe {
                let path: *mut AnyObject = msg_send![objc2::class!(NSBezierPath), bezierPath];
                let _: () = msg_send![path,
                    appendBezierPathWithArcWithCenter: canvas_point(*center),
                    radius: *radius,
                    startAngle: start_angle.to_degrees(),
                    endAngle: end_angle.to_degrees(),
                    clockwise: false
                ];
                let _: () = msg_send![path, setLineWidth: stroke_width(*width, scale)];
                let _: () = msg_send![path, stroke];
            }
        }
        DrawCommand::GlyphRun {
            origin,
            text,
            font_size,
            color,
        } => draw_glyph_run(*origin, text, *font_size, *color),
        DrawCommand::PushClip { rect } => {
            *clip_depth += 1;
            // SAFETY: The graphics-context stack is balanced by PopClip or by
            // the defensive restore in replay_scene.
            unsafe {
                let _: () = msg_send![objc2::class!(NSGraphicsContext), saveGraphicsState];
                let path: *mut AnyObject = msg_send![
                    objc2::class!(NSBezierPath),
                    bezierPathWithRect: canvas_rect(*rect)
                ];
                let _: () = msg_send![path, addClip];
            }
        }
        DrawCommand::PopClip => {
            if let Some(depth) = clip_depth.checked_sub(1) {
                *clip_depth = depth;
                // SAFETY: Balances the saveGraphicsState of the matching push.
                unsafe {
                    let _: () = msg_send![objc2::class!(NSGraphicsContext), restoreGraphicsState];
                }
            }
        }
    }
}

fn draw_glyph_run(origin: CanvasPoint, text: &str, font_size: f64, color: CanvasColor) {
    let string = ns_string(text);
    // SAFETY: The attribute keys are live static NSString constants, the
    // dictionary retains its values, and NSString drawing inside drawRect:
    // honors the flipped context, placing the origin at the line-box top-left.
    unsafe {
        let font: *mut AnyObject = msg_send![
            objc2::class!(NSFont),
            monospacedSystemFontOfSize: font_size,
            weight: 0.0_f64
        ];
        let color: *mut AnyObject = msg_send![
            objc2::class!(NSColor),
            colorWithSRGBRed: color.red,
            green: color.green,
            blue: color.blue,
            alpha: color.alpha
        ];
        let attributes: *mut AnyObject = msg_send![objc2::class!(NSMutableDictionary), dictionary];
        let _: () = msg_send![attributes, setObject: font, forKey: FONT_ATTRIBUTE_NAME];
        let _: () = msg_send![attributes, setObject: color, forKey: FOREGROUND_COLOR_ATTRIBUTE_NAME];
        let _: () = msg_send![
            string.as_object(),
            drawAtPoint: canvas_point(origin),
            withAttributes: attributes
        ];
    }
}

fn set_fill_color(color: CanvasColor) {
    // SAFETY: NSColor factories return live autoreleased colors and setFill
    // configures the current drawRect: graphics context.
    unsafe {
        let color: *mut AnyObject = msg_send![
            objc2::class!(NSColor),
            colorWithSRGBRed: color.red,
            green: color.green,
            blue: color.blue,
            alpha: color.alpha
        ];
        let _: () = msg_send![color, setFill];
    }
}

fn set_stroke_color(color: CanvasColor) {
    // SAFETY: See set_fill_color; setStroke configures the stroke color.
    unsafe {
        let color: *mut AnyObject = msg_send![
            objc2::class!(NSColor),
            colorWithSRGBRed: color.red,
            green: color.green,
            blue: color.blue,
            alpha: color.alpha
        ];
        let _: () = msg_send![color, setStroke];
    }
}

const fn canvas_point(point: CanvasPoint) -> Point {
    Point {
        x: point.x,
        y: point.y,
    }
}

const fn canvas_rect(rect: CanvasRect) -> Rect {
    Rect {
        origin: Point {
            x: rect.origin.x,
            y: rect.origin.y,
        },
        size: Size {
            width: rect.size.width,
            height: rect.size.height,
        },
    }
}

fn circle_bounds(center: CanvasPoint, radius: f64) -> Rect {
    Rect {
        origin: Point {
            x: center.x - radius,
            y: center.y - radius,
        },
        size: Size {
            width: radius * 2.0,
            height: radius * 2.0,
        },
    }
}

/// Resolves a semantic stroke width into logical points for this backing.
fn stroke_width(width: LineWidth, scale: f64) -> f64 {
    match width {
        LineWidth::Hairline => 1.0 / scale,
        LineWidth::Points(value) => value,
    }
}

/// Snaps a logical coordinate onto the nearest device-pixel center so a
/// hairline stroke covers exactly one device pixel instead of smearing
/// across two half-covered neighbors.
fn snap_to_device_pixel_center(value: f64, scale: f64) -> f64 {
    ((value * scale - 0.5).round() + 0.5) / scale
}

fn snap_point_to_device_pixels(point: CanvasPoint, scale: f64) -> CanvasPoint {
    CanvasPoint::new(
        snap_to_device_pixel_center(point.x, scale),
        snap_to_device_pixel_center(point.y, scale),
    )
}

fn snap_rect_to_device_pixels(rect: CanvasRect, scale: f64) -> CanvasRect {
    let left = snap_to_device_pixel_center(rect.origin.x, scale);
    let top = snap_to_device_pixel_center(rect.origin.y, scale);
    let right = snap_to_device_pixel_center(rect.origin.x + rect.size.width, scale);
    let bottom = snap_to_device_pixel_center(rect.origin.y + rect.size.height, scale);
    CanvasRect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
}

/// Measures the native monospace system font for canvas glyph layout.
fn measure_monospace_metrics(font_size: f64) -> Option<MonospaceMetrics> {
    if !font_size.is_finite() || font_size <= 0.0 {
        return None;
    }
    // SAFETY: NSFont metrics are queried on the main thread where the
    // backend lives; the font object is a live autoreleased NSFont.
    let (row_height, glyph_width) = unsafe {
        let font: *mut AnyObject = msg_send![
            objc2::class!(NSFont),
            monospacedSystemFontOfSize: font_size,
            weight: 0.0_f64
        ];
        if font.is_null() {
            return None;
        }
        let ascender: f64 = msg_send![font, ascender];
        let descender: f64 = msg_send![font, descender];
        let leading: f64 = msg_send![font, leading];
        let advancement: Size = msg_send![font, maximumAdvancement];
        (ascender - descender + leading, advancement.width)
    };
    if !(row_height.is_finite() && row_height > 0.0 && glyph_width.is_finite() && glyph_width > 0.0)
    {
        return None;
    }
    Some(MonospaceMetrics {
        row_height,
        glyph_width,
    })
}
