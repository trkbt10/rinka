// NSTextView-backed multi-line text area.
//
// The native view owns the live buffer; reconciliation follows the
// controlled-text protocol of `rinka_core::TextContent`. This file keeps a
// Rust mirror of the buffer so UTF-16 native ranges convert to the core's
// character index space without bridging the whole document per keystroke,
// captures every buffer mutation at the NSTextStorage delegate seam, and
// applies highlight spans as NSLayoutManager temporary attributes so syntax
// color never touches the text storage, the undo stack, or IME marked text.

/// `NSTextStorageEditActions` bit reporting edited characters.
const TEXT_STORAGE_EDITED_CHARACTERS: usize = 1 << 1;

/// Microseconds since the first probe stamp, for latency diagnostics.
fn probe_stamp() -> u128 {
    use std::sync::OnceLock;
    static EPOCH: OnceLock<std::time::Instant> = OnceLock::new();
    EPOCH.get_or_init(std::time::Instant::now).elapsed().as_micros()
}

/// A UTF-16 native range located inside the UTF-8 mirror.
struct LocatedRange {
    char_range: TextRange,
    byte_range: std::ops::Range<usize>,
}

/// Maps a UTF-16 code-unit range onto character and byte ranges of `text`.
///
/// Returns [`None`] when an endpoint is past the end or splits a surrogate
/// pair, which signals mirror drift and triggers the full resynchronization
/// path.
fn locate_utf16_range(text: &str, start_utf16: usize, end_utf16: usize) -> Option<LocatedRange> {
    if end_utf16 < start_utf16 {
        return None;
    }
    let mut utf16_index = 0_usize;
    let mut char_count = 0_usize;
    let mut start: Option<(usize, usize)> = None;
    let mut end: Option<(usize, usize)> = None;
    for (char_index, (byte_index, character)) in text.char_indices().enumerate() {
        if start.is_none() && utf16_index == start_utf16 {
            start = Some((char_index, byte_index));
        }
        if utf16_index == end_utf16 {
            end = Some((char_index, byte_index));
            break;
        }
        utf16_index += character.len_utf16();
        char_count = char_index + 1;
    }
    if start.is_none() && utf16_index == start_utf16 {
        start = Some((char_count, text.len()));
    }
    if end.is_none() && utf16_index == end_utf16 {
        end = Some((char_count, text.len()));
    }
    let (start_char, start_byte) = start?;
    let (end_char, end_byte) = end?;
    Some(LocatedRange {
        char_range: TextRange::new(start_char, end_char),
        byte_range: start_byte..end_byte,
    })
}

/// Maps a character range onto the UTF-16 code-unit range of `text`.
fn locate_char_range(text: &str, range: TextRange) -> Option<NSRange> {
    if range.end < range.start {
        return None;
    }
    let mut utf16_index = 0_usize;
    let mut char_count = 0_usize;
    let mut start: Option<usize> = None;
    let mut end: Option<usize> = None;
    for (char_index, character) in text.chars().enumerate() {
        if start.is_none() && char_index == range.start {
            start = Some(utf16_index);
        }
        if char_index == range.end {
            end = Some(utf16_index);
            break;
        }
        utf16_index += character.len_utf16();
        char_count = char_index + 1;
    }
    if start.is_none() && char_count == range.start {
        start = Some(utf16_index);
    }
    if end.is_none() && char_count == range.end {
        end = Some(utf16_index);
    }
    let start = start?;
    let end = end?;
    Some(NSRange {
        location: start,
        length: end - start,
    })
}

/// Converts ordered highlight spans into UTF-16 ranges in one buffer pass,
/// clamping every span to the current buffer so stale spans stay safe.
fn spans_to_utf16_ranges(
    text: &str,
    spans: &[HighlightSpan],
) -> Vec<(NSRange, HighlightRole)> {
    let mut ranges = Vec::with_capacity(spans.len());
    let mut span_index = 0_usize;
    let mut utf16_index = 0_usize;
    let mut open: Option<(usize, HighlightRole, usize)> = None;
    for (char_index, character) in text.chars().enumerate() {
        if let Some((start_utf16, role, end_char)) = open
            && char_index >= end_char
        {
            ranges.push((
                NSRange {
                    location: start_utf16,
                    length: utf16_index - start_utf16,
                },
                role,
            ));
            open = None;
        }
        if open.is_none()
            && let Some(span) = spans.get(span_index)
            && span.range.start <= char_index
        {
            open = Some((utf16_index, span.role, span.range.end.max(char_index)));
            span_index += 1;
        }
        utf16_index += character.len_utf16();
    }
    if let Some((start_utf16, role, _)) = open
        && utf16_index > start_utf16
    {
        ranges.push((
            NSRange {
                location: start_utf16,
                length: utf16_index - start_utf16,
            },
            role,
        ));
    }
    ranges
}

