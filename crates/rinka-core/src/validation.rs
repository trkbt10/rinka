//! Declarative tree invariants checked before native mutation.

use crate::accelerator::AcceleratorScope;
use crate::{Element, ElementKind};
use std::collections::HashSet;
use std::error::Error;
use std::fmt;

/// Invalid declarative tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TreeError {
    /// Two siblings used the same key.
    DuplicateKey {
        /// Duplicated key text.
        key: String,
        /// Parent path used for diagnostics.
        parent: String,
    },
    /// A container received an invalid child count.
    ChildCount {
        /// Element path.
        path: String,
        /// Required minimum.
        minimum: usize,
        /// Required maximum.
        maximum: usize,
        /// Actual count.
        actual: usize,
    },
    /// A non-row element was placed directly in a list.
    InvalidListChild {
        /// Child path.
        path: String,
    },
    /// A list row used semantics that its presentation cannot represent.
    InvalidListSchema {
        /// Element path.
        path: String,
        /// Human-readable invariant violation.
        reason: String,
    },
    /// A table declared invalid columns or row values.
    InvalidTableSchema {
        /// Element path.
        path: String,
        /// Human-readable invariant violation.
        reason: String,
    },
    /// A canvas declared an invalid extent or an invalid recorded scene.
    InvalidCanvasScene {
        /// Element path.
        path: String,
        /// Human-readable invariant violation.
        reason: String,
    },
    /// An image element declared pixel geometry its buffer cannot satisfy.
    InvalidImage {
        /// Element path.
        path: String,
        /// Human-readable invariant violation.
        reason: String,
    },
    /// A context menu declared invalid entry identities.
    InvalidMenu {
        /// Element path.
        path: String,
        /// Human-readable invariant violation.
        reason: String,
    },
    /// A text area declared spans, edits, or a selection its document cannot
    /// satisfy.
    InvalidTextArea {
        /// Element path.
        path: String,
        /// Human-readable invariant violation.
        reason: String,
    },
    /// A mounted native window attempted to replace its promoted root class.
    WindowRootKindChanged {
        /// Root kind retained by the native window.
        previous: ElementKind,
        /// Root kind requested by the next component view.
        next: ElementKind,
    },
    /// Two accelerator entries in the same scope declared the same chord.
    DuplicateAcceleratorChord {
        /// Canonical chord text.
        chord: String,
        /// Scope containing the collision.
        scope: AcceleratorScope,
    },
    /// An accelerator table violated a structural invariant.
    InvalidAcceleratorTable {
        /// Element path used for diagnostics.
        path: String,
        /// Human-readable invariant violation.
        reason: String,
    },
    /// A drag-and-drop declaration violated a structural invariant.
    InvalidDragDeclaration {
        /// Element path used for diagnostics.
        path: String,
        /// Human-readable invariant violation.
        reason: String,
    },
}

impl fmt::Display for TreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateKey { key, parent } => {
                write!(formatter, "duplicate sibling key '{key}' under {parent}")
            }
            Self::ChildCount {
                path,
                minimum,
                maximum,
                actual,
            } => write!(
                formatter,
                "{path} requires {minimum}..={maximum} children, received {actual}"
            ),
            Self::InvalidListChild { path } => {
                write!(formatter, "list child at {path} is not a list row")
            }
            Self::InvalidListSchema { path, reason } => {
                write!(formatter, "invalid list at {path}: {reason}")
            }
            Self::InvalidTableSchema { path, reason } => {
                write!(formatter, "invalid table at {path}: {reason}")
            }
            Self::InvalidCanvasScene { path, reason } => {
                write!(formatter, "invalid canvas at {path}: {reason}")
            }
            Self::InvalidImage { path, reason } => {
                write!(formatter, "invalid image at {path}: {reason}")
            }
            Self::InvalidMenu { path, reason } => {
                write!(formatter, "invalid context menu at {path}: {reason}")
            }
            Self::InvalidTextArea { path, reason } => {
                write!(formatter, "invalid text area at {path}: {reason}")
            }
            Self::WindowRootKindChanged { previous, next } => write!(
                formatter,
                "native window root kind must remain stable: mounted {previous:?}, received {next:?}"
            ),
            Self::DuplicateAcceleratorChord { chord, scope } => write!(
                formatter,
                "duplicate accelerator chord '{chord}' in {scope} scope"
            ),
            Self::InvalidAcceleratorTable { path, reason } => {
                write!(formatter, "invalid accelerator table at {path}: {reason}")
            }
            Self::InvalidDragDeclaration { path, reason } => {
                write!(formatter, "invalid drag declaration at {path}: {reason}")
            }
        }
    }
}

