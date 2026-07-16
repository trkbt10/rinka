//! Window-modal dialog descriptions, outcomes, and typed validation.
//!
//! A dialog is declared as data, requested from [`crate::Component::update`]
//! through [`crate::UpdateContext::dialogs`], presented window-modally by the
//! host's injected [`DialogService`], and answered with an ordinary component
//! message delivered through the queued dispatch path. The core never draws a
//! dialog; it validates the description and hands a type-erased
//! [`DialogRequest`] to the service.

use crate::runtime::Dispatch;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;

/// Semantic treatment of one alert button, translated per platform.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DialogButtonRole {
    /// Normal choice without additional key behavior.
    Standard,
    /// Dismissing choice; receives the platform escape behavior.
    Cancel,
    /// Choice with destructive consequences; receives the platform
    /// destructive treatment and must never be the return-key default.
    Destructive,
}

/// One button in an alert description.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogButton {
    /// Visible title.
    pub label: String,
    /// Semantic treatment.
    pub role: DialogButtonRole,
}

impl DialogButton {
    /// Creates a button description.
    pub fn new(label: impl Into<String>, role: DialogButtonRole) -> Self {
        Self {
            label: label.into(),
            role,
        }
    }
}

/// Alert or confirmation presented as a window-modal sheet.
///
/// The return-key default is explicit: only the button named by
/// `default_button` may receive the platform return-key equivalent, and
/// adapters must clear any implicit platform default (such as AppKit's
/// first-button return key) so an absent index means no return default at
/// all. [`DialogDescription::validity_error`] rejects a description whose
/// default is a destructive button.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlertDescription {
    /// Primary message.
    pub title: String,
    /// Supporting explanation.
    pub body: String,
    /// Choices in declaration order.
    pub buttons: Vec<DialogButton>,
    /// Index of the button receiving the return-key default, if any.
    pub default_button: Option<usize>,
}

/// File-open panel options presented as a window-modal sheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenPanelDescription {
    /// Explanatory prompt shown by the panel, if the platform displays one.
    pub title: Option<String>,
    /// Whether existing files can be chosen.
    pub choose_files: bool,
    /// Whether directories can be chosen.
    pub choose_directories: bool,
    /// Whether more than one item can be chosen.
    pub allows_multiple: bool,
    /// Directory the panel initially displays.
    pub starting_directory: Option<PathBuf>,
}

/// File-save panel options presented as a window-modal sheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavePanelDescription {
    /// Explanatory prompt shown by the panel, if the platform displays one.
    pub title: Option<String>,
    /// Initial destination file name.
    pub suggested_filename: Option<String>,
    /// Directory the panel initially displays.
    pub starting_directory: Option<PathBuf>,
}

/// Complete window-modal dialog description.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogDescription {
    /// Alert or confirmation with role-carrying buttons.
    Alert(AlertDescription),
    /// File-open panel returning chosen paths.
    OpenPanel(OpenPanelDescription),
    /// File-save panel returning one destination path.
    SavePanel(SavePanelDescription),
}

impl DialogDescription {
    /// Returns the typed violation that makes this description unpresentable.
    pub fn validity_error(&self) -> Option<DialogError> {
        match self {
            Self::Alert(alert) => alert_validity_error(alert),
            Self::OpenPanel(panel) => (!panel.choose_files && !panel.choose_directories)
                .then_some(DialogError::OpenPanelChoosesNothing),
            Self::SavePanel(_) => None,
        }
    }
}

fn alert_validity_error(alert: &AlertDescription) -> Option<DialogError> {
    if alert.buttons.is_empty() {
        return Some(DialogError::NoButtons);
    }
    if let Some(index) = alert.default_button {
        let Some(button) = alert.buttons.get(index) else {
            return Some(DialogError::DefaultButtonOutOfRange {
                index,
                buttons: alert.buttons.len(),
            });
        };
        if button.role == DialogButtonRole::Destructive {
            return Some(DialogError::DestructiveDefault {
                index,
                label: button.label.clone(),
            });
        }
    }
    let cancel_buttons = alert
        .buttons
        .iter()
        .filter(|button| button.role == DialogButtonRole::Cancel)
        .count();
    (cancel_buttons > 1).then_some(DialogError::MultipleCancelButtons)
}