/// Converts the ordered document spans intersecting one visible window into
/// UTF-16 ranges, clamped to the window, costing one pass over the window's
/// text plus a binary search over the span set.
fn spans_to_utf16_ranges_in_window(
    window_text: &str,
    window_chars: TextRange,
    window_utf16_start: usize,
    spans: &[HighlightSpan],
) -> Vec<(NSRange, HighlightRole)> {
    let first = spans.partition_point(|span| span.range.end <= window_chars.start);
    let rebased: Vec<HighlightSpan> = spans[first..]
        .iter()
        .take_while(|span| span.range.start < window_chars.end)
        .map(|span| {
            HighlightSpan::new(
                TextRange::new(
                    span.range.start.max(window_chars.start) - window_chars.start,
                    span.range.end.min(window_chars.end) - window_chars.start,
                ),
                span.role,
            )
        })
        .collect();
    let mut converted = spans_to_utf16_ranges(window_text, &rebased);
    for (range, _) in &mut converted {
        range.location += window_utf16_start;
    }
    converted
}

/// Returns the native palette color realizing one semantic highlight role.
///
/// System colors follow the effective light or dark appearance; the mapping
/// is this adapter's, never the core's.
fn highlight_color(role: HighlightRole) -> Id {
    // SAFETY: Every arm names a public NSColor class-property color.
    unsafe {
        let color: *mut AnyObject = match role {
            HighlightRole::Keyword => msg_send![objc2::class!(NSColor), systemPurpleColor],
            HighlightRole::String => msg_send![objc2::class!(NSColor), systemRedColor],
            HighlightRole::Number => msg_send![objc2::class!(NSColor), systemBlueColor],
            HighlightRole::Comment => msg_send![objc2::class!(NSColor), systemGrayColor],
            HighlightRole::Type => msg_send![objc2::class!(NSColor), systemTealColor],
            HighlightRole::Function => msg_send![objc2::class!(NSColor), systemIndigoColor],
            HighlightRole::Variable => msg_send![objc2::class!(NSColor), systemCyanColor],
            HighlightRole::Constant => msg_send![objc2::class!(NSColor), systemOrangeColor],
            HighlightRole::Operator => msg_send![objc2::class!(NSColor), systemBrownColor],
            HighlightRole::Punctuation => msg_send![objc2::class!(NSColor), secondaryLabelColor],
            HighlightRole::Attribute => msg_send![objc2::class!(NSColor), systemMintColor],
            HighlightRole::Preprocessor => msg_send![objc2::class!(NSColor), systemPinkColor],
        };
        Id::from_borrowed(color)
    }
}

/// Declarative state deferred while an IME composition owns the buffer.
struct DeferredTextArea {
    content: TextContent,
    spans: HighlightSpans,
    selection: Option<TextSelection>,
}

struct TextAreaDelegateIvars {
    events: EventBindings,
    /// Rust mirror of the native buffer, kept current at the text-storage
    /// delegate seam so index conversions never bridge the whole document.
    mirror: RefCell<String>,
    /// The adapter-side document revision of the native buffer.
    revision: Cell<TextRevision>,
    /// Native edits recorded but not yet reported to the application.
    pending_edits: RefCell<Vec<TextEdit>>,
    emission_scheduled: Cell<bool>,
    selection_pending: Cell<bool>,
    /// Suppresses recording and event emission while the adapter itself
    /// mutates the view during reconciliation.
    applying: Cell<bool>,
    text_view: RefCell<Option<Id>>,
    deferred: RefCell<Option<DeferredTextArea>>,
    /// Last applied highlight-span revision; [`None`] forces re-application.
    spans_revision: Cell<Option<u64>>,
    /// The declared span set, applied lazily over the visible range so a
    /// document-wide highlight never costs more than the viewport.
    declared_spans: RefCell<Vec<HighlightSpan>>,
    role: Cell<TextRole>,
    /// Diagnostic-only: the instant the text-area probe typed, so the edit
    /// round trip is measured on the causal path instead of by polling.
    probe_typed_at: Cell<Option<std::time::Instant>>,
}

