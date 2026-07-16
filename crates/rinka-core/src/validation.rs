//! Declarative tree invariants checked before native mutation.

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
    /// A mounted native window attempted to replace its promoted root class.
    WindowRootKindChanged {
        /// Root kind retained by the native window.
        previous: ElementKind,
        /// Root kind requested by the next component view.
        next: ElementKind,
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
            Self::WindowRootKindChanged { previous, next } => write!(
                formatter,
                "native window root kind must remain stable: mounted {previous:?}, received {next:?}"
            ),
        }
    }
}

impl Error for TreeError {}

pub(crate) fn validate_tree(root: &Element) -> Result<(), TreeError> {
    validate_node(root, "root")
}

fn validate_node(element: &Element, path: &str) -> Result<(), TreeError> {
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
        | crate::ElementKind::Toggle
        | crate::ElementKind::Progress
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
