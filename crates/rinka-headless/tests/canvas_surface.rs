//! Consumer-level contracts of the owned-drawing canvas element.

use rinka_core::{
    AppRuntime, CanvasColor, CanvasPoint, CanvasRect, CanvasSize, CanvasVector, Component,
    Dispatch, DrawCommand, DrawScene, Element, LineWidth, NativeBackend, PlatformServices,
    PointerButton, PointerEvent, PointerModifiers, PointerPhase, Props, Renderer, UpdateContext,
    canvas, column, label,
};
use rinka_headless::{HeadlessBackend, Operation};
use std::cell::RefCell;
use std::rc::Rc;

fn meter_scene(level: f64) -> DrawScene {
    let mut scene = DrawScene::new();
    scene.fill_rect(
        CanvasRect::new(0.0, 0.0, 120.0, 16.0),
        CanvasColor::rgb(0.1, 0.1, 0.1),
    );
    scene.fill_rect(
        CanvasRect::new(0.0, 0.0, 120.0 * level, 16.0),
        CanvasColor::rgb(0.2, 0.8, 0.3),
    );
    scene
}

fn meter(level: f64) -> Element {
    canvas(
        CanvasSize::new(120.0, 16.0),
        meter_scene(level),
        "Output level meter",
    )
    .with_key("meter")
}

#[test]
fn canvas_mounts_with_its_recorded_scene() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(column([meter(0.25), label("Output").with_key("caption")]).with_key("panel"))
        .unwrap();

    let handle = renderer.backend().find_by_key("meter").unwrap();
    let Some(Props::Canvas { size, scene, .. }) = renderer.backend().props_of(handle) else {
        panic!("mounted canvas must retain canvas properties");
    };
    assert_eq!(*size, CanvasSize::new(120.0, 16.0));
    assert_eq!(scene, &meter_scene(0.25));
    assert!(matches!(
        scene.commands()[1],
        DrawCommand::FillRect { rect, .. } if rect.size.width == 30.0
    ));
}

#[test]
fn scene_change_patches_without_replacing_native_identity() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer.render(meter(0.25)).unwrap();
    let before = renderer.backend().find_by_key("meter").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer.render(meter(0.75)).unwrap();

    assert_eq!(renderer.backend().find_by_key("meter"), Some(before));
    assert_eq!(stats.patched, 1);
    assert_eq!(stats.created, 0);
    assert_eq!(stats.replaced, 0);
    assert!(renderer.backend().operations().iter().any(
        |operation| matches!(operation, Operation::Patch { handle, .. } if *handle == before)
    ));
}

#[test]
fn an_unchanged_scene_issues_no_native_mutation() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer.render(meter(0.5)).unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer.render(meter(0.5)).unwrap();

    assert_eq!(stats.patched, 0);
    assert_eq!(stats.created, 0);
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn keyed_reorder_moves_the_canvas_without_recreating_it() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(column([meter(0.5), label("Output").with_key("caption")]).with_key("panel"))
        .unwrap();
    let meter_handle = renderer.backend().find_by_key("meter").unwrap();
    let panel = renderer.backend().find_by_key("panel").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer
        .render(column([label("Output").with_key("caption"), meter(0.5)]).with_key("panel"))
        .unwrap();

    let caption = renderer.backend().find_by_key("caption").unwrap();
    assert_eq!(
        renderer.backend().children_of(panel),
        Some([caption, meter_handle].as_slice())
    );
    assert_eq!(stats.moved, 1);
    assert_eq!(stats.created, 0);
}

#[test]
fn unmounting_destroys_the_canvas() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(column([meter(0.5), label("Output").with_key("caption")]).with_key("panel"))
        .unwrap();
    let meter_handle = renderer.backend().find_by_key("meter").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer
        .render(column([label("Output").with_key("caption")]).with_key("panel"))
        .unwrap();

    assert_eq!(stats.removed, 1);
    assert_eq!(renderer.backend().find_by_key("meter"), None);
    assert!(renderer.backend().operations().iter().any(
        |operation| matches!(operation, Operation::Destroy { handle } if *handle == meter_handle)
    ));
}

