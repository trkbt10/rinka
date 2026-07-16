// Owned-drawing canvas realized as an NSView subclass painting in drawRect:.
//
// The canvas is reserved for inherently graphical content (terminal cell
// grids, meters, widget faces). It never imitates a native control; the
// adapter replays the recorded platform-neutral scene and nothing else.

unsafe extern "C" {
    #[link_name = "NSFontAttributeName"]
    static FONT_ATTRIBUTE_NAME: *mut AnyObject;
    #[link_name = "NSForegroundColorAttributeName"]
    static FOREGROUND_COLOR_ATTRIBUTE_NAME: *mut AnyObject;
    #[link_name = "NSAccessibilityImageRole"]
    static ACCESSIBILITY_IMAGE_ROLE: *mut AnyObject;
}

/// `NSTrackingMouseMoved | NSTrackingActiveInKeyWindow | NSTrackingInVisibleRect`.
/// The visible-rect option keeps the tracking region synchronized with the
/// view automatically, so no updateTrackingAreas override is required.
const CANVAS_TRACKING_OPTIONS: usize = 0x02 | 0x20 | 0x200;

const MODIFIER_FLAG_SHIFT: usize = 1 << 17;
const MODIFIER_FLAG_CONTROL: usize = 1 << 18;
const MODIFIER_FLAG_OPTION: usize = 1 << 19;
const MODIFIER_FLAG_COMMAND: usize = 1 << 20;

objc2::extern_class!(
    /// AppKit event-handling superclass in the NSView inheritance chain.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct NSResponder;
);

objc2::extern_class!(
    /// AppKit drawing surface superclass of the canvas view.
    #[unsafe(super(NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct NSView;
);

struct CanvasViewIvars {
    size: Cell<CanvasSize>,
    scene: RefCell<DrawScene>,
    events: EventBindings,
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = CanvasViewIvars]
    struct CanvasView;

    impl CanvasView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            // Scene coordinates are top-left origin with y increasing down.
            true
        }

        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size(&self) -> Size {
            let size = self.ivars().size.get();
            Size {
                width: size.width,
                height: size.height,
            }
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: *mut AnyObject) -> bool {
            // Graphical content reacts to the click that focuses the window.
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: Rect) {
            self.replay_scene();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Down, PointerButton::Primary);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Up, PointerButton::Primary);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Drag, PointerButton::Primary);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Down, PointerButton::Secondary);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Up, PointerButton::Secondary);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Drag, PointerButton::Secondary);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Down, PointerButton::Middle);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Up, PointerButton::Middle);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Drag, PointerButton::Middle);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Move, PointerButton::None);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &AnyObject) {
            self.emit_pointer(event, PointerPhase::Scroll, PointerButton::None);
        }
    }
);

impl CanvasView {
    fn new(
        mtm: MainThreadMarker,
        size: CanvasSize,
        scene: DrawScene,
        events: EventBindings,
    ) -> Retained<Self> {
        let object = Self::alloc(mtm).set_ivars(CanvasViewIvars {
            size: Cell::new(size),
            scene: RefCell::new(scene),
            events,
        });
        // SAFETY: initWithFrame: is NSView's designated initializer and the
        // ivars were initialized above on the main thread.
        let view: Retained<Self> = unsafe { msg_send![super(object), initWithFrame: Rect::default()] };
        // SAFETY: The tracking area is created owned, the view retains it in
        // addTrackingArea:, and the Id wrapper releases the creation retain.
        unsafe {
            let bounds: Rect = msg_send![&*view, bounds];
            let allocated: *mut AnyObject = msg_send![objc2::class!(NSTrackingArea), alloc];
            let area = Id::from_owned(msg_send![allocated,
                initWithRect: bounds,
                options: CANVAS_TRACKING_OPTIONS,
                owner: &*view,
                userInfo: std::ptr::null::<AnyObject>()
            ]);
            let _: () = msg_send![&*view, addTrackingArea: area.as_object()];
        }
        view
    }

    /// Replaces the retained scene and coalesces the change into one native
    /// redraw: AppKit collapses any number of setNeedsDisplay: marks issued
    /// before the next display pass into a single drawRect: invocation.
    fn apply_content(&self, size: CanvasSize, scene: &DrawScene) {
        let size_changed = self.ivars().size.get() != size;
        self.ivars().size.set(size);
        self.ivars().scene.borrow_mut().clone_from(scene);
        // SAFETY: The receiver is a live NSView on the main thread.
        unsafe {
            if size_changed {
                let _: () = msg_send![self, invalidateIntrinsicContentSize];
            }
            let _: () = msg_send![self, setNeedsDisplay: true];
        }
    }