impl fmt::Debug for TextAreaDelegateIvars {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextAreaDelegateIvars")
            .field("revision", &self.revision.get())
            .field("pending_edit_count", &self.pending_edits.borrow().len())
            .field("applying", &self.applying.get())
            .finish_non_exhaustive()
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = TextAreaDelegateIvars]
    struct TextAreaDelegate;

    // SAFETY: NSObjectProtocol adds no invariants beyond the NSObject superclass.
    unsafe impl NSObjectProtocol for TextAreaDelegate {}

    impl TextAreaDelegate {
        /// NSTextStorageDelegate seam: every buffer mutation — typing, IME
        /// marked text, undo, paste, drops — reports here exactly once.
        #[unsafe(method(textStorage:didProcessEditing:range:changeInLength:))]
        fn did_process_editing(
            &self,
            storage: &AnyObject,
            edited_mask: usize,
            edited_range: NSRange,
            change_in_length: isize,
        ) {
            if self.ivars().applying.get() {
                return;
            }
            if edited_mask & TEXT_STORAGE_EDITED_CHARACTERS == 0 {
                return;
            }
            self.record_native_edit(storage, edited_range, change_in_length);
            self.schedule_drain();
        }

        /// Scroll and resize reveal buffer regions whose highlight has not
        /// been materialized yet; re-apply the declared spans there.
        #[unsafe(method(viewBoundsDidChange:))]
        fn view_bounds_did_change(&self, _notification: &AnyObject) {
            self.apply_visible_spans();
        }

        #[unsafe(method(textViewDidChangeSelection:))]
        fn text_view_did_change_selection(&self, _notification: &AnyObject) {
            if self.ivars().applying.get() {
                return;
            }
            if self.ivars().emission_scheduled.get() {
                // A text edit is pending; report the selection only after the
                // application has received the document delta, so a selection
                // event never references characters it does not know yet.
                self.ivars().selection_pending.set(true);
                return;
            }
            self.emit_native_selection();
        }

        /// Fires when a user edit completes, outside NSTextStorage's edit
        /// transaction: the recorded delta is reported immediately, without
        /// waiting for the scheduled run-loop turn.
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, _notification: &AnyObject) {
            self.drain_now();
        }

        /// Backstop for buffer mutations that post no textDidChange (for
        /// example another component mutating the storage directly): the
        /// delayed selector drains whatever the change seam recorded. Event
        /// emission still never runs inside NSTextStorage's edit transaction.
        #[unsafe(method(drainPendingTextEdits:))]
        fn drain_pending_text_edits(&self, _sender: *mut AnyObject) {
            self.drain_now();
        }
    }
);

impl TextAreaDelegate {
    fn new(
        mtm: MainThreadMarker,
        events: EventBindings,
        text: &str,
        revision: TextRevision,
        role: TextRole,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(TextAreaDelegateIvars {
            events,
            mirror: RefCell::new(text.to_owned()),
            revision: Cell::new(revision),
            pending_edits: RefCell::new(Vec::new()),
            emission_scheduled: Cell::new(false),
            selection_pending: Cell::new(false),
            applying: Cell::new(false),
            text_view: RefCell::new(None),
            deferred: RefCell::new(None),
            spans_revision: Cell::new(None),
            declared_spans: RefCell::new(Vec::new()),
            role: Cell::new(role),
            probe_typed_at: Cell::new(None),
        });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }

    fn text_view(&self) -> Option<Id> {
        self.ivars().text_view.borrow().clone()
    }

    /// Reports recorded edits, then the pending selection, then any state
    /// deferred behind a finished IME composition. Idempotent: an empty
    /// pending set makes the backstop drain a no-op.
    fn drain_now(&self) {
        if std::env::var_os("RINKA_APPKIT_TEXTAREA_PROBE").is_some() {
            eprintln!("Rinka textarea drain fired stamp={}", probe_stamp());
        }
        if std::env::var_os("RINKA_APPKIT_TEXTAREA_SPAN_DUMP").is_some()
            && let Some(view) = self.text_view()
        {
            // Diagnostic-only consistency check between the mirror and the
            // native backing string.
            let native = unsafe {
                let string: *mut AnyObject = msg_send![view.as_object(), string];
                rust_string(string)
            };
            let mirror = self.ivars().mirror.borrow();
            if *mirror != native {
                let divergence = mirror
                    .char_indices()
                    .zip(native.char_indices())
                    .find(|((_, ours), (_, theirs))| ours != theirs);
                eprintln!(
                    "Rinka textarea MIRROR DIVERGED mirror_len={} native_len={} first_difference={divergence:?}",
                    mirror.len(),
                    native.len()
                );
            }
        }
        self.ivars().emission_scheduled.set(false);
        let edits = std::mem::take(&mut *self.ivars().pending_edits.borrow_mut());
        let composing = self.has_marked_text();
        if !edits.is_empty() {
            let base_revision = self.ivars().revision.get();
            let revision = base_revision.next_edit();
            self.ivars().revision.set(revision);
            let probing = std::env::var_os("RINKA_APPKIT_TEXTAREA_PROBE").is_some();
            let started = std::time::Instant::now();
            self.ivars().events.emit_text_change(TextChange {
                base_revision,
                revision,
                edits,
                composing,
            });
            if probing {
                // Covers the application's update, its re-render, and the
                // adapter's echo reconciliation, synchronously.
                eprintln!(
                    "Rinka textarea change emit micros={}",
                    started.elapsed().as_micros()
                );
            }
            if let Some(typed_at) = self.ivars().probe_typed_at.take() {
                // The causal keystroke round trip: native edit recorded,
                // emission drained, application updated and re-rendered,
                // echo reconciled — complete at this point.
                eprintln!(
                    "Rinka textarea probe single-edit round-trip micros={}",
                    typed_at.elapsed().as_micros()
                );
            }
        }
        if self.ivars().selection_pending.replace(false) {
            self.emit_native_selection();
        }
        if !composing {
            self.apply_deferred();
        }
    }

