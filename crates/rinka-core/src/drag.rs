//! Platform-neutral drag-and-drop contracts.
//!
//! Three layers share this vocabulary (`reports/drag-and-drop`):
//!
//! 1. **Operating-system file drop-in** — an element declares
//!    [`crate::Element::on_file_drop`] and receives a [`FileDrop`]: the
//!    dropped file paths plus the drop position in element-local
//!    coordinates.
//! 2. **File drag-out** — an element declares
//!    [`crate::Element::draggable_file`] with a [`FilePromise`]; the file
//!    content materializes lazily, through the promise's write callback,
//!    only when a destination accepts the drop.
//! 3. **Intra-application item drag** — an element declares
//!    [`crate::Element::drag_payload`] with a typed [`DragPayload`] and a
//!    target declares [`crate::Element::on_drop_accepting`]; the accepted
//!    payload arrives as a [`PayloadDrop`].
//!
//! The declarative models here compare by their descriptive data only.
//! Callbacks are excluded from equality for the same reason context-menu
//! activation handlers are: handlers reach the adapters through the stable
//! event binding on every render regardless of property patching.

use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Drop or hover position in element-local coordinates.
///
/// The origin is the target element's top-left corner and values are logical
/// points. Adapters convert from platform coordinate spaces; consumers never
/// see window or screen geometry (`AGENTS.md` visual rule 3).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DropPosition {
    /// Distance from the element's leading edge.
    pub x: f64,
    /// Distance from the element's top edge.
    pub y: f64,
}

impl DropPosition {
    /// Creates a position.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Files the operating system dropped onto an accepting element.
#[derive(Clone, Debug, PartialEq)]
pub struct FileDrop {
    /// Absolute paths of the dropped files in pasteboard order.
    pub paths: Vec<PathBuf>,
    /// Drop position in the target element's local coordinates.
    pub position: DropPosition,
}

/// Typed intra-application payload carried by a drag source.
///
/// The payload is pure data — a consumer-defined type identifier plus the
/// dragged item's stable identity — so it survives any transport, including
/// the operating-system pasteboard, without an in-process session registry.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DragPayload {
    payload_type: String,
    id: String,
}

impl DragPayload {
    /// Creates a payload from a consumer-defined type identifier and the
    /// dragged item's identity within that type.
    pub fn new(payload_type: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            payload_type: payload_type.into(),
            id: id.into(),
        }
    }

    /// Returns the consumer-defined payload type identifier.
    pub fn payload_type(&self) -> &str {
        &self.payload_type
    }

    /// Returns the dragged item's identity within its payload type.
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// A typed payload dropped onto an accepting element.
#[derive(Clone, Debug, PartialEq)]
pub struct PayloadDrop {
    /// The payload declared by the drag source.
    pub payload: DragPayload,
    /// Drop position in the target element's local coordinates.
    pub position: DropPosition,
}

/// Callback that writes one promised file to the destination path.
///
/// It runs only when a destination materializes the promise — never
/// speculatively — so the consumer may perform its real export (for
/// Overshell, fetching remote bytes over SSH) inside it. The error string
/// surfaces to the platform as the promise failure reason and to the
/// consumer through the callback's own follow-up dispatch.
pub type FilePromiseWriter = Rc<dyn Fn(&Path) -> Result<(), String>>;

/// Lazily materialized file export attached to a drag source.
///
/// Equality compares the declared file name and content type only; the
/// write callback is refreshed through the stable event binding on every
/// render, so a handler-only change never patches the native drag source.
#[derive(Clone)]
pub struct FilePromise {
    file_name: String,
    content_type: String,
    write: FilePromiseWriter,
}

impl FilePromise {
    /// Creates a promise from the exported file name, its uniform type
    /// identifier, and the write callback that materializes the content.
    pub fn new(
        file_name: impl Into<String>,
        content_type: impl Into<String>,
        write: impl Fn(&Path) -> Result<(), String> + 'static,
    ) -> Self {
        Self {
            file_name: file_name.into(),
            content_type: content_type.into(),
            write: Rc::new(write),
        }
    }

    /// Returns the bare file name the destination should create.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// Returns the promised content's uniform type identifier.
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// Materializes the promised content at the destination path.
    ///
    /// Platform adapters call this exactly once per accepted drop, from the
    /// UI thread, after the destination chose where the file lands.
    pub fn write_to(&self, destination: &Path) -> Result<(), String> {
        (self.write)(destination)
    }
}

impl PartialEq for FilePromise {
    fn eq(&self, other: &Self) -> bool {
        self.file_name == other.file_name && self.content_type == other.content_type
    }
}

impl fmt::Debug for FilePromise {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilePromise")
            .field("file_name", &self.file_name)
            .field("content_type", &self.content_type)
            .finish_non_exhaustive()
    }
}

/// Declarative drop-target model attached to one element.
///
/// The model is pure data compared by value during reconciliation; the
/// handlers that consume the accepted drops ride in the element's event
/// binding beside it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DropTarget {
    files: bool,
    payload_types: Vec<String>,
}

impl DropTarget {
    pub(crate) fn accept_files(&mut self) {
        self.files = true;
    }

    pub(crate) fn accept_payload_types(&mut self, types: impl IntoIterator<Item = String>) {
        self.payload_types = types.into_iter().collect();
    }

    /// Returns whether the element accepts operating-system file drops.
    pub fn accepts_files(&self) -> bool {
        self.files
    }

    /// Returns the accepted intra-application payload type identifiers.
    pub fn payload_types(&self) -> &[String] {
        &self.payload_types
    }

    /// Returns whether the element accepts a payload of the given type.
    pub fn accepts_payload_type(&self, payload_type: &str) -> bool {
        self.payload_types
            .iter()
            .any(|accepted| accepted == payload_type)
    }
}

#[cfg(test)]
mod tests {
    use super::{DragPayload, DropPosition, DropTarget, FilePromise};
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn a_file_promise_compares_by_its_declared_descriptor_only() {
        let first = FilePromise::new("notes.txt", "public.plain-text", |_| Ok(()));
        let same_descriptor = FilePromise::new("notes.txt", "public.plain-text", |_| {
            Err("different callback".to_owned())
        });
        let different_name = FilePromise::new("other.txt", "public.plain-text", |_| Ok(()));

        assert_eq!(first, same_descriptor);
        assert_ne!(first, different_name);
    }

    #[test]
    fn a_file_promise_materializes_through_its_write_callback() {
        let written = Rc::new(Cell::new(false));
        let observed = written.clone();
        let promise = FilePromise::new("notes.txt", "public.plain-text", move |path| {
            assert_eq!(
                path.file_name().and_then(|name| name.to_str()),
                Some("notes.txt")
            );
            observed.set(true);
            Ok(())
        });

        promise
            .write_to(std::path::Path::new("/tmp/notes.txt"))
            .expect("write succeeds");
        assert!(written.get());
    }

    #[test]
    fn a_drop_target_accepts_only_its_declared_payload_types() {
        let mut target = DropTarget::default();
        target.accept_payload_types(["demo.file".to_owned()]);

        assert!(target.accepts_payload_type("demo.file"));
        assert!(!target.accepts_payload_type("demo.widget"));
        assert!(!target.accepts_files());
    }

    #[test]
    fn payload_and_position_expose_their_declared_data() {
        let payload = DragPayload::new("demo.file", "readme");
        assert_eq!(payload.payload_type(), "demo.file");
        assert_eq!(payload.id(), "readme");

        let position = DropPosition::new(12.5, 40.0);
        assert_eq!(position.x, 12.5);
        assert_eq!(position.y, 40.0);
    }
}
