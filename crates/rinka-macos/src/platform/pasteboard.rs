// NSPasteboard implementation of the core clipboard service.
//
// Decision record for `reports/clipboard-access`: the host registers this
// service in the window's `PlatformServices` at mount; components reach it
// through `UpdateContext`. AppKit pasteboard reads are synchronous, so the
// read completion is invoked before `read_text` returns — the core runtimes
// queue messages emitted during a running update, which turns the
// synchronous completion into an ordinary follow-up message.
//
// NSPasteboard is documented thread-safe (AppKit release notes since
// macOS 10.5), which is what allows the unit tests below to exercise a
// uniquely named pasteboard from a Rust test thread; the application host
// itself only uses the service on the main thread.

unsafe extern "C" {
    #[link_name = "NSPasteboardTypeString"]
    static PASTEBOARD_TYPE_STRING: *mut AnyObject;
}

/// Clipboard service over one retained NSPasteboard.
#[derive(Clone)]
struct PasteboardClipboard {
    pasteboard: Id,
}

impl PasteboardClipboard {
    /// Wraps the system general pasteboard.
    fn general() -> Self {
        // SAFETY: generalPasteboard returns the shared pasteboard singleton
        // with non-owning conventions; the wrapper balances its own retain.
        let pointer: *mut AnyObject =
            unsafe { msg_send![objc2::class!(NSPasteboard), generalPasteboard] };
        Self {
            pasteboard: unsafe { Id::from_borrowed(pointer) },
        }
    }

    /// Wraps the named pasteboard, creating it on first use.
    ///
    /// Tests isolate their state on a uniquely named pasteboard so the
    /// user's general pasteboard is never touched; the owner must call
    /// [`Self::release_globally`] afterwards to remove it from the
    /// pasteboard server.
    #[cfg(test)]
    fn with_name(name: &str) -> Self {
        let name = ns_string(name);
        // SAFETY: pasteboardWithName: returns an autoreleased pasteboard
        // with non-owning conventions; the wrapper balances its own retain.
        let pointer: *mut AnyObject = unsafe {
            msg_send![objc2::class!(NSPasteboard), pasteboardWithName: name.as_object()]
        };
        Self {
            pasteboard: unsafe { Id::from_borrowed(pointer) },
        }
    }

    /// Releases a named pasteboard's server-side resources after a test.
    #[cfg(test)]
    fn release_globally(&self) {
        // SAFETY: releaseGlobally removes the named pasteboard from the
        // pasteboard server; the local retain held by `Id` stays balanced.
        unsafe {
            let _: () = msg_send![self.pasteboard.as_object(), releaseGlobally];
        }
    }
}

impl rinka_core::ClipboardService for PasteboardClipboard {
    fn write_text(&self, text: &str) -> Result<(), rinka_core::ClipboardError> {
        let string = ns_string(text);
        // SAFETY: clearContents and setString:forType: are public
        // NSPasteboard API on a retained receiver; the string and type
        // arguments are live NSString objects for the duration of the call.
        let stored: bool = unsafe {
            let _: isize = msg_send![self.pasteboard.as_object(), clearContents];
            msg_send![self.pasteboard.as_object(),
                setString: string.as_object(),
                forType: PASTEBOARD_TYPE_STRING
            ]
        };
        if stored {
            Ok(())
        } else {
            Err(rinka_core::ClipboardError::Platform {
                reason: "NSPasteboard rejected setString:forType:".to_owned(),
            })
        }
    }

    fn read_text(
        &self,
        deliver: Box<dyn FnOnce(Result<Option<String>, rinka_core::ClipboardError>)>,
    ) {
        // SAFETY: stringForType: returns an autoreleased NSString or nil on
        // a retained receiver; the bytes are copied into an owned Rust
        // String before the surrounding autorelease scope drains.
        let value: *mut AnyObject = unsafe {
            msg_send![self.pasteboard.as_object(), stringForType: PASTEBOARD_TYPE_STRING]
        };
        let text = NonNull::new(value).map(|value| rust_string(value.as_ptr()));
        deliver(Ok(text));
    }
}

/// Builds the AppKit host's service registry over the general pasteboard.
fn pasteboard_platform_services() -> rinka_core::PlatformServices {
    rinka_core::PlatformServices::new(rinka_core::Clipboard::new(PasteboardClipboard::general()))
}

#[cfg(test)]
mod pasteboard_tests {
    use super::PasteboardClipboard;
    use objc2::rc::autoreleasepool;
    use rinka_core::Clipboard;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Names one disposable pasteboard per test so parallel tests and
    /// parallel gate runs never share state — and never touch the user's
    /// general pasteboard.
    fn unique_test_pasteboard() -> PasteboardClipboard {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "jp.bunko.rinka.test-pasteboard.{}.{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        PasteboardClipboard::with_name(&name)
    }

    fn read_now(clipboard: &Clipboard) -> Option<String> {
        let delivered = Rc::new(RefCell::new(None));
        let sink = delivered.clone();
        clipboard.read_text(move |result| *sink.borrow_mut() = Some(result));
        delivered
            .borrow_mut()
            .take()
            .expect("NSPasteboard reads deliver synchronously")
            .expect("NSPasteboard read succeeds")
    }

    #[test]
    fn a_named_pasteboard_round_trips_cjk_and_multiline_text() {
        autoreleasepool(|_| {
            let service = unique_test_pasteboard();
            let clipboard = Clipboard::new(service.clone());
            for text in ["日本語\nline two", "first\nsecond\nthird", "ascii ✓"] {
                clipboard.write_text(text).expect("pasteboard write");
                assert_eq!(read_now(&clipboard).as_deref(), Some(text));
            }
            service.release_globally();
        });
    }

    #[test]
    fn a_cleared_named_pasteboard_reads_as_no_text() {
        autoreleasepool(|_| {
            let service = unique_test_pasteboard();
            let clipboard = Clipboard::new(service.clone());
            assert_eq!(read_now(&clipboard), None);
            service.release_globally();
        });
    }
}