    fn has_marked_text(&self) -> bool {
        let Some(view) = self.text_view() else {
            return false;
        };
        // SAFETY: hasMarkedText is an NSTextInputClient query on a live view.
        unsafe { msg_send![view.as_object(), hasMarkedText] }
    }

    /// Records one native buffer mutation into the pending delta, keeping the
    /// mirror synchronized. A range the mirror cannot locate means the mirror
    /// drifted; the fallback re-reads the native string and records one
    /// whole-buffer replacement so the reported deltas stay correct.
    fn record_native_edit(
        &self,
        storage: &AnyObject,
        edited_range: NSRange,
        change_in_length: isize,
    ) {
        let replacement = if edited_range.length == 0 {
            String::new()
        } else {
            // SAFETY: editedRange addresses the storage's current string.
            unsafe {
                let string: *mut AnyObject = msg_send![storage, string];
                let sub: *mut AnyObject = msg_send![string, substringWithRange: edited_range];
                rust_string(sub)
            }
        };
        let mut mirror = self.ivars().mirror.borrow_mut();
        if std::env::var_os("RINKA_APPKIT_TEXTAREA_SPAN_DUMP").is_some() {
            eprintln!(
                "Rinka textarea storage edit location={} length={} delta={change_in_length} replacement={replacement:?}",
                edited_range.location, edited_range.length
            );
        }
        let located = isize::try_from(edited_range.length)
            .ok()
            .map(|new_length| new_length - change_in_length)
            .and_then(|old_length| usize::try_from(old_length).ok())
            .and_then(|old_length| {
                locate_utf16_range(
                    &mirror,
                    edited_range.location,
                    edited_range.location + old_length,
                )
            });
        match located {
            Some(edit) => {
                mirror.replace_range(edit.byte_range, &replacement);
                self.ivars()
                    .pending_edits
                    .borrow_mut()
                    .push(TextEdit::new(edit.char_range, replacement));
            }
            None => {
                let stale_chars = mirror.chars().count();
                // SAFETY: string returns the storage's live backing string.
                let full = unsafe {
                    let string: *mut AnyObject = msg_send![storage, string];
                    rust_string(string)
                };
                mirror.clone_from(&full);
                self.ivars()
                    .pending_edits
                    .borrow_mut()
                    .push(TextEdit::new(TextRange::new(0, stale_chars), full));
            }
        }
    }

    fn schedule_drain(&self) {
        if self.ivars().emission_scheduled.replace(true) {
            return;
        }
        if std::env::var_os("RINKA_APPKIT_TEXTAREA_PROBE").is_some() {
            eprintln!("Rinka textarea drain scheduled stamp={}", probe_stamp());
        }
        // SAFETY: The delayed selector runs on the main run loop with self
        // retained by the perform request.
        unsafe {
            let _: () = msg_send![self,
                performSelector: sel!(drainPendingTextEdits:),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.0_f64
            ];
        }
    }

    fn emit_native_selection(&self) {
        let Some(view) = self.text_view() else {
            return;
        };
        // SAFETY: selectedRange is a main-thread query on a live NSTextView.
        let range: NSRange = unsafe { msg_send![view.as_object(), selectedRange] };
        let selection = {
            let mirror = self.ivars().mirror.borrow();
            locate_utf16_range(&mirror, range.location, range.location + range.length)
                .map(|located| {
                    TextSelection::new(located.char_range.start, located.char_range.end)
                })
        };
        // The borrow is released before emission: the handler re-renders and
        // reconciliation re-enters this delegate's state.
        if let Some(selection) = selection {
            self.ivars().events.emit_selection_change(selection);
        }
    }

    fn apply_deferred(&self) {
        let Some(deferred) = self.ivars().deferred.borrow_mut().take() else {
            return;
        };
        let Some(view) = self.text_view() else {
            return;
        };
        let result = apply_text_area_content(self, view.as_object(), &deferred.content);
        if let Err(error) = result {
            eprintln!("Rinka textarea deferred content application failed: {error}");
            return;
        }
        apply_text_area_spans(self, view.as_object(), &deferred.spans);
        apply_text_area_selection(self, view.as_object(), deferred.selection);
    }