impl Error for TreeError {}

pub(crate) fn validate_tree(root: &Element) -> Result<(), TreeError> {
    validate_accelerator_table(root)?;
    validate_node(root, "root")
}

fn validate_accelerator_table(root: &Element) -> Result<(), TreeError> {
    let mut identities = HashSet::new();
    let mut chords = HashSet::new();
    for entry in root.accelerator_table() {
        if entry.id().is_empty() {
            return Err(TreeError::InvalidAcceleratorTable {
                path: "root".to_owned(),
                reason: format!("accelerator for chord '{}' has an empty id", entry.chord()),
            });
        }
        if !identities.insert(entry.id().to_owned()) {
            return Err(TreeError::InvalidAcceleratorTable {
                path: "root".to_owned(),
                reason: format!("duplicate accelerator id '{}'", entry.id()),
            });
        }
        if !chords.insert((entry.declared_scope(), entry.chord())) {
            return Err(TreeError::DuplicateAcceleratorChord {
                chord: entry.chord().to_string(),
                scope: entry.declared_scope(),
            });
        }
    }
    validate_accelerator_placement(root.children(), "root")
}

fn validate_accelerator_placement(children: &[Element], path: &str) -> Result<(), TreeError> {
    for (index, child) in children.iter().enumerate() {
        let name = child
            .key()
            .map_or_else(|| index.to_string(), |key| key.as_str().to_owned());
        let child_path = format!("{path}/{name}");
        if !child.accelerator_table().is_empty() {
            return Err(TreeError::InvalidAcceleratorTable {
                path: child_path,
                reason: "accelerators must be declared on the window content root".to_owned(),
            });
        }
        validate_accelerator_placement(child.children(), &child_path)?;
    }
    Ok(())
}

fn validate_node(element: &Element, path: &str) -> Result<(), TreeError> {
    if let Some(menu) = element.context_menu_model()
        && let Err(reason) = menu.validate_identities()
    {
        return Err(TreeError::InvalidMenu {
            path: path.to_owned(),
            reason,
        });
    }

    validate_drag_declarations(element, path)?;

    let children = element.children();
    let mut keys = HashSet::new();
    for child in children {
        if let Some(key) = child.key()
            && !keys.insert(key.as_str())
        {
            return Err(TreeError::DuplicateKey {
                key: key.as_str().to_owned(),
                parent: path.to_owned(),
            });
        }
    }

    let exact = match element.kind() {
        crate::ElementKind::Scroll => Some(1),
        crate::ElementKind::Pattern => match element.props() {
            crate::Props::Pattern { pattern } => Some(pattern.regions().len()),
            _ => unreachable!("pattern kind must contain pattern properties"),
        },
        crate::ElementKind::Label
        | crate::ElementKind::Button
        | crate::ElementKind::Input
        | crate::ElementKind::TextArea
        | crate::ElementKind::Toggle
        | crate::ElementKind::Progress
        | crate::ElementKind::Image
        | crate::ElementKind::Separator
        | crate::ElementKind::Spacer
        | crate::ElementKind::Status
        | crate::ElementKind::Canvas => Some(0),
        crate::ElementKind::Stack | crate::ElementKind::List | crate::ElementKind::ListRow => None,
    };
    if let Some(exact) = exact
        && children.len() != exact
    {
        return Err(TreeError::ChildCount {
            path: path.to_owned(),
            minimum: exact,
            maximum: exact,
            actual: children.len(),
        });
    }

    if element.kind() == crate::ElementKind::Canvas {
        validate_canvas(element, path)?;
    }

    if element.kind() == crate::ElementKind::Image
        && let crate::Props::Image { content, .. } = element.props()
        && let Some(reason) = content.validity_error()
    {
        return Err(TreeError::InvalidImage {
            path: path.to_owned(),
            reason,
        });
    }

    if element.kind() == crate::ElementKind::TextArea {
        validate_text_area(element, path)?;
    }

    if element.kind() == crate::ElementKind::List {
        for (index, child) in children.iter().enumerate() {
            if child.kind() != crate::ElementKind::ListRow {
                return Err(TreeError::InvalidListChild {
                    path: format!("{path}/{index}"),
                });
            }
        }
        validate_table_schema(element, path)?;
    }
    if element.kind() == crate::ElementKind::ListRow {
        for (index, child) in children.iter().enumerate() {
            if child.kind() != crate::ElementKind::ListRow {
                return Err(TreeError::InvalidListChild {
                    path: format!("{path}/{index}"),
                });
            }
        }
    }

    for (index, child) in children.iter().enumerate() {
        let name = child
            .key()
            .map_or_else(|| index.to_string(), |key| key.as_str().to_owned());
        validate_node(child, &format!("{path}/{name}"))?;
    }
    Ok(())
}