#[test]
fn invalid_scenes_are_rejected_before_native_mutation() {
    let mut renderer = Renderer::new(HeadlessBackend::new());

    let mut unbalanced = DrawScene::new();
    unbalanced.push_clip(CanvasRect::new(0.0, 0.0, 8.0, 8.0));
    let error = renderer
        .render(canvas(CanvasSize::new(8.0, 8.0), unbalanced, "Meter"))
        .unwrap_err();
    assert!(error.to_string().contains("invalid canvas"));
    assert!(error.to_string().contains("without a matching pop"));
    assert!(renderer.backend().operations().is_empty());

    let error = renderer
        .render(canvas(CanvasSize::new(0.0, 8.0), DrawScene::new(), "Meter"))
        .unwrap_err();
    assert!(error.to_string().contains("canvas size"));
    assert!(renderer.backend().operations().is_empty());

    let mut non_finite = DrawScene::new();
    non_finite.line(
        CanvasPoint::new(0.0, 0.0),
        CanvasPoint::new(f64::INFINITY, 4.0),
        LineWidth::Points(1.0),
        CanvasColor::rgb(0.0, 0.0, 0.0),
    );
    let error = renderer
        .render(canvas(CanvasSize::new(8.0, 8.0), non_finite, "Meter"))
        .unwrap_err();
    assert!(error.to_string().contains("finite"));
    assert!(renderer.backend().operations().is_empty());
}

fn gauge_view(level: f64) -> Element {
    column([meter(level), label("Output").with_key("caption")]).with_key("panel")
}

#[test]
fn many_scene_changes_within_one_frame_coalesce_into_one_patch() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let mut level = 0.0_f64;
    renderer.render(gauge_view(level)).unwrap();
    let meter_handle = renderer.backend().find_by_key("meter").unwrap();
    renderer.backend_mut().clear_operations();

    // Many dirty marks land between two frames: every one mutates the state
    // feeding the scene, and the next render folds them into a single
    // rebuilt scene.
    for _ in 0..16 {
        level = (level + 0.05).clamp(0.0, 1.0);
    }
    let stats = renderer.render(gauge_view(level)).unwrap();

    // One native property patch for the canvas, nothing else: no created or
    // replaced native objects, and no patch on any sibling.
    assert_eq!(stats.patched, 1);
    assert_eq!(stats.created, 0);
    assert_eq!(stats.replaced, 0);
    assert_eq!(stats.removed, 0);
    let patches: Vec<_> = renderer
        .backend()
        .operations()
        .iter()
        .filter_map(|operation| match operation {
            Operation::Patch { handle, .. } => Some(*handle),
            _ => None,
        })
        .collect();
    assert_eq!(patches, [meter_handle]);
    let mutations = renderer
        .backend()
        .operations()
        .iter()
        .filter(|operation| !matches!(operation, Operation::Patch { .. }))
        .count();
    assert_eq!(mutations, 0);
}

struct CrosshairComponent {
    pointer: Option<PointerEvent>,
}

impl Component for CrosshairComponent {
    type Message = PointerEvent;

    fn update(&mut self, message: Self::Message, _context: &UpdateContext<Self::Message>) {
        self.pointer = Some(message);
    }

    fn view(&self, dispatch: Dispatch<Self::Message>) -> Element {
        let mut scene = DrawScene::new();
        if let Some(event) = self.pointer {
            scene.line(
                CanvasPoint::new(event.position.x, 0.0),
                CanvasPoint::new(event.position.x, 64.0),
                LineWidth::Hairline,
                CanvasColor::rgb(1.0, 0.0, 0.0),
            );
        }
        canvas(CanvasSize::new(64.0, 64.0), scene, "Pointer crosshair")
            .on_pointer(move |event| dispatch.emit(event))
            .with_key("crosshair")
    }
}

