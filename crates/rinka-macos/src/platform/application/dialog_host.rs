// Window-modal dialog presentation: NSAlert, NSOpenPanel, and NSSavePanel
// sheets attached to the owning window, never app-modal run loops.
//
// The destructive idiom is enforced twice: core validation rejects a
// description whose return-key default is destructive before it reaches this
// host, and this host clears every implicit AppKit key equivalent so only
// the validated `default_button` ever receives the return key.

/// `NSAlertFirstButtonReturn`.
const NS_ALERT_FIRST_BUTTON_RETURN: isize = 1000;
/// `NSModalResponseOK`.
const NS_MODAL_RESPONSE_OK: isize = 1;

/// Presents one window's dialog requests as AppKit sheets on that window.
struct AppKitWindowDialogService {
    window: Id,
}

impl DialogService for AppKitWindowDialogService {
    fn present(&self, request: DialogRequest) {
        // SAFETY: Dialog requests are raised by component updates, which the
        // host only drives from AppKit's main thread; the retained NSWindow
        // outlives its runtime inside the application delegate.
        unsafe { present_dialog_request(self.window.as_object(), request) };
    }
}

/// Presents one validated dialog request as a sheet on `window`.
///
/// # Safety
///
/// `window` must be a live NSWindow used on AppKit's main thread.
unsafe fn present_dialog_request(window: &AnyObject, request: DialogRequest) {
    let (description, responder) = request.into_parts();
    // SAFETY: Deferred to each presentation helper; all run on main.
    unsafe {
        match description {
            DialogDescription::Alert(alert) => present_alert_sheet(window, &alert, responder),
            DialogDescription::OpenPanel(panel) => {
                present_open_panel_sheet(window, &panel, responder);
            }
            DialogDescription::SavePanel(panel) => {
                present_save_panel_sheet(window, &panel, responder);
            }
        }
    }
}

/// Presents an alert sheet via `beginSheetModalForWindow:completionHandler:`.
///
/// # Safety
///
/// `window` must be a live NSWindow used on AppKit's main thread.
unsafe fn present_alert_sheet(
    window: &AnyObject,
    description: &rinka_core::AlertDescription,
    responder: DialogResponder,
) {
    let alert = new_object(objc2::class!(NSAlert));
    // SAFETY: NSAlert text, button, and key-equivalent properties are public
    // AppKit API applied to the alert this host just created.
    unsafe {
        let title = ns_string(&description.title);
        let body = ns_string(&description.body);
        let _: () = msg_send![alert.as_object(), setMessageText: title.as_object()];
        let _: () = msg_send![alert.as_object(), setInformativeText: body.as_object()];
        let empty = ns_string("");
        let escape = ns_string("\u{1b}");
        let return_key = ns_string("\r");
        for (index, button) in description.buttons.iter().enumerate() {
            let label = ns_string(&button.label);
            let native: *mut AnyObject =
                msg_send![alert.as_object(), addButtonWithTitle: label.as_object()];
            // AppKit gives the first button the return key and a button
            // titled "Cancel" the escape key implicitly; both are cleared so
            // the declared roles and the validated default are the only key
            // sources ("destructive stays destructive").
            let _: () = msg_send![native, setKeyEquivalent: empty.as_object()];
            match button.role {
                DialogButtonRole::Standard => {}
                DialogButtonRole::Cancel => {
                    let _: () = msg_send![native, setKeyEquivalent: escape.as_object()];
                }
                DialogButtonRole::Destructive => {
                    let _: () = msg_send![native, setHasDestructiveAction: true];
                }
            }
            if description.default_button == Some(index) {
                let _: () = msg_send![native, setKeyEquivalent: return_key.as_object()];
            }
        }
    }
    let responder = Cell::new(Some(responder));
    let alert_owner = alert.clone();
    let handler = block2::RcBlock::new(move |response: isize| {
        // The retained alert lives until its sheet completes.
        let _ = &alert_owner;
        let Some(responder) = responder.take() else {
            return;
        };
        let outcome = usize::try_from(response - NS_ALERT_FIRST_BUTTON_RETURN)
            .map_or(DialogOutcome::Cancelled, DialogOutcome::ButtonChosen);
        responder.deliver(outcome);
    });
    // SAFETY: beginSheetModalForWindow attaches the alert's panel to the
    // live parent window and copies the completion block.
    unsafe {
        let _: () = msg_send![alert.as_object(),
            beginSheetModalForWindow: window,
            completionHandler: &*handler
        ];
    }
}