fn validate_text_area(element: &Element, path: &str) -> Result<(), TreeError> {
    let crate::Props::TextArea {
        content,
        spans,
        selection,
        ..
    } = element.props()
    else {
        return Ok(());
    };
    let error = |reason: String| TreeError::InvalidTextArea {
        path: path.to_owned(),
        reason,
    };
    let length = content.char_len();
    for edit in content.edits() {
        if edit.range.end < edit.range.start {
            return Err(error(format!(
                "edit range {}..{} is inverted",
                edit.range.start, edit.range.end
            )));
        }
    }
    let mut previous_end = 0_usize;
    for span in spans.spans() {
        if span.range.end <= span.range.start {
            return Err(error(format!(
                "highlight span {}..{} is empty or inverted",
                span.range.start, span.range.end
            )));
        }
        if span.range.start < previous_end {
            return Err(error(format!(
                "highlight span {}..{} overlaps or precedes the span ending at {previous_end}; \
                 spans must be ordered and non-overlapping",
                span.range.start, span.range.end
            )));
        }
        if span.range.end > length {
            return Err(error(format!(
                "highlight span {}..{} exceeds the {length}-character document",
                span.range.start, span.range.end
            )));
        }
        previous_end = span.range.end;
    }
    if let Some(selection) = selection
        && (selection.anchor > length || selection.head > length)
    {
        return Err(error(format!(
            "selection {}..{} exceeds the {length}-character document",
            selection.anchor, selection.head
        )));
    }
    Ok(())
}