    /// Materializes the declared spans as foreground-color temporary
    /// attributes over the currently visible character range.
    ///
    /// Application is viewport-scoped so a document-wide span set costs the
    /// viewport, not the document, per update; scrolling re-applies the
    /// newly revealed range through the bounds-change notification. While an
    /// IME composition is active nothing is touched: in TextKit 1 the input
    /// method renders its preedit underline through the same
    /// temporary-attribute channel.
    fn apply_visible_spans(&self) {
        if self.has_marked_text() {
            return;
        }
        let Some(view) = self.text_view() else {
            return;
        };
        let mirror = self.ivars().mirror.borrow();
        let spans = self.ivars().declared_spans.borrow();
        let probing = std::env::var_os("RINKA_APPKIT_TEXTAREA_PROBE").is_some();
        // SAFETY: Layout, container, and temporary-attribute calls address
        // the live text view's TextKit 1 objects on the main thread.
        unsafe {
            let layout: *mut AnyObject = msg_send![view.as_object(), layoutManager];
            let container: *mut AnyObject = msg_send![view.as_object(), textContainer];
            if layout.is_null() || container.is_null() {
                return;
            }
            let visible: Rect = msg_send![view.as_object(), visibleRect];
            // Right after a whole-document replacement no layout exists yet
            // and a bounding-rect glyph query would answer with the entire
            // document; laying out the viewport first keeps the answer — and
            // therefore the attribute application — viewport-sized.
            let ensure_started = std::time::Instant::now();
            let _: () = msg_send![layout,
                ensureLayoutForBoundingRect: visible,
                inTextContainer: container
            ];
            let ensure_micros = ensure_started.elapsed().as_micros();
            let query_started = std::time::Instant::now();
            let glyph_range: NSRange = msg_send![layout,
                glyphRangeForBoundingRect: visible,
                inTextContainer: container
            ];
            let window_utf16: NSRange = msg_send![layout,
                characterRangeForGlyphRange: glyph_range,
                actualGlyphRange: std::ptr::null_mut::<NSRange>()
            ];
            let query_micros = query_started.elapsed().as_micros();
            let Some(window) = locate_utf16_range(
                &mirror,
                window_utf16.location,
                window_utf16.location + window_utf16.length,
            ) else {
                return;
            };
            let paint_started = std::time::Instant::now();
            let attribute = Id::from_borrowed(FOREGROUND_COLOR_ATTRIBUTE_NAME);
            let _: () = msg_send![layout,
                removeTemporaryAttribute: attribute.as_object(),
                forCharacterRange: window_utf16
            ];
            let window_text = &mirror[window.byte_range.clone()];
            let converted = spans_to_utf16_ranges_in_window(
                window_text,
                window.char_range,
                window_utf16.location,
                &spans,
            );
            for (range, role) in &converted {
                let color = highlight_color(*role);
                let _: () = msg_send![layout,
                    addTemporaryAttribute: attribute.as_object(),
                    value: color.as_object(),
                    forCharacterRange: *range
                ];
            }
            if probing {
                eprintln!(
                    "Rinka textarea visible-window chars={} applied={} ensure_micros={ensure_micros} query_micros={query_micros} paint_micros={}",
                    window.char_range.len(),
                    converted.len(),
                    paint_started.elapsed().as_micros()
                );
                if std::env::var_os("RINKA_APPKIT_TEXTAREA_SPAN_DUMP").is_some() {
                    let native: *mut AnyObject = msg_send![view.as_object(), string];
                    for (range, role) in converted.iter().take(8) {
                        let sub: *mut AnyObject =
                            msg_send![native, substringWithRange: *range];
                        eprintln!(
                            "Rinka textarea span dump utf16=({},{}) role={role:?} native_text={:?}",
                            range.location,
                            range.length,
                            rust_string(sub)
                        );
                    }
                }
            }
        }
    }
}

/// Declarative text-area state routed to creation and reconciliation.
struct TextAreaConfig<'declaration> {
    content: &'declaration TextContent,
    spans: &'declaration HighlightSpans,
    selection: Option<TextSelection>,
    read_only: bool,
    role: TextRole,
    accessibility_label: &'declaration str,
}

