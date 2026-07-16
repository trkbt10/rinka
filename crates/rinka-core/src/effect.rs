//! Platform effects requested by component state transitions.
//!
//! [`crate::Component::update`] returns an [`Effects`] value describing what
//! the platform should do besides re-rendering — today, presenting a
//! window-modal dialog whose result arrives as an ordinary message. The
//! runtime erases the message type into [`DialogRequest`] values after the
//! update's render settles, so an effect can never mutate the tree
//! mid-reconciliation. Runtime window management is designed to extend this
//! same channel with further effect kinds.

use crate::dialog::{
    AlertDescription, DialogButton, DialogButtonRole, DialogDescription, DialogOutcome,
    DialogRequest, DialogResponder, OpenPanelDescription, SavePanelDescription,
};
use crate::runtime::Dispatch;
use std::fmt;

/// One platform effect carrying its typed result mapping.
pub enum Effect<M> {
    /// Present a window-modal dialog; the mapped message delivers the outcome.
    PresentDialog {
        /// Platform-neutral dialog description.
        description: DialogDescription,
        /// Maps the dialog outcome onto a component message, or none to
        /// drop the outcome without a state transition.
        on_outcome: Box<dyn FnOnce(DialogOutcome) -> Option<M>>,
    },
}

impl<M> fmt::Debug for Effect<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PresentDialog { description, .. } => formatter
                .debug_struct("Effect::PresentDialog")
                .field("description", description)
                .finish_non_exhaustive(),
        }
    }
}

/// Platform effects returned by one [`crate::Component::update`] call.
#[derive(Debug)]
pub struct Effects<M> {
    requests: Vec<Effect<M>>,
}

impl<M> Default for Effects<M> {
    fn default() -> Self {
        Self::none()
    }
}

impl<M> Effects<M> {
    /// Returns the effect-free update result.
    pub fn none() -> Self {
        Self {
            requests: Vec::new(),
        }
    }

    /// Returns whether this update requested no effects.
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Appends another update result's effects in order.
    pub fn then(mut self, other: Self) -> Self {
        self.requests.extend(other.requests);
        self
    }
}

impl<M: 'static> Effects<M> {
    /// Requests one window-modal dialog with an explicit outcome mapping.
    pub fn dialog(
        description: DialogDescription,
        on_outcome: impl FnOnce(DialogOutcome) -> Option<M> + 'static,
    ) -> Self {
        Self {
            requests: vec![Effect::PresentDialog {
                description,
                on_outcome: Box::new(on_outcome),
            }],
        }
    }

    /// Requests one alert whose buttons carry their own messages.
    pub fn alert(alert: Alert<M>) -> Self {
        let Alert {
            title,
            body,
            buttons,
            messages,
            default_button,
        } = alert;
        Self::dialog(
            DialogDescription::Alert(AlertDescription {
                title,
                body,
                buttons,
                default_button,
            }),
            move |outcome| match outcome {
                DialogOutcome::ButtonChosen(index) => messages.into_iter().nth(index),
                DialogOutcome::PathsChosen(_)
                | DialogOutcome::SavePathChosen(_)
                | DialogOutcome::Cancelled => None,
            },
        )
    }

    /// Requests one file-open panel.
    pub fn open_panel(
        description: OpenPanelDescription,
        on_outcome: impl FnOnce(DialogOutcome) -> Option<M> + 'static,
    ) -> Self {
        Self::dialog(DialogDescription::OpenPanel(description), on_outcome)
    }

    /// Requests one file-save panel.
    pub fn save_panel(
        description: SavePanelDescription,
        on_outcome: impl FnOnce(DialogOutcome) -> Option<M> + 'static,
    ) -> Self {
        Self::dialog(DialogDescription::SavePanel(description), on_outcome)
    }

    /// Erases the message type, binding each outcome to the dispatch loop.
    pub(crate) fn erase(self, dispatch: &Dispatch<M>) -> Vec<DialogRequest> {
        self.requests
            .into_iter()
            .map(|effect| match effect {
                Effect::PresentDialog {
                    description,
                    on_outcome,
                } => {
                    let dispatch = dispatch.clone();
                    DialogRequest::new(
                        description,
                        DialogResponder::new(move |outcome| {
                            if let Some(message) = on_outcome(outcome) {
                                dispatch.emit(message);
                            }
                        }),
                    )
                }
            })
            .collect()
    }
}

/// Typed alert builder pairing each button with the message it delivers.
#[derive(Debug)]
pub struct Alert<M> {
    title: String,
    body: String,
    buttons: Vec<DialogButton>,
    messages: Vec<M>,
    default_button: Option<usize>,
}

impl<M> Alert<M> {
    /// Creates an alert with a title and supporting body.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            buttons: Vec::new(),
            messages: Vec::new(),
            default_button: None,
        }
    }

    /// Appends a button delivering `message` when chosen.
    pub fn button(mut self, label: impl Into<String>, role: DialogButtonRole, message: M) -> Self {
        self.buttons.push(DialogButton::new(label, role));
        self.messages.push(message);
        self
    }

    /// Declares which button index receives the platform return-key default.
    ///
    /// Without this declaration no button receives the return key; core
    /// validation rejects an index naming a destructive button.
    pub fn default_button(mut self, index: usize) -> Self {
        self.default_button = Some(index);
        self
    }
}