/// Result delivered when a presented dialog completes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogOutcome {
    /// The alert button at this description index was chosen.
    ButtonChosen(usize),
    /// The open panel confirmed these paths.
    PathsChosen(Vec<PathBuf>),
    /// The save panel confirmed this destination path.
    SavePathChosen(PathBuf),
    /// The dialog was dismissed without a choice.
    Cancelled,
}

/// Invalid dialog description or unpresentable dialog request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogError {
    /// An alert declared no buttons.
    NoButtons,
    /// The declared return-key default index has no button.
    DefaultButtonOutOfRange {
        /// Declared index.
        index: usize,
        /// Declared button count.
        buttons: usize,
    },
    /// A destructive button was declared as the return-key default.
    DestructiveDefault {
        /// Declared index.
        index: usize,
        /// Destructive button title.
        label: String,
    },
    /// An alert declared more than one cancel-role button.
    MultipleCancelButtons,
    /// An open panel allowed choosing neither files nor directories.
    OpenPanelChoosesNothing,
    /// The window host has not installed a dialog presenter.
    NoPresenter,
}

impl fmt::Display for DialogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoButtons => formatter.write_str("alert declares no buttons"),
            Self::DefaultButtonOutOfRange { index, buttons } => write!(
                formatter,
                "return-key default index {index} is out of range for {buttons} buttons"
            ),
            Self::DestructiveDefault { index, label } => write!(
                formatter,
                "destructive button '{label}' at index {index} must not be the return-key default"
            ),
            Self::MultipleCancelButtons => {
                formatter.write_str("alert declares more than one cancel button")
            }
            Self::OpenPanelChoosesNothing => {
                formatter.write_str("open panel must allow choosing files or directories")
            }
            Self::NoPresenter => {
                formatter.write_str("the window host has not installed a dialog presenter")
            }
        }
    }
}

impl Error for DialogError {}

/// Delivers one dialog outcome back into the requesting component.
///
/// The responder is single-use: platform completion handlers consume it when
/// the native dialog ends.
pub struct DialogResponder(Box<dyn FnOnce(DialogOutcome)>);

impl DialogResponder {
    pub(crate) fn new(deliver: impl FnOnce(DialogOutcome) + 'static) -> Self {
        Self(Box::new(deliver))
    }

    /// Delivers the outcome as a component message.
    pub fn deliver(self, outcome: DialogOutcome) {
        (self.0)(outcome);
    }
}

impl fmt::Debug for DialogResponder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DialogResponder(..)")
    }
}

/// Type-erased, validated dialog request handed to a platform presenter.
#[derive(Debug)]
pub struct DialogRequest {
    description: DialogDescription,
    responder: DialogResponder,
}

impl DialogRequest {
    pub(crate) fn new(description: DialogDescription, responder: DialogResponder) -> Self {
        Self {
            description,
            responder,
        }
    }

    /// Reads the dialog description.
    pub fn description(&self) -> &DialogDescription {
        &self.description
    }

    /// Splits the request into its description and single-use responder.
    pub fn into_parts(self) -> (DialogDescription, DialogResponder) {
        (self.description, self.responder)
    }
}

/// Host-implemented window-modal dialog presenter.
///
/// The host injects an implementation through
/// [`crate::PlatformServices::with_dialog_service`]; the request's
/// description is already validated when `present` is called.
pub trait DialogService {
    /// Presents one validated dialog request window-modally.
    fn present(&self, request: DialogRequest);
}

/// Sink recording a dialog request the runtime could not present.
pub(crate) type DialogErrorReport = Rc<dyn Fn(DialogError)>;

/// Typed dialog presentation reached through [`crate::UpdateContext`].
///
/// Every presentation is validated first — an invalid description or a host
/// without an injected [`DialogService`] is recorded as the typed
/// [`crate::RenderError::Dialog`] on the mounting runtime, never silently
/// dropped — and every outcome is delivered as an ordinary component message
/// through the queued dispatch path, so a synchronously answered dialog can
/// never re-enter the running update.
pub struct Dialogs<'a, M> {
    service: Option<&'a Rc<dyn DialogService>>,
    dispatch: &'a Dispatch<M>,
    report: &'a DialogErrorReport,
}