/// Creates the NSScrollView plus NSTextView realization of a text area.
fn create_text_area(
    mtm: MainThreadMarker,
    config: TextAreaConfig<'_>,
    events: EventBindings,
) -> AppKitHandle {
    let TextAreaConfig {
        content,
        spans,
        selection,
        read_only,
        role,
        accessibility_label,
    } = config;
    let scroll = new_view(objc2::class!(NSScrollView));
    let text_view = new_view(objc2::class!(NSTextView));
    // SAFETY: The receivers are the live scroll and text views created above;
    // the configuration is the standard NSTextView-in-NSScrollView recipe.
    unsafe {
        let _: () = msg_send![scroll.as_object(), setHasVerticalScroller: true];
        let _: () = msg_send![scroll.as_object(), setHasHorizontalScroller: false];
        let _: () = msg_send![scroll.as_object(), setAutohidesScrollers: true];
        let _: () = msg_send![scroll.as_object(), setDrawsBackground: false];

        // Accessing layoutManager selects the TextKit 1 compatibility path:
        // highlight spans are NSLayoutManager temporary attributes, which
        // never dirty the storage, the undo stack, or IME marked text.
        let _: *mut AnyObject = msg_send![text_view.as_object(), layoutManager];

        let content_size: Size = msg_send![scroll.as_object(), contentSize];
        let frame = Rect {
            origin: Point::default(),
            size: content_size,
        };
        let _: () = msg_send![text_view.as_object(), setFrame: frame];
        let _: () = msg_send![text_view.as_object(), setVerticallyResizable: true];
        let _: () = msg_send![text_view.as_object(), setHorizontallyResizable: false];
        let _: () = msg_send![text_view.as_object(), setAutoresizingMask: 2_usize];
        let _: () = msg_send![text_view.as_object(), setMinSize: Size::default()];
        let _: () = msg_send![text_view.as_object(), setMaxSize: Size {
            width: 1.0e7,
            height: 1.0e7,
        }];
        let container: *mut AnyObject = msg_send![text_view.as_object(), textContainer];
        let _: () = msg_send![container, setWidthTracksTextView: true];
        let _: () = msg_send![container, setContainerSize: Size {
            width: content_size.width,
            height: 1.0e7,
        }];

        // Plain-text semantics: syntax color arrives as temporary attributes,
        // never as storage formatting the user could edit.
        let _: () = msg_send![text_view.as_object(), setRichText: false];
        let _: () = msg_send![text_view.as_object(), setImportsGraphics: false];
        let _: () = msg_send![text_view.as_object(), setUsesFontPanel: false];
        let _: () = msg_send![text_view.as_object(), setAllowsUndo: true];
        let _: () = msg_send![text_view.as_object(), setEditable: !read_only];
        let _: () = msg_send![text_view.as_object(), setSelectable: true];

        let value = ns_string(content.text());
        let _: () = msg_send![text_view.as_object(), setString: value.as_object()];
        apply_text_area_role(text_view.as_object(), role);

        let _: () = msg_send![scroll.as_object(), setDocumentView: text_view.as_object()];
    }
    configure_growth(scroll.as_object(), true, true);
    set_string(
        scroll.as_object(),
        SET_ACCESSIBILITY_LABEL,
        accessibility_label,
    );
    set_string(
        text_view.as_object(),
        SET_ACCESSIBILITY_LABEL,
        accessibility_label,
    );

    let delegate = TextAreaDelegate::new(mtm, events, content.text(), content.revision(), role);
    *delegate.ivars().text_view.borrow_mut() = Some(text_view.clone());
    // SAFETY: NSTextView and NSTextStorage delegates are non-owning; the
    // delegate is retained by the AppKitHandle for the view's lifetime.
    // The bounds observation is balanced by removeObserver in the backend's
    // destroy for this element.
    unsafe {
        let _: () = msg_send![text_view.as_object(), setDelegate: &*delegate];
        let storage: *mut AnyObject = msg_send![text_view.as_object(), textStorage];
        let _: () = msg_send![storage, setDelegate: &*delegate];
        let clip: *mut AnyObject = msg_send![scroll.as_object(), contentView];
        let _: () = msg_send![clip, setPostsBoundsChangedNotifications: true];
        let center: *mut AnyObject =
            msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
        let _: () = msg_send![center,
            addObserver: &*delegate,
            selector: sel!(viewBoundsDidChange:),
            name: VIEW_BOUNDS_DID_CHANGE_NOTIFICATION,
            object: clip
        ];
    }
    apply_text_area_spans(&delegate, text_view.as_object(), spans);
    apply_text_area_selection(&delegate, text_view.as_object(), selection);

    let handle = AppKitHandle::new_container(
        scroll,
        text_view,
        HostKind::Element(ElementKind::TextArea),
        None,
        Vec::new(),
    );
    *handle.0.text_delegate.borrow_mut() = Some(delegate);
    handle
}

/// Applies a semantic typography role to the editable view.
///
/// # Safety
///
/// The receiver must be a live NSTextView used on the main thread.
unsafe fn apply_text_area_role(text_view: &AnyObject, role: TextRole) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        let font = text_role_font(role);
        let _: () = msg_send![text_view, setFont: font];
        // Monospace text is verbatim source: automatic substitutions would
        // corrupt code, so the role decides them semantically.
        let verbatim = role == TextRole::Monospace;
        let _: () = msg_send![text_view, setAutomaticQuoteSubstitutionEnabled: !verbatim];
        let _: () = msg_send![text_view, setAutomaticDashSubstitutionEnabled: !verbatim];
        let _: () = msg_send![text_view, setAutomaticTextReplacementEnabled: !verbatim];
        let _: () = msg_send![text_view, setAutomaticSpellingCorrectionEnabled: !verbatim];
        let _: () = msg_send![text_view, setContinuousSpellCheckingEnabled: !verbatim];
    }
}