/// Rejects drag-and-drop declarations whose descriptive data cannot reach a
/// native session: a promise without a legal bare file name or content type,
/// a payload without a transportable type and identity, and a drop target
/// whose accepted type list is empty or ambiguous.
fn validate_drag_declarations(element: &Element, path: &str) -> Result<(), TreeError> {
    let invalid = |reason: String| TreeError::InvalidDragDeclaration {
        path: path.to_owned(),
        reason,
    };
    if let Some(promise) = element.file_promise_model() {
        if promise.file_name().is_empty() {
            return Err(invalid("promised file name is empty".to_owned()));
        }
        if promise.file_name().contains(['/', '\\'])
            || promise.file_name() == "."
            || promise.file_name() == ".."
        {
            return Err(invalid(format!(
                "promised file name '{}' must be a bare file name",
                promise.file_name()
            )));
        }
        if promise.content_type().is_empty() {
            return Err(invalid("promised content type is empty".to_owned()));
        }
    }
    if let Some(payload) = element.drag_payload_model() {
        if payload.payload_type().is_empty() {
            return Err(invalid("drag payload type is empty".to_owned()));
        }
        if payload.payload_type().contains('\n') {
            // The payload type frames the pasteboard transport encoding;
            // the identity after it may contain any text.
            return Err(invalid(format!(
                "drag payload type '{}' must not contain a line break",
                payload.payload_type()
            )));
        }
        if payload.id().is_empty() {
            return Err(invalid(format!(
                "drag payload of type '{}' has an empty id",
                payload.payload_type()
            )));
        }
    }
    if let Some(target) = element.drop_target_model() {
        let mut accepted = HashSet::new();
        for payload_type in target.payload_types() {
            if payload_type.is_empty() {
                return Err(invalid("accepted payload type is empty".to_owned()));
            }
            if !accepted.insert(payload_type.as_str()) {
                return Err(invalid(format!(
                    "accepted payload type '{payload_type}' is duplicated"
                )));
            }
        }
        if !target.accepts_files() && target.payload_types().is_empty() {
            return Err(invalid(
                "drop target accepts neither files nor any payload type".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_canvas(element: &Element, path: &str) -> Result<(), TreeError> {
    let crate::Props::Canvas { size, scene, .. } = element.props() else {
        return Ok(());
    };
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
    {
        return Err(TreeError::InvalidCanvasScene {
            path: path.to_owned(),
            reason: format!(
                "canvas size must be finite and positive, received {} x {}",
                size.width, size.height
            ),
        });
    }
    if let Some(reason) = scene.invalid_reason() {
        return Err(TreeError::InvalidCanvasScene {
            path: path.to_owned(),
            reason,
        });
    }
    Ok(())
}

fn validate_table_schema(element: &Element, path: &str) -> Result<(), TreeError> {
    let crate::Props::List {
        pattern, columns, ..
    } = element.props()
    else {
        return Ok(());
    };
    if !pattern.presents_columns() {
        if !columns.is_empty() {
            return Err(TreeError::InvalidTableSchema {
                path: path.to_owned(),
                reason: "columns require table presentation".to_owned(),
            });
        }
        return validate_non_table_rows(element.children(), path, *pattern);
    }
    let mut identifiers = HashSet::new();
    let mut active_sort_count = 0;
    for column in columns {
        if column.id.is_empty() || !identifiers.insert(column.id.as_str()) {
            return Err(TreeError::InvalidTableSchema {
                path: path.to_owned(),
                reason: format!("column identifier '{}' is empty or duplicated", column.id),
            });
        }
        if column.sort_direction.is_some() {
            active_sort_count += 1;
            if !column.sortable {
                return Err(TreeError::InvalidTableSchema {
                    path: path.to_owned(),
                    reason: format!("active sort column '{}' is not sortable", column.id),
                });
            }
        }
    }
    if active_sort_count > 1 {
        return Err(TreeError::InvalidTableSchema {
            path: path.to_owned(),
            reason: "only one active sort column is supported".to_owned(),
        });
    }
    let expected = columns.len().saturating_sub(1);
    validate_table_rows(element.children(), path, expected)
}

fn validate_table_rows(rows: &[Element], path: &str, expected: usize) -> Result<(), TreeError> {
    for (index, child) in rows.iter().enumerate() {
        let row_path = format!("{path}/{index}");
        let crate::Props::ListRow { cells, .. } = child.props() else {
            continue;
        };
        if cells.len() != expected {
            return Err(TreeError::InvalidTableSchema {
                path: row_path.clone(),
                reason: format!(
                    "row provides {} secondary cells for {expected} secondary columns",
                    cells.len()
                ),
            });
        }
        let crate::Props::ListRow { role, .. } = child.props() else {
            continue;
        };
        if *role != crate::ListRowRole::Item {
            return Err(TreeError::InvalidTableSchema {
                path: row_path.clone(),
                reason: "table rows cannot declare section semantics".to_owned(),
            });
        }
        validate_table_rows(child.children(), &row_path, expected)?;
    }
    Ok(())
}

fn validate_non_table_rows(
    rows: &[Element],
    path: &str,
    pattern: crate::CollectionPattern,
) -> Result<(), TreeError> {
    for (index, child) in rows.iter().enumerate() {
        let row_path = format!("{path}/{index}");
        let crate::Props::ListRow {
            cells,
            role,
            expanded,
            ..
        } = child.props()
        else {
            continue;
        };
        if !cells.is_empty() {
            return Err(TreeError::InvalidTableSchema {
                path: row_path,
                reason: "secondary cells require table presentation".to_owned(),
            });
        }
        if !pattern.supports_hierarchy() {
            if !child.children().is_empty() || *role != crate::ListRowRole::Item || *expanded {
                return Err(TreeError::InvalidListSchema {
                    path: row_path,
                    reason:
                        "hierarchy, sections, and expansion require hierarchical collection pattern"
                            .to_owned(),
                });
            }
            continue;
        }
        validate_non_table_rows(child.children(), &row_path, pattern)?;
    }
    Ok(())
}
