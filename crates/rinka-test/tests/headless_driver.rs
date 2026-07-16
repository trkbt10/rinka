//! Deterministic semantics of the driver verbs over the headless backend.
//!
//! These tests run with no window server on every platform: they prove the
//! find/act/settle contract — locator resolution, typed not-found and
//! wrong-role diagnostics, event dispatch through the stable bindings, and
//! the named settlement timeout — against `rinka-headless`.

use rinka_core::{
    Component, Dispatch, Element, InputKind, UpdateContext, WindowContent, button, column, input,
    label, list, list_row, toggle,
};
use rinka_test::{HarnessError, HeadlessHost};

struct FixtureComponent {
    count: usize,
    filter: String,
    dark: bool,
}

enum FixtureMessage {
    Increment,
    SetFilter(String),
    SetDark(bool),
}

impl Component for FixtureComponent {
    type Message = FixtureMessage;

    fn update(&mut self, message: Self::Message, _context: &UpdateContext<Self::Message>) {
        match message {
            FixtureMessage::Increment => self.count += 1,
            FixtureMessage::SetFilter(filter) => self.filter = filter,
            FixtureMessage::SetDark(dark) => self.dark = dark,
        }
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let increment = dispatch.clone();
        let filter = dispatch.clone();
        let rows = ["Alpha", "Beta", "Gamma"]
            .into_iter()
            .filter(|title| title.contains(self.filter.as_str()))
            .map(|title| {
                list_row(title, None, None, false, false, title, || {})
                    .with_key(format!("row-{}", title.to_lowercase()))
            });
        column([
            label(format!("count: {}", self.count)).with_key("count-label"),
            button("Increment", "Increment the counter", move || {
                increment.emit(FixtureMessage::Increment);
            })
            .with_key("increment"),
            input(
                self.filter.clone(),
                "Filter",
                InputKind::Search,
                "Filter rows",
                move |value| filter.emit(FixtureMessage::SetFilter(value)),
            )
            .with_key("filter"),
            toggle("Dark", self.dark, "Dark appearance", move |value| {
                dispatch.emit(FixtureMessage::SetDark(value));
            })
            .with_key("dark"),
            list("Rows", rows).with_key("rows"),
        ])
        .with_key("fixture-root")
    }
}

fn mounted_fixture() -> HeadlessHost {
    HeadlessHost::mount(WindowContent::component(FixtureComponent {
        count: 0,
        filter: String::new(),
        dark: false,
    }))
    .expect("the fixture mounts")
}

#[test]
fn pressing_a_button_found_by_accessibility_label_updates_state() {
    let host = mounted_fixture();
    let button = host
        .find_by_label("Increment the counter")
        .expect("the button is found by its accessibility label");

    host.press(&button).expect("press dispatches");
    host.press(&button).expect("press dispatches again");
    host.settle().expect("no render error is pending");

    let count = host.find_by_key("count-label").expect("the count label");
    assert_eq!(host.read_value(&count).expect("label text"), "count: 2");
}

#[test]
fn typing_into_an_input_found_by_label_filters_the_rows() {
    let host = mounted_fixture();
    assert!(host.exists_by_key("row-alpha"));
    assert!(host.exists_by_key("row-beta"));

    let field = host
        .find_by_label("Filter rows")
        .expect("the input is found by its accessibility label");
    host.type_text(&field, "Bet").expect("typing dispatches");

    host.settle_until("only the Beta row remains", |host| {
        host.exists_by_key("row-beta") && !host.exists_by_key("row-alpha")
    })
    .expect("the filter reaches the mounted tree");
    assert_eq!(host.read_value(&field).expect("input value"), "Bet");
}

#[test]
fn toggling_updates_the_controlled_value() {
    let host = mounted_fixture();
    let dark = host.find_by_label("Dark appearance").expect("the toggle");
    assert!(!host.is_checked(&dark).expect("initial value"));
    assert!(host.is_enabled(&dark).expect("enabled state"));

    host.toggle(&dark, true).expect("toggle dispatches");

    assert!(host.is_checked(&dark).expect("updated value"));
}

#[test]
fn a_missing_element_is_a_typed_not_found_naming_the_locator() {
    let host = mounted_fixture();
    let error = host
        .find_by_label("No such element")
        .expect_err("nothing matches");
    assert!(matches!(error, HarnessError::NotFound { .. }));
    assert_eq!(
        error.to_string(),
        "no mounted element matches accessibility label 'No such element'"
    );
}

#[test]
fn a_verb_on_the_wrong_role_is_a_typed_diagnostic() {
    let host = mounted_fixture();
    let label = host.find_by_key("count-label").expect("the label");
    let error = host.press(&label).expect_err("labels cannot be pressed");
    assert!(matches!(
        error,
        HarnessError::WrongRole {
            verb: "press",
            found: rinka_core::ElementKind::Label,
            ..
        }
    ));
}

#[test]
fn a_settlement_timeout_names_the_unmet_condition() {
    let host = mounted_fixture();
    let error = host
        .settle_until("the moon turns blue", |_| false)
        .expect_err("the condition never holds");
    let HarnessError::SettlementTimeout { turns, unmet } = error else {
        panic!("expected a settlement timeout, received {error}");
    };
    assert!(turns > 0);
    assert_eq!(unmet, vec!["the moon turns blue".to_owned()]);
}

#[test]
fn the_tree_snapshot_names_kinds_keys_and_accessibility_names() {
    let host = mounted_fixture();
    let snapshot = host.tree_snapshot().expect("a mounted root");
    assert!(snapshot.contains("Button key=increment name=\"Increment the counter\""));
    assert!(snapshot.contains("Input key=filter name=\"Filter rows\""));
    assert!(snapshot.contains("ListRow key=row-alpha name=\"Alpha\""));
}