/// Reconciles one text-area property snapshot into the retained native view.
fn apply_text_area(handle: &AppKitHandle, config: TextAreaConfig<'_>) -> Result<(), AppKitError> {
    let TextAreaConfig {
        content,
        spans,
        selection,
        read_only,
        role,
        accessibility_label,
    } = config;
    let delegate = handle
        .0
        .text_delegate
        .borrow()
        .clone()
        .ok_or_else(|| AppKitError("text area handle has no native text delegate".to_owned()))?;
    let text_view = delegate
        .text_view()
        .ok_or_else(|| AppKitError("text area delegate lost its native view".to_owned()))?;
    set_string(handle.view(), SET_ACCESSIBILITY_LABEL, accessibility_label);
    set_string(
        text_view.as_object(),
        SET_ACCESSIBILITY_LABEL,
        accessibility_label,
    );
    // SAFETY: setEditable is a main-thread NSTextView setter.
    unsafe {
        let _: () = msg_send![text_view.as_object(), setEditable: !read_only];
    }
    if delegate.ivars().role.get() != role {
        delegate.ivars().role.set(role);
        // SAFETY: The receiver is the live text view retained by the delegate.
        unsafe { apply_text_area_role(text_view.as_object(), role) };
    }
    if delegate.has_marked_text() {
        // An IME composition owns the buffer. Every non-echo mutation —
        // content, spans (the input method renders its preedit underline
        // through the same temporary-attribute channel), and selection — is
        // deferred until the composition ends; only the latest deferred
        // state matters.
        *delegate.ivars().deferred.borrow_mut() = Some(DeferredTextArea {
            content: content.clone(),
            spans: spans.clone(),
            selection,
        });
        return Ok(());
    }
    // This declaration is newer than anything deferred during a composition;
    // an earlier snapshot applied afterwards would regress the view state.
    *delegate.ivars().deferred.borrow_mut() = None;
    apply_text_area_content(&delegate, text_view.as_object(), content)?;
    apply_text_area_spans(&delegate, text_view.as_object(), spans);
    apply_text_area_selection(&delegate, text_view.as_object(), selection);
    Ok(())
}

/// Synchronizes the native buffer with declared content per
/// `TextContent::sync_action`: echoes keep the buffer untouched, declared
/// deltas apply in place, and everything else replaces the document.
fn apply_text_area_content(
    delegate: &TextAreaDelegate,
    text_view: &AnyObject,
    content: &TextContent,
) -> Result<(), AppKitError> {
    match content.sync_action(delegate.ivars().revision.get()) {
        TextSyncAction::Keep => Ok(()),
        TextSyncAction::ApplyEdits(edits) => {
            delegate.ivars().applying.set(true);
            // SAFETY: The storage belongs to the live text view; edits are
            // applied inside one beginEditing/endEditing transaction.
            let result = unsafe {
                let storage: *mut AnyObject = msg_send![text_view, textStorage];
                let _: () = msg_send![storage, beginEditing];
                let mut applied = Ok(());
                {
                    let mut mirror = delegate.ivars().mirror.borrow_mut();
                    for edit in edits {
                        let Some(native_range) = locate_char_range(&mirror, edit.range) else {
                            applied = Err(AppKitError(format!(
                                "declared edit {}..{} exceeds the native buffer",
                                edit.range.start, edit.range.end
                            )));
                            break;
                        };
                        let byte_range = match rinka_core::char_range_to_byte_range(
                            &mirror, edit.range,
                        ) {
                            Some(byte_range) => byte_range,
                            None => {
                                applied = Err(AppKitError(format!(
                                    "declared edit {}..{} exceeds the native buffer",
                                    edit.range.start, edit.range.end
                                )));
                                break;
                            }
                        };
                        let replacement = ns_string(&edit.replacement);
                        let _: () = msg_send![storage,
                            replaceCharactersInRange: native_range,
                            withString: replacement.as_object()
                        ];
                        mirror.replace_range(byte_range, &edit.replacement);
                    }
                }
                let _: () = msg_send![storage, endEditing];
                applied
            };
            delegate.ivars().applying.set(false);
            result?;
            // Recorded user undo ranges no longer address this document.
            // SAFETY: undoManager is a main-thread NSTextView query.
            unsafe { clear_text_undo(text_view) };
            delegate.ivars().revision.set(content.revision());
            Ok(())
        }
        TextSyncAction::Replace => {
            let started = std::time::Instant::now();
            delegate.ivars().applying.set(true);
            // SAFETY: setString replaces the live text view's whole document.
            unsafe {
                let value = ns_string(content.text());
                let _: () = msg_send![text_view, setString: value.as_object()];
            }
            delegate.ivars().applying.set(false);
            delegate.ivars().mirror.replace(content.text().to_owned());
            // SAFETY: undoManager is a main-thread NSTextView query.
            unsafe { clear_text_undo(text_view) };
            delegate.ivars().revision.set(content.revision());
            // The replaced document dropped every temporary attribute.
            delegate.ivars().spans_revision.set(None);
            if std::env::var_os("RINKA_APPKIT_TEXTAREA_PROBE").is_some() {
                eprintln!(
                    "Rinka textarea replace chars={} micros={}",
                    content.char_len(),
                    started.elapsed().as_micros()
                );
            }
            Ok(())
        }
    }
}

/// Adopts a changed span set and materializes it over the visible range.
///
/// The stored set stays authoritative for regions revealed later by
/// scrolling; an unchanged revision is a no-op.
fn apply_text_area_spans(
    delegate: &TextAreaDelegate,
    _text_view: &AnyObject,
    spans: &HighlightSpans,
) {
    if delegate.ivars().spans_revision.get() == Some(spans.revision()) {
        return;
    }
    let started = std::time::Instant::now();
    delegate
        .ivars()
        .declared_spans
        .replace(spans.spans().to_vec());
    delegate.ivars().spans_revision.set(Some(spans.revision()));
    delegate.apply_visible_spans();
    if std::env::var_os("RINKA_APPKIT_TEXTAREA_PROBE").is_some() {
        eprintln!(
            "Rinka textarea spans adopted count={} micros={}",
            spans.spans().len(),
            started.elapsed().as_micros()
        );
    }
}