    fn backing_scale(&self) -> f64 {
        // SAFETY: window is a nullable NSWindow queried on the main thread.
        let window: *mut AnyObject = unsafe { msg_send![self, window] };
        if window.is_null() {
            return 1.0;
        }
        // SAFETY: The receiver is the live NSWindow returned above.
        let scale: f64 = unsafe { msg_send![window, backingScaleFactor] };
        if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        }
    }

    fn replay_scene(&self) {
        let scale = self.backing_scale();
        let scene = self.ivars().scene.borrow();
        let mut clip_depth = 0_usize;
        for command in scene.commands() {
            replay_command(command, scale, &mut clip_depth);
        }
        // Validation guarantees balance; restore defensively so a future
        // regression cannot corrupt the shared graphics-context stack.
        for _ in 0..clip_depth {
            // SAFETY: Balances a saveGraphicsState issued by replay_command.
            unsafe {
                let _: () = msg_send![objc2::class!(NSGraphicsContext), restoreGraphicsState];
            }
        }
    }

    fn emit_pointer(&self, event: &AnyObject, phase: PointerPhase, button: PointerButton) {
        // SAFETY: The argument is a live NSEvent delivered by AppKit and the
        // receiver converts window coordinates on the main thread; the view
        // is flipped, so the local point is already top-left based.
        let (position, flags, scroll_delta) = unsafe {
            let location: Point = msg_send![event, locationInWindow];
            let local: Point = msg_send![
                self,
                convertPoint: location,
                fromView: std::ptr::null::<AnyObject>()
            ];
            let flags: usize = msg_send![event, modifierFlags];
            let scroll_delta = if phase == PointerPhase::Scroll {
                let dx: f64 = msg_send![event, scrollingDeltaX];
                let dy: f64 = msg_send![event, scrollingDeltaY];
                CanvasVector::new(dx, dy)
            } else {
                CanvasVector::default()
            };
            (CanvasPoint::new(local.x, local.y), flags, scroll_delta)
        };
        self.ivars().events.emit_pointer(PointerEvent {
            phase,
            position,
            button,
            modifiers: PointerModifiers {
                shift: flags & MODIFIER_FLAG_SHIFT != 0,
                control: flags & MODIFIER_FLAG_CONTROL != 0,
                option: flags & MODIFIER_FLAG_OPTION != 0,
                command: flags & MODIFIER_FLAG_COMMAND != 0,
            },
            scroll_delta,
        });
    }
}

fn create_canvas(
    mtm: MainThreadMarker,
    size: CanvasSize,
    scene: &DrawScene,
    accessibility_label: &str,
    events: EventBindings,
) -> AppKitHandle {
    let canvas = CanvasView::new(mtm, size, scene.clone(), events);
    // SAFETY: Retained keeps the object alive across this borrow.
    let view = unsafe {
        Id::from_borrowed(Retained::as_ptr(&canvas) as *mut AnyObject)
    };
    // SAFETY: NSView exposes the NSAccessibility configuration setters and
    // the role constant is a live static NSString.
    unsafe {
        let _: () = msg_send![view.as_object(), setAccessibilityElement: true];
        let _: () = msg_send![view.as_object(), setAccessibilityRole: ACCESSIBILITY_IMAGE_ROLE];
    }
    set_string(
        view.as_object(),
        SET_ACCESSIBILITY_LABEL,
        accessibility_label,
    );
    // The canvas retains its declared content size; parents may still
    // stretch it with required constraints.
    configure_growth(view.as_object(), false, false);
    let handle = AppKitHandle::new(
        view,
        HostKind::Element(ElementKind::Canvas),
        None,
        Vec::new(),
    );
    *handle.0.canvas_view.borrow_mut() = Some(canvas);
    handle
}