/// Presents a file-open panel sheet returning the confirmed paths.
///
/// # Safety
///
/// `window` must be a live NSWindow used on AppKit's main thread.
unsafe fn present_open_panel_sheet(
    window: &AnyObject,
    description: &rinka_core::OpenPanelDescription,
    responder: DialogResponder,
) {
    // SAFETY: openPanel returns an autoreleased shared-style instance the
    // host retains; every configured property is public AppKit API.
    let panel = unsafe {
        let pointer: *mut AnyObject = msg_send![objc2::class!(NSOpenPanel), openPanel];
        Id::from_borrowed(pointer)
    };
    unsafe {
        let _: () = msg_send![panel.as_object(), setCanChooseFiles: description.choose_files];
        let _: () = msg_send![
            panel.as_object(),
            setCanChooseDirectories: description.choose_directories
        ];
        let _: () = msg_send![
            panel.as_object(),
            setAllowsMultipleSelection: description.allows_multiple
        ];
        configure_panel_commons(
            panel.as_object(),
            description.title.as_deref(),
            description.starting_directory.as_deref(),
        );
    }
    let responder = Cell::new(Some(responder));
    let panel_owner = panel.clone();
    let handler = block2::RcBlock::new(move |response: isize| {
        let Some(responder) = responder.take() else {
            return;
        };
        if response != NS_MODAL_RESPONSE_OK {
            responder.deliver(DialogOutcome::Cancelled);
            return;
        }
        // SAFETY: The retained panel's URL array is read on the main thread
        // inside its own completion handler.
        let paths = unsafe {
            let urls: *mut AnyObject = msg_send![panel_owner.as_object(), URLs];
            let count: usize = msg_send![urls, count];
            (0..count)
                .filter_map(|index| {
                    let url: *mut AnyObject = msg_send![urls, objectAtIndex: index];
                    let path: *mut AnyObject = msg_send![url, path];
                    let path = rust_string(path);
                    (!path.is_empty()).then(|| std::path::PathBuf::from(path))
                })
                .collect::<Vec<_>>()
        };
        responder.deliver(DialogOutcome::PathsChosen(paths));
    });
    // SAFETY: The sheet attaches to the live parent window; the copied block
    // retains the panel until completion.
    unsafe {
        let _: () = msg_send![panel.as_object(),
            beginSheetModalForWindow: window,
            completionHandler: &*handler
        ];
    }
}

/// Presents a file-save panel sheet returning the confirmed destination.
///
/// # Safety
///
/// `window` must be a live NSWindow used on AppKit's main thread.
unsafe fn present_save_panel_sheet(
    window: &AnyObject,
    description: &rinka_core::SavePanelDescription,
    responder: DialogResponder,
) {
    // SAFETY: savePanel returns an autoreleased instance the host retains;
    // every configured property is public AppKit API.
    let panel = unsafe {
        let pointer: *mut AnyObject = msg_send![objc2::class!(NSSavePanel), savePanel];
        Id::from_borrowed(pointer)
    };
    unsafe {
        if let Some(filename) = &description.suggested_filename {
            let filename = ns_string(filename);
            let _: () = msg_send![
                panel.as_object(),
                setNameFieldStringValue: filename.as_object()
            ];
        }
        configure_panel_commons(
            panel.as_object(),
            description.title.as_deref(),
            description.starting_directory.as_deref(),
        );
    }
    let responder = Cell::new(Some(responder));
    let panel_owner = panel.clone();
    let handler = block2::RcBlock::new(move |response: isize| {
        let Some(responder) = responder.take() else {
            return;
        };
        if response != NS_MODAL_RESPONSE_OK {
            responder.deliver(DialogOutcome::Cancelled);
            return;
        }
        // SAFETY: The retained panel's URL is read on the main thread inside
        // its own completion handler.
        let path = unsafe {
            let url: *mut AnyObject = msg_send![panel_owner.as_object(), URL];
            let path: *mut AnyObject = msg_send![url, path];
            rust_string(path)
        };
        if path.is_empty() {
            responder.deliver(DialogOutcome::Cancelled);
        } else {
            responder.deliver(DialogOutcome::SavePathChosen(std::path::PathBuf::from(path)));
        }
    });
    // SAFETY: The sheet attaches to the live parent window; the copied block
    // retains the panel until completion.
    unsafe {
        let _: () = msg_send![panel.as_object(),
            beginSheetModalForWindow: window,
            completionHandler: &*handler
        ];
    }
}

/// Applies the prompt and starting directory shared by both panel kinds.
///
/// # Safety
///
/// `panel` must be a live NSSavePanel (or subclass) on the main thread.
unsafe fn configure_panel_commons(
    panel: &AnyObject,
    title: Option<&str>,
    starting_directory: Option<&std::path::Path>,
) {
    // SAFETY: message and directoryURL are public NSSavePanel API; NSURL
    // copies the provided path string.
    unsafe {
        if let Some(title) = title {
            let message = ns_string(title);
            let _: () = msg_send![panel, setMessage: message.as_object()];
        }
        if let Some(directory) = starting_directory {
            let path = ns_string(&directory.to_string_lossy());
            let url: *mut AnyObject =
                msg_send![objc2::class!(NSURL), fileURLWithPath: path.as_object()];
            let _: () = msg_send![panel, setDirectoryURL: url];
        }
    }
}
