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
    role: Cell<TextRole>,
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

        /// Deferred to the next main-loop turn so event emission never runs
        /// inside NSTextStorage's edit transaction, and rapid storage edits
        /// in one turn coalesce into a single delta event.
        #[unsafe(method(drainPendingTextEdits:))]
        fn drain_pending_text_edits(&self, _sender: *mut AnyObject) {
            self.ivars().emission_scheduled.set(false);
            let edits = std::mem::take(&mut *self.ivars().pending_edits.borrow_mut());
            let composing = self.has_marked_text();
            if !edits.is_empty() {
                let base_revision = self.ivars().revision.get();
                let revision = base_revision.next_edit();
                self.ivars().revision.set(revision);
                self.ivars().events.emit_text_change(TextChange {
                    base_revision,
                    revision,
                    edits,
                    composing,
                });
            }
            if self.ivars().selection_pending.replace(false) {
                self.emit_native_selection();
            }
            if !composing {
                self.apply_deferred();
            }
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
            role: Cell::new(role),
        });
        // SAFETY: NSObject's init signature and ownership convention are stable.
        unsafe { msg_send![super(object), init] }
    }

    fn text_view(&self) -> Option<Id> {
        self.ivars().text_view.borrow().clone()
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
    unsafe {
        let _: () = msg_send![text_view.as_object(), setDelegate: &*delegate];
        let storage: *mut AnyObject = msg_send![text_view.as_object(), textStorage];
        let _: () = msg_send![storage, setDelegate: &*delegate];
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

/// Applies highlight spans as foreground-color temporary attributes.
fn apply_text_area_spans(
    delegate: &TextAreaDelegate,
    text_view: &AnyObject,
    spans: &HighlightSpans,
) {
    if delegate.ivars().spans_revision.get() == Some(spans.revision()) {
        return;
    }
    let started = std::time::Instant::now();
    let converted = {
        let mirror = delegate.ivars().mirror.borrow();
        spans_to_utf16_ranges(&mirror, spans.spans())
    };
    let buffer_utf16: usize = {
        let mirror = delegate.ivars().mirror.borrow();
        mirror.encode_utf16().count()
    };
    // SAFETY: The layout manager belongs to the live text view; temporary
    // attributes take UTF-16 character ranges inside the current document.
    unsafe {
        let layout: *mut AnyObject = msg_send![text_view, layoutManager];
        if layout.is_null() {
            return;
        }
        let attribute = Id::from_borrowed(FOREGROUND_COLOR_ATTRIBUTE_NAME);
        let full = NSRange {
            location: 0,
            length: buffer_utf16,
        };
        let _: () = msg_send![layout,
            removeTemporaryAttribute: attribute.as_object(),
            forCharacterRange: full
        ];
        for (range, role) in &converted {
            let color = highlight_color(*role);
            let _: () = msg_send![layout,
                addTemporaryAttribute: attribute.as_object(),
                value: color.as_object(),
                forCharacterRange: *range
            ];
        }
    }
    delegate.ivars().spans_revision.set(Some(spans.revision()));
    if std::env::var_os("RINKA_APPKIT_TEXTAREA_PROBE").is_some() {
        eprintln!(
            "Rinka textarea spans applied count={} micros={}",
            converted.len(),
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