fn replay_command(command: &DrawCommand, scale: f64, clip_depth: &mut usize) {
    match command {
        DrawCommand::FillRect { rect, color } => {
            set_fill_color(*color);
            // SAFETY: NSBezierPath class factories return live autoreleased
            // paths; drawing happens inside drawRect: on the main thread.
            unsafe {
                let path: *mut AnyObject = msg_send![
                    objc2::class!(NSBezierPath),
                    bezierPathWithRect: canvas_rect(*rect)
                ];
                let _: () = msg_send![path, fill];
            }
        }
        DrawCommand::StrokeRect { rect, width, color } => {
            set_stroke_color(*color);
            let rect = match width {
                LineWidth::Hairline => snap_rect_to_device_pixels(*rect, scale),
                LineWidth::Points(_) => *rect,
            };
            // SAFETY: See FillRect.
            unsafe {
                let path: *mut AnyObject = msg_send![
                    objc2::class!(NSBezierPath),
                    bezierPathWithRect: canvas_rect(rect)
                ];
                let _: () = msg_send![path, setLineWidth: stroke_width(*width, scale)];
                let _: () = msg_send![path, stroke];
            }
        }
        DrawCommand::Line {
            from,
            to,
            width,
            color,
        } => {
            set_stroke_color(*color);
            let (from, to) = match width {
                LineWidth::Hairline => (
                    snap_point_to_device_pixels(*from, scale),
                    snap_point_to_device_pixels(*to, scale),
                ),
                LineWidth::Points(_) => (*from, *to),
            };
            // SAFETY: See FillRect.
            unsafe {
                let path: *mut AnyObject = msg_send![objc2::class!(NSBezierPath), bezierPath];
                let _: () = msg_send![path, moveToPoint: canvas_point(from)];
                let _: () = msg_send![path, lineToPoint: canvas_point(to)];
                let _: () = msg_send![path, setLineWidth: stroke_width(*width, scale)];
                let _: () = msg_send![path, stroke];
            }
        }
        DrawCommand::FillCircle {
            center,
            radius,
            color,
        } => {
            set_fill_color(*color);
            // SAFETY: See FillRect.
            unsafe {
                let path: *mut AnyObject = msg_send![
                    objc2::class!(NSBezierPath),
                    bezierPathWithOvalInRect: circle_bounds(*center, *radius)
                ];
                let _: () = msg_send![path, fill];
            }
        }
        DrawCommand::StrokeCircle {
            center,
            radius,
            width,
            color,
        } => {
            set_stroke_color(*color);
            // SAFETY: See FillRect.
            unsafe {
                let path: *mut AnyObject = msg_send![
                    objc2::class!(NSBezierPath),
                    bezierPathWithOvalInRect: circle_bounds(*center, *radius)
                ];
                let _: () = msg_send![path, setLineWidth: stroke_width(*width, scale)];
                let _: () = msg_send![path, stroke];
            }
        }
        DrawCommand::StrokeArc {
            center,
            radius,
            start_angle,
            end_angle,
            width,
            color,
        } => {
            set_stroke_color(*color);
            // The common contract measures angles toward positive y. In this
            // flipped view increasing angles already sweep toward positive y,
            // so clockwise:NO follows the declared direction.
            // SAFETY: See FillRect.
            unsafe {
                let path: *mut AnyObject = msg_send![objc2::class!(NSBezierPath), bezierPath];
                let _: () = msg_send![path,
                    appendBezierPathWithArcWithCenter: canvas_point(*center),
                    radius: *radius,
                    startAngle: start_angle.to_degrees(),
                    endAngle: end_angle.to_degrees(),
                    clockwise: false
                ];
                let _: () = msg_send![path, setLineWidth: stroke_width(*width, scale)];
                let _: () = msg_send![path, stroke];
            }
        }
        DrawCommand::GlyphRun {
            origin,
            text,
            font_size,
            color,
        } => draw_glyph_run(*origin, text, *font_size, *color),
        DrawCommand::PushClip { rect } => {
            *clip_depth += 1;
            // SAFETY: The graphics-context stack is balanced by PopClip or by
            // the defensive restore in replay_scene.
            unsafe {
                let _: () = msg_send![objc2::class!(NSGraphicsContext), saveGraphicsState];
                let path: *mut AnyObject = msg_send![
                    objc2::class!(NSBezierPath),
                    bezierPathWithRect: canvas_rect(*rect)
                ];
                let _: () = msg_send![path, addClip];
            }
        }
        DrawCommand::PopClip => {
            if let Some(depth) = clip_depth.checked_sub(1) {
                *clip_depth = depth;
                // SAFETY: Balances the saveGraphicsState of the matching push.
                unsafe {
                    let _: () = msg_send![objc2::class!(NSGraphicsContext), restoreGraphicsState];
                }
            }
        }
    }
}