/// Applies a controlled selection; an echo of the native selection is a
/// no-op, and a genuinely new selection also scrolls the caret into view.
fn apply_text_area_selection(
    delegate: &TextAreaDelegate,
    text_view: &AnyObject,
    selection: Option<TextSelection>,
) {
    let Some(selection) = selection else {
        return;
    };
    let target = {
        let mirror = delegate.ivars().mirror.borrow();
        locate_char_range(&mirror, selection.range())
    };
    let Some(target) = target else {
        return;
    };
    // SAFETY: Selection getters and setters are main-thread NSTextView calls.
    unsafe {
        let current: NSRange = msg_send![text_view, selectedRange];
        if current.location == target.location && current.length == target.length {
            return;
        }
        delegate.ivars().applying.set(true);
        let _: () = msg_send![text_view, setSelectedRange: target];
        delegate.ivars().applying.set(false);
        let _: () = msg_send![text_view, scrollRangeToVisible: target];
    }
}

/// Drops recorded undo actions whose ranges no longer address the document.
///
/// # Safety
///
/// The receiver must be a live NSTextView used on the main thread.
unsafe fn clear_text_undo(text_view: &AnyObject) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        let undo: *mut AnyObject = msg_send![text_view, undoManager];
        if !undo.is_null() {
            let _: () = msg_send![undo, removeAllActions];
        }
    }
}

#[cfg(test)]
mod text_conversion_tests {
    use super::*;

    #[test]
    fn utf16_ranges_locate_chars_and_bytes_across_planes() {
        // "aあ😀b": a=1 utf16, あ=1, 😀=2 (surrogate pair), b=1.
        let text = "aあ😀b";
        let located = locate_utf16_range(text, 1, 4).expect("range covers あ😀");
        assert_eq!(located.char_range, TextRange::new(1, 3));
        assert_eq!(&text[located.byte_range], "あ😀");

        assert!(locate_utf16_range(text, 1, 3).is_none(), "splits a pair");
        let end = locate_utf16_range(text, 5, 5).expect("end of buffer");
        assert_eq!(end.char_range, TextRange::new(4, 4));
    }

    #[test]
    fn char_ranges_locate_utf16_ranges() {
        let text = "aあ😀b";
        let range = locate_char_range(text, TextRange::new(2, 4)).expect("😀b");
        assert_eq!((range.location, range.length), (2, 3));
        assert!(locate_char_range(text, TextRange::new(4, 5)).is_none());
    }

    #[test]
    fn window_conversion_clamps_and_rebases_spans_to_the_viewport() {
        // Document: "abcあdefghij" — window covers chars 3..8 ("あdefg"),
        // which starts at UTF-16 offset 3 as well (all ASCII before it).
        let text = "abcあdefghij";
        let window_chars = TextRange::new(3, 8);
        let window_text = "あdefg";
        let spans = [
            HighlightSpan::new(TextRange::new(0, 2), HighlightRole::Comment), // before
            HighlightSpan::new(TextRange::new(2, 5), HighlightRole::Keyword), // straddles start
            HighlightSpan::new(TextRange::new(6, 10), HighlightRole::String), // straddles end
            HighlightSpan::new(TextRange::new(10, 11), HighlightRole::Number), // after
        ];
        assert_eq!(text.chars().count(), 11);

        let converted = spans_to_utf16_ranges_in_window(window_text, window_chars, 3, &spans);

        assert_eq!(converted.len(), 2);
        // Keyword clamped to chars 3..5 → utf16 3..5 (あ is one utf16 unit).
        assert_eq!(
            (converted[0].0.location, converted[0].0.length),
            (3, 2)
        );
        assert_eq!(converted[0].1, HighlightRole::Keyword);
        // String clamped to chars 6..8 → utf16 6..8.
        assert_eq!(
            (converted[1].0.location, converted[1].0.length),
            (6, 2)
        );
        assert_eq!(converted[1].1, HighlightRole::String);
    }

    #[test]
    fn ordered_spans_convert_in_one_pass_with_clamping() {
        let text = "fn 名前()";
        let spans = [
            HighlightSpan::new(TextRange::new(0, 2), HighlightRole::Keyword),
            HighlightSpan::new(TextRange::new(3, 5), HighlightRole::Function),
            HighlightSpan::new(TextRange::new(20, 25), HighlightRole::String),
        ];
        let converted = spans_to_utf16_ranges(text, &spans);
        assert_eq!(converted.len(), 2, "the out-of-range span is dropped");
        assert_eq!(
            (converted[0].0.location, converted[0].0.length),
            (0, 2)
        );
        assert_eq!(converted[0].1, HighlightRole::Keyword);
        assert_eq!(
            (converted[1].0.location, converted[1].0.length),
            (3, 2)
        );
    }
}