impl<'a, M: 'static> Dialogs<'a, M> {
    pub(crate) fn new(
        service: Option<&'a Rc<dyn DialogService>>,
        dispatch: &'a Dispatch<M>,
        report: &'a DialogErrorReport,
    ) -> Self {
        Self {
            service,
            dispatch,
            report,
        }
    }

    /// Presents one window-modal dialog with an explicit outcome mapping.
    pub fn present(
        &self,
        description: DialogDescription,
        on_outcome: impl FnOnce(DialogOutcome) -> Option<M> + 'static,
    ) {
        if let Some(error) = description.validity_error() {
            (self.report)(error);
            return;
        }
        let Some(service) = self.service else {
            (self.report)(DialogError::NoPresenter);
            return;
        };
        let dispatch = self.dispatch.clone();
        service.present(DialogRequest::new(
            description,
            DialogResponder::new(move |outcome| {
                if let Some(message) = on_outcome(outcome) {
                    dispatch.emit(message);
                }
            }),
        ));
    }

    /// Presents one alert whose buttons carry their own messages.
    pub fn alert(&self, alert: Alert<M>) {
        let Alert {
            title,
            body,
            buttons,
            messages,
            default_button,
        } = alert;
        self.present(
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
        );
    }

    /// Presents one file-open panel.
    pub fn open_panel(
        &self,
        description: OpenPanelDescription,
        on_outcome: impl FnOnce(DialogOutcome) -> Option<M> + 'static,
    ) {
        self.present(DialogDescription::OpenPanel(description), on_outcome);
    }

    /// Presents one file-save panel.
    pub fn save_panel(
        &self,
        description: SavePanelDescription,
        on_outcome: impl FnOnce(DialogOutcome) -> Option<M> + 'static,
    ) {
        self.present(DialogDescription::SavePanel(description), on_outcome);
    }
}

impl<M> fmt::Debug for Dialogs<'_, M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Dialogs")
            .field("service", &self.service.is_some())
            .finish_non_exhaustive()
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

#[cfg(test)]
mod tests {
    use super::{
        AlertDescription, DialogButton, DialogButtonRole, DialogDescription, DialogError,
        OpenPanelDescription,
    };

    fn alert(buttons: Vec<DialogButton>, default_button: Option<usize>) -> DialogDescription {
        DialogDescription::Alert(AlertDescription {
            title: "Delete file?".to_owned(),
            body: "This cannot be undone.".to_owned(),
            buttons,
            default_button,
        })
    }

    #[test]
    fn a_destructive_return_key_default_is_a_typed_violation() {
        let description = alert(
            vec![
                DialogButton::new("Delete", DialogButtonRole::Destructive),
                DialogButton::new("Cancel", DialogButtonRole::Cancel),
            ],
            Some(0),
        );
        assert_eq!(
            description.validity_error(),
            Some(DialogError::DestructiveDefault {
                index: 0,
                label: "Delete".to_owned(),
            })
        );
    }

    #[test]
    fn a_safe_default_next_to_a_destructive_button_is_valid() {
        let description = alert(
            vec![
                DialogButton::new("Delete", DialogButtonRole::Destructive),
                DialogButton::new("Cancel", DialogButtonRole::Cancel),
            ],
            Some(1),
        );
        assert_eq!(description.validity_error(), None);
    }

    #[test]
    fn an_absent_default_means_no_return_key_default_and_is_valid() {
        let description = alert(
            vec![
                DialogButton::new("Delete", DialogButtonRole::Destructive),
                DialogButton::new("Cancel", DialogButtonRole::Cancel),
            ],
            None,
        );
        assert_eq!(description.validity_error(), None);
    }

    #[test]
    fn structural_alert_violations_are_typed() {
        assert_eq!(
            alert(Vec::new(), None).validity_error(),
            Some(DialogError::NoButtons)
        );
        assert_eq!(
            alert(
                vec![DialogButton::new("OK", DialogButtonRole::Standard)],
                Some(3)
            )
            .validity_error(),
            Some(DialogError::DefaultButtonOutOfRange {
                index: 3,
                buttons: 1,
            })
        );
        assert_eq!(
            alert(
                vec![
                    DialogButton::new("Cancel", DialogButtonRole::Cancel),
                    DialogButton::new("Dismiss", DialogButtonRole::Cancel),
                ],
                None
            )
            .validity_error(),
            Some(DialogError::MultipleCancelButtons)
        );
    }

    #[test]
    fn an_open_panel_must_be_able_to_choose_something() {
        let description = DialogDescription::OpenPanel(OpenPanelDescription {
            title: None,
            choose_files: false,
            choose_directories: false,
            allows_multiple: false,
            starting_directory: None,
        });
        assert_eq!(
            description.validity_error(),
            Some(DialogError::OpenPanelChoosesNothing)
        );
    }
}
