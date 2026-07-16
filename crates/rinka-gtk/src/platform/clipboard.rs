// GDK clipboard service: synchronous writes, asynchronous reads.
//
// Async delivery decision, recorded for `reports/clipboard-access`:
// `gdk::Clipboard` reads complete on the GLib main context, so this adapter
// never blocks the UI thread and never nests a main-loop iteration. The
// async completion wraps the core `deliver` callback — which is the
// component's dispatch-mapping closure — so the outcome enters the
// component's message queue exactly like a native signal. Writes
// (`gdk_clipboard_set_text`) are synchronous and report success directly.

/// Clipboard service over the default display's clipboard.
struct DisplayClipboard {
    clipboard: gtk::gdk::Clipboard,
}

impl rinka_core::ClipboardService for DisplayClipboard {
    fn write_text(&self, text: &str) -> Result<(), rinka_core::ClipboardError> {
        self.clipboard.set_text(text);
        Ok(())
    }

    fn read_text(
        &self,
        deliver: Box<dyn FnOnce(Result<Option<String>, rinka_core::ClipboardError>)>,
    ) {
        self.clipboard
            .read_text_async(None::<&gio::Cancellable>, move |result| {
                deliver(match result {
                    Ok(Some(text)) => Ok(Some(text.to_string())),
                    Ok(None) => Ok(None),
                    // GDK reports a clipboard without readable text as
                    // G_IO_ERROR_NOT_SUPPORTED; that is an empty read, not a
                    // platform failure.
                    Err(error) if error.matches(gio::IOErrorEnum::NotSupported) => Ok(None),
                    Err(error) => Err(rinka_core::ClipboardError::Platform {
                        reason: error.to_string(),
                    }),
                });
            });
    }
}

/// Builds the GTK host's service registry over the default display.
fn display_platform_services() -> Result<rinka_core::PlatformServices, GtkError> {
    let display = gtk::gdk::Display::default().ok_or_else(|| {
        GtkError("clipboard access requires a default GDK display".to_owned())
    })?;
    Ok(rinka_core::PlatformServices::new(rinka_core::Clipboard::new(
        DisplayClipboard {
            clipboard: display.clipboard(),
        },
    )))
}
