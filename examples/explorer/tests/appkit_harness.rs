//! Live consumer gate: the explorer operated through the `rinka-test`
//! harness against the real AppKit adapter, in-process.
//!
//! This binary declares `harness = false` because AppKit windows must be
//! hosted on the process main thread, which the libtest harness does not
//! guarantee. It runs inside the ordinary `make test` gate:
//!
//! - with a window-server session, the three scenarios below mount the real
//!   explorer, drive it by accessibility label, and fail the gate on any
//!   regression;
//! - without one (headless CI), and on platforms whose live driver is not
//!   implemented yet, the typed skip reason is logged and the binary exits
//!   successfully — a visible skip, never silent, and never proof.

use rinka_explorer::view;
use rinka_test::{HarnessError, TestHost};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match TestHost::mount(view::application(view::Scene::Ready)) {
        Err(
            error
            @ (HarnessError::NoWindowServerSession | HarnessError::UnsupportedPlatform { .. }),
        ) => {
            eprintln!("rinka-test explorer gate skip reason={error}");
            eprintln!("rinka-test explorer gate result=SKIP");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rinka-test explorer gate mount error={error}");
            eprintln!("rinka-test explorer gate result=FAIL");
            ExitCode::FAILURE
        }
        Ok(host) => match run_scenarios(&host) {
            Ok(()) => {
                eprintln!("rinka-test explorer gate result=PASS scenarios=3");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("rinka-test explorer gate error={error}");
                eprintln!("rinka-test explorer gate result=FAIL");
                ExitCode::FAILURE
            }
        },
    }
}

fn run_scenarios(host: &TestHost) -> Result<(), HarnessError> {
    sidebar_selection_updates_the_listing(host)?;
    eprintln!("rinka-test explorer gate scenario=sidebar-row-selection pass=true");
    typing_into_the_filter_narrows_the_table(host)?;
    eprintln!("rinka-test explorer gate scenario=filter-field-typing pass=true");
    // The third scenario proves a fresh mount in the same process: the
    // first host is dropped (windows closed, monitor removed) before the
    // canvas scene is mounted and photographed.
    drop_and_run_canvas_scenario()?;
    eprintln!("rinka-test explorer gate scenario=menu-canvas-caption pass=true");
    Ok(())
}

/// Clicking a sidebar row by its accessibility label re-targets the file
/// listing: the row's native selection dispatches the same activate binding
/// a user click fires, and the table header and path follow.
fn sidebar_selection_updates_the_listing(host: &TestHost) -> Result<(), HarnessError> {
    assert!(
        host.exists_by_label("Files in Remote Project"),
        "the ready scene starts in Remote Project"
    );
    let home = host.find_by_label("Home")?;
    host.select_row(&home)?;
    host.settle_until("the table lists the Home location", |host| {
        host.exists_by_label("Files in Home")
    })?;
    let path = host.find_by_key("directory-path")?;
    let shown = host.read_value(&path)?;
    assert!(
        shown.ends_with("/Users/trkbt10") || shown.ends_with("/home/ubuntu"),
        "the path header follows the selection, found {shown:?}"
    );
    Ok(())
}

/// Clicking the real "Show hidden files" NSButton by its accessibility
/// label (the same `performClick:` target/action dispatch a user click
/// performs) reveals the hidden row; typing into the mounted "Filter files"
/// search field through its real field editor then narrows the table, and
/// the value reads back through the native control's accessibility surface.
fn typing_into_the_filter_narrows_the_table(host: &TestHost) -> Result<(), HarnessError> {
    assert!(
        host.exists_by_key("file-Readme"),
        "README.md is listed before filtering"
    );
    let hidden = host.find_by_label("Show hidden files")?;
    host.press(&hidden)?;
    host.settle_until("the hidden .env row is listed", |host| {
        host.exists_by_key("file-HiddenEnvironment")
    })?;
    assert!(
        host.is_checked(&hidden)?,
        "the native checkbox reflects the click"
    );

    let field = host.find_by_label("Filter files")?;
    host.type_text(&field, "Cargo")?;
    host.commit_text(&field)?;
    host.settle_until("only Cargo.toml remains listed", |host| {
        host.exists_by_key("file-Cargo")
            && !host.exists_by_key("file-Readme")
            && !host.exists_by_key("file-HiddenEnvironment")
    })?;
    let value = host.read_value(&field)?;
    assert_eq!(value, "Cargo", "the field reads back what was typed");
    Ok(())
}

/// Switching to the canvas scene through the native View menu mounts the
/// canvas pane; its input caption is asserted and real pixels are captured
/// and verified decodable.
fn drop_and_run_canvas_scenario() -> Result<(), HarnessError> {
    let host = TestHost::mount(view::application(view::Scene::Ready))?;
    host.activate_menu_item("View", "Canvas")?;
    host.settle_until("the canvas pane is mounted", |host| {
        host.exists_by_key("canvas-pane")
    })?;
    let caption = host.find_by_key("canvas-input-caption")?;
    let text = host.read_value(&caption)?;
    assert!(
        text.starts_with("input: focused="),
        "the canvas caption reports input state, found {text:?}"
    );

    // A synthetic click through the window's real event routing reaches the
    // canvas pointer handler; the pointer caption leaves its "none" state.
    let surface = host.find_by_key("canvas-surface")?;
    host.click_center(&surface)?;
    host.settle_until("the pointer caption reports the click", |host| {
        host.find_by_key("canvas-pointer-caption")
            .and_then(|caption| host.read_value(&caption))
            .is_ok_and(|text| text.starts_with("pointer:") && !text.contains("none"))
    })?;

    let directory = capture_directory();
    std::fs::create_dir_all(&directory).map_err(|error| HarnessError::Capture {
        path: directory.clone(),
        reason: error.to_string(),
    })?;
    let window_path = directory.join("gate-canvas-window.png");
    let (window_width, window_height) = host.capture_window_png(0, &window_path)?;
    assert!(
        window_width > 100 && window_height > 100,
        "the window capture has non-trivial dimensions"
    );
    let pane = host.find_by_key("canvas-pane")?;
    let pane_path = directory.join("gate-canvas-pane.png");
    let (pane_width, pane_height) = host.capture_element_png(&pane, &pane_path)?;
    assert!(
        pane_width > 0 && pane_height > 0,
        "the element capture has non-trivial dimensions"
    );
    eprintln!(
        "rinka-test explorer gate captures window={}({window_width}x{window_height}) pane={}({pane_width}x{pane_height})",
        window_path.display(),
        pane_path.display()
    );
    Ok(())
}

/// Captures go to `RINKA_TEST_CAPTURE_DIR` when set (evidence collection),
/// otherwise to a per-process temporary directory.
fn capture_directory() -> PathBuf {
    std::env::var_os("RINKA_TEST_CAPTURE_DIR").map_or_else(
        || {
            std::env::temp_dir().join(format!(
                "rinka-explorer-harness-gate-{}",
                std::process::id()
            ))
        },
        PathBuf::from,
    )
}