fn draw_glyph_run(origin: CanvasPoint, text: &str, font_size: f64, color: CanvasColor) {
    let string = ns_string(text);
    // SAFETY: The attribute keys are live static NSString constants, the
    // dictionary retains its values, and NSString drawing inside drawRect:
    // honors the flipped context, placing the origin at the line-box top-left.
    unsafe {
        let font: *mut AnyObject = msg_send![
            objc2::class!(NSFont),
            monospacedSystemFontOfSize: font_size,
            weight: 0.0_f64
        ];
        let color: *mut AnyObject = msg_send![
            objc2::class!(NSColor),
            colorWithSRGBRed: color.red,
            green: color.green,
            blue: color.blue,
            alpha: color.alpha
        ];
        let attributes: *mut AnyObject = msg_send![objc2::class!(NSMutableDictionary), dictionary];
        let _: () = msg_send![attributes, setObject: font, forKey: FONT_ATTRIBUTE_NAME];
        let _: () = msg_send![attributes, setObject: color, forKey: FOREGROUND_COLOR_ATTRIBUTE_NAME];
        let _: () = msg_send![
            string.as_object(),
            drawAtPoint: canvas_point(origin),
            withAttributes: attributes
        ];
    }
}

fn set_fill_color(color: CanvasColor) {
    // SAFETY: NSColor factories return live autoreleased colors and setFill
    // configures the current drawRect: graphics context.
    unsafe {
        let color: *mut AnyObject = msg_send![
            objc2::class!(NSColor),
            colorWithSRGBRed: color.red,
            green: color.green,
            blue: color.blue,
            alpha: color.alpha
        ];
        let _: () = msg_send![color, setFill];
    }
}

fn set_stroke_color(color: CanvasColor) {
    // SAFETY: See set_fill_color; setStroke configures the stroke color.
    unsafe {
        let color: *mut AnyObject = msg_send![
            objc2::class!(NSColor),
            colorWithSRGBRed: color.red,
            green: color.green,
            blue: color.blue,
            alpha: color.alpha
        ];
        let _: () = msg_send![color, setStroke];
    }
}

const fn canvas_point(point: CanvasPoint) -> Point {
    Point {
        x: point.x,
        y: point.y,
    }
}

const fn canvas_rect(rect: CanvasRect) -> Rect {
    Rect {
        origin: Point {
            x: rect.origin.x,
            y: rect.origin.y,
        },
        size: Size {
            width: rect.size.width,
            height: rect.size.height,
        },
    }
}

fn circle_bounds(center: CanvasPoint, radius: f64) -> Rect {
    Rect {
        origin: Point {
            x: center.x - radius,
            y: center.y - radius,
        },
        size: Size {
            width: radius * 2.0,
            height: radius * 2.0,
        },
    }
}

/// Resolves a semantic stroke width into logical points for this backing.
fn stroke_width(width: LineWidth, scale: f64) -> f64 {
    match width {
        LineWidth::Hairline => 1.0 / scale,
        LineWidth::Points(value) => value,
    }
}

/// Snaps a logical coordinate onto the nearest device-pixel center so a
/// hairline stroke covers exactly one device pixel instead of smearing
/// across two half-covered neighbors.
fn snap_to_device_pixel_center(value: f64, scale: f64) -> f64 {
    ((value * scale - 0.5).round() + 0.5) / scale
}

fn snap_point_to_device_pixels(point: CanvasPoint, scale: f64) -> CanvasPoint {
    CanvasPoint::new(
        snap_to_device_pixel_center(point.x, scale),
        snap_to_device_pixel_center(point.y, scale),
    )
}

fn snap_rect_to_device_pixels(rect: CanvasRect, scale: f64) -> CanvasRect {
    let left = snap_to_device_pixel_center(rect.origin.x, scale);
    let top = snap_to_device_pixel_center(rect.origin.y, scale);
    let right = snap_to_device_pixel_center(rect.origin.x + rect.size.width, scale);
    let bottom = snap_to_device_pixel_center(rect.origin.y + rect.size.height, scale);
    CanvasRect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
}

/// Measures the native monospace system font for canvas glyph layout.
fn measure_monospace_metrics(font_size: f64) -> Option<MonospaceMetrics> {
    if !font_size.is_finite() || font_size <= 0.0 {
        return None;
    }
    // SAFETY: NSFont metrics are queried on the main thread where the
    // backend lives; the font object is a live autoreleased NSFont.
    let (row_height, glyph_width) = unsafe {
        let font: *mut AnyObject = msg_send![
            objc2::class!(NSFont),
            monospacedSystemFontOfSize: font_size,
            weight: 0.0_f64
        ];
        if font.is_null() {
            return None;
        }
        let ascender: f64 = msg_send![font, ascender];
        let descender: f64 = msg_send![font, descender];
        let leading: f64 = msg_send![font, leading];
        let advancement: Size = msg_send![font, maximumAdvancement];
        (ascender - descender + leading, advancement.width)
    };
    if !(row_height.is_finite() && row_height > 0.0 && glyph_width.is_finite() && glyph_width > 0.0)
    {
        return None;
    }
    Some(MonospaceMetrics {
        row_height,
        glyph_width,
    })
}