#[test]
fn pointer_events_round_trip_element_local_coordinates_into_messages() {
    let runtime = AppRuntime::mount(
        Renderer::new(HeadlessBackend::new()),
        CrosshairComponent { pointer: None },
        PlatformServices::default(),
    )
    .unwrap();
    let handle = runtime
        .with_renderer(|renderer| renderer.backend().find_by_key("crosshair"))
        .unwrap();
    let events = runtime
        .with_renderer(|renderer| renderer.backend().events_of(handle))
        .unwrap();

    let event = PointerEvent {
        phase: PointerPhase::Down,
        position: CanvasPoint::new(12.5, 40.0),
        button: PointerButton::Primary,
        modifiers: PointerModifiers {
            shift: true,
            ..PointerModifiers::default()
        },
        scroll_delta: CanvasVector::default(),
    };
    events.emit_pointer(event);

    assert_eq!(runtime.with_component(|state| state.pointer), Some(event));
    runtime.with_renderer(|renderer| {
        assert_eq!(renderer.backend().find_by_key("crosshair"), Some(handle));
        let Some(Props::Canvas { scene, .. }) = renderer.backend().props_of(handle) else {
            panic!("crosshair canvas must retain canvas properties");
        };
        assert!(matches!(
            scene.commands()[0],
            DrawCommand::Line { from, .. } if from.x == 12.5
        ));
    });

    let scroll = PointerEvent {
        phase: PointerPhase::Scroll,
        position: CanvasPoint::new(2.0, 3.0),
        button: PointerButton::None,
        modifiers: PointerModifiers::default(),
        scroll_delta: CanvasVector::new(0.0, -24.0),
    };
    events.emit_pointer(scroll);
    assert_eq!(runtime.with_component(|state| state.pointer), Some(scroll));
}

#[test]
fn pointer_handler_is_replaced_without_reconnecting_native_identity() {
    let observed = Rc::new(RefCell::new(Vec::<PointerPhase>::new()));
    let build = |observed: Rc<RefCell<Vec<PointerPhase>>>| {
        canvas(
            CanvasSize::new(32.0, 32.0),
            DrawScene::new(),
            "Pointer target",
        )
        .on_pointer(move |event| observed.borrow_mut().push(event.phase))
        .with_key("target")
    };
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer.render(build(observed.clone())).unwrap();
    let handle = renderer.backend().find_by_key("target").unwrap();
    let events = renderer.backend().events_of(handle).unwrap();

    renderer.render(build(observed.clone())).unwrap();
    events.emit_pointer(PointerEvent {
        phase: PointerPhase::Move,
        position: CanvasPoint::new(1.0, 1.0),
        button: PointerButton::None,
        modifiers: PointerModifiers::default(),
        scroll_delta: CanvasVector::default(),
    });

    assert_eq!(observed.borrow().as_slice(), [PointerPhase::Move]);
    assert_eq!(renderer.backend().find_by_key("target"), Some(handle));
}

#[test]
fn monospace_metrics_lay_out_a_terminal_cell_grid() {
    let backend = HeadlessBackend::new();
    let metrics = backend
        .monospace_metrics(13.0)
        .expect("headless adapter measures its synthetic monospace font");

    assert!(metrics.row_height.is_finite() && metrics.row_height >= 13.0);
    assert!(metrics.glyph_width.is_finite() && metrics.glyph_width > 0.0);
    assert_eq!(backend.monospace_metrics(13.0), Some(metrics));
    assert_eq!(backend.monospace_metrics(f64::NAN), None);
    assert_eq!(backend.monospace_metrics(0.0), None);

    // An 80 x 24 terminal grid: every cell origin advances by exactly one
    // glyph width and one row height, so the grid tiles without drift.
    let (columns, rows) = (80_usize, 24_usize);
    let cell = |row: usize, column: usize| {
        CanvasPoint::new(
            column as f64 * metrics.glyph_width,
            row as f64 * metrics.row_height,
        )
    };
    let last = cell(rows - 1, columns - 1);
    assert_eq!(last.x, 79.0 * metrics.glyph_width);
    assert_eq!(last.y, 23.0 * metrics.row_height);
    let mut scene = DrawScene::new();
    for row in 0..rows {
        let origin = cell(row, 0);
        scene.glyph_run(
            origin,
            "x".repeat(columns),
            13.0,
            CanvasColor::rgb(0.9, 0.9, 0.9),
        );
    }
    assert_eq!(scene.commands().len(), rows);
    let grid_size = CanvasSize::new(
        columns as f64 * metrics.glyph_width,
        rows as f64 * metrics.row_height,
    );
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(canvas(grid_size, scene, "Terminal grid").with_key("grid"))
        .unwrap();
    assert!(renderer.backend().find_by_key("grid").is_some());
}
