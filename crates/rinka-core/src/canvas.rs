//! Platform-neutral owned-drawing vocabulary for inherently graphical content.
//!
//! A canvas element is reserved for content that has no native control
//! equivalent — terminal cell grids, audio meters, dashboard widget faces.
//! It is never an escape hatch for imitating a native button, list, input, or
//! any other control; such use is a contract violation, not a workaround.
//!
//! Coordinates are logical points with the origin at the element's top-left
//! corner and the y axis increasing downward. Adapters translate logical
//! points into device pixels using the native backing scale, so a scene is
//! HiDPI-correct without carrying a scale factor. A stroke that must resolve
//! to exactly one device pixel declares [`LineWidth::Hairline`] and the
//! adapter both converts and pixel-aligns it natively.
//!
//! The scene is explicit retained state: the application rebuilds a
//! [`DrawScene`] in its view function and the reconciler compares scenes by
//! value. Draw closures are rejected by design because a closure cannot be
//! diffed by a retained reconciler.

/// Position in element-local logical points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasPoint {
    /// Distance from the element's leading edge.
    pub x: f64,
    /// Distance from the element's top edge.
    pub y: f64,
}

impl CanvasPoint {
    /// Creates a point.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    const fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// Displacement in logical points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasVector {
    /// Horizontal displacement.
    pub dx: f64,
    /// Vertical displacement.
    pub dy: f64,
}

impl CanvasVector {
    /// Creates a displacement.
    pub const fn new(dx: f64, dy: f64) -> Self {
        Self { dx, dy }
    }
}

/// Extent in logical points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasSize {
    /// Horizontal extent.
    pub width: f64,
    /// Vertical extent.
    pub height: f64,
}

impl CanvasSize {
    /// Creates a size.
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    const fn is_finite(self) -> bool {
        self.width.is_finite() && self.height.is_finite()
    }
}

/// Axis-aligned rectangle in element-local logical points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasRect {
    /// Top-left corner.
    pub origin: CanvasPoint,
    /// Extent toward the bottom-right.
    pub size: CanvasSize,
}

impl CanvasRect {
    /// Creates a rectangle from its top-left corner and extent.
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: CanvasPoint::new(x, y),
            size: CanvasSize::new(width, height),
        }
    }

    fn invalid_reason(&self) -> Option<String> {
        if !self.origin.is_finite() || !self.size.is_finite() {
            return Some("rectangle coordinates must be finite".to_owned());
        }
        if self.size.width < 0.0 || self.size.height < 0.0 {
            return Some("rectangle extent must not be negative".to_owned());
        }
        None
    }
}

/// Straight-alpha sRGB color.
///
/// The canvas owns its pixels, so colors here are literal values chosen by
/// the graphical content, not semantic roles resolved by the platform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasColor {
    /// Red component in `0.0..=1.0`.
    pub red: f64,
    /// Green component in `0.0..=1.0`.
    pub green: f64,
    /// Blue component in `0.0..=1.0`.
    pub blue: f64,
    /// Opacity in `0.0..=1.0`.
    pub alpha: f64,
}

impl CanvasColor {
    /// Creates an opaque color.
    pub const fn rgb(red: f64, green: f64, blue: f64) -> Self {
        Self::rgba(red, green, blue, 1.0)
    }

    /// Creates a color with explicit opacity.
    pub const fn rgba(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    fn invalid_reason(&self) -> Option<String> {
        for component in [self.red, self.green, self.blue, self.alpha] {
            if !component.is_finite() || !(0.0..=1.0).contains(&component) {
                return Some(format!(
                    "color components must be finite and within 0..=1, received {component}"
                ));
            }
        }
        None
    }
}

/// Stroke thickness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineWidth {
    /// Exactly one device pixel, converted and pixel-aligned by the adapter.
    Hairline,
    /// Thickness in logical points.
    Points(f64),
}

impl LineWidth {
    fn invalid_reason(&self) -> Option<String> {
        match self {
            Self::Hairline => None,
            Self::Points(value) if value.is_finite() && *value > 0.0 => None,
            Self::Points(value) => Some(format!(
                "stroke width in points must be finite and positive, received {value}"
            )),
        }
    }
}

/// One recorded drawing operation replayed by a native adapter.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCommand {
    /// Fills a rectangle.
    FillRect {
        /// Filled area.
        rect: CanvasRect,
        /// Fill color.
        color: CanvasColor,
    },
    /// Strokes a rectangle outline centered on its edge.
    StrokeRect {
        /// Outlined area.
        rect: CanvasRect,
        /// Stroke thickness.
        width: LineWidth,
        /// Stroke color.
        color: CanvasColor,
    },
    /// Strokes a straight line segment.
    Line {
        /// Start point.
        from: CanvasPoint,
        /// End point.
        to: CanvasPoint,
        /// Stroke thickness.
        width: LineWidth,
        /// Stroke color.
        color: CanvasColor,
    },
    /// Fills a circle.
    FillCircle {
        /// Circle center.
        center: CanvasPoint,
        /// Radius in logical points.
        radius: f64,
        /// Fill color.
        color: CanvasColor,
    },
    /// Strokes a circle outline.
    StrokeCircle {
        /// Circle center.
        center: CanvasPoint,
        /// Radius in logical points.
        radius: f64,
        /// Stroke thickness.
        width: LineWidth,
        /// Stroke color.
        color: CanvasColor,
    },
    /// Strokes a circular arc.
    ///
    /// Angles are radians measured from the positive x axis toward the
    /// positive y axis (clockwise on screen), sweeping from `start_angle`
    /// to `end_angle` in that direction.
    StrokeArc {
        /// Arc center.
        center: CanvasPoint,
        /// Radius in logical points.
        radius: f64,
        /// Sweep start in radians.
        start_angle: f64,
        /// Sweep end in radians.
        end_angle: f64,
        /// Stroke thickness.
        width: LineWidth,
        /// Stroke color.
        color: CanvasColor,
    },
    /// Draws one run of text in the platform's monospace font.
    ///
    /// `origin` is the top-left corner of the run's line box; the adapter
    /// resolves the native baseline. Cell placement uses the adapter's
    /// [`MonospaceMetrics`].
    GlyphRun {
        /// Top-left corner of the run's line box.
        origin: CanvasPoint,
        /// Text content.
        text: String,
        /// Font size in logical points.
        font_size: f64,
        /// Text color.
        color: CanvasColor,
    },
    /// Intersects the clip region with a rectangle until the matching pop.
    PushClip {
        /// Clip area.
        rect: CanvasRect,
    },
    /// Restores the clip region saved by the matching push.
    PopClip,
}

impl DrawCommand {
    fn invalid_reason(&self) -> Option<String> {
        match self {
            Self::FillRect { rect, color } => {
                rect.invalid_reason().or_else(|| color.invalid_reason())
            }
            Self::StrokeRect { rect, width, color } => rect
                .invalid_reason()
                .or_else(|| width.invalid_reason())
                .or_else(|| color.invalid_reason()),
            Self::Line {
                from,
                to,
                width,
                color,
            } => {
                if !from.is_finite() || !to.is_finite() {
                    return Some("line endpoints must be finite".to_owned());
                }
                width.invalid_reason().or_else(|| color.invalid_reason())
            }
            Self::FillCircle {
                center,
                radius,
                color,
            } => circle_invalid_reason(*center, *radius).or_else(|| color.invalid_reason()),
            Self::StrokeCircle {
                center,
                radius,
                width,
                color,
            } => circle_invalid_reason(*center, *radius)
                .or_else(|| width.invalid_reason())
                .or_else(|| color.invalid_reason()),
            Self::StrokeArc {
                center,
                radius,
                start_angle,
                end_angle,
                width,
                color,
            } => {
                if !start_angle.is_finite() || !end_angle.is_finite() {
                    return Some("arc angles must be finite".to_owned());
                }
                circle_invalid_reason(*center, *radius)
                    .or_else(|| width.invalid_reason())
                    .or_else(|| color.invalid_reason())
            }
            Self::GlyphRun {
                origin,
                font_size,
                color,
                ..
            } => {
                if !origin.is_finite() {
                    return Some("glyph run origin must be finite".to_owned());
                }
                if !font_size.is_finite() || *font_size <= 0.0 {
                    return Some(format!(
                        "glyph run font size must be finite and positive, received {font_size}"
                    ));
                }
                color.invalid_reason()
            }
            Self::PushClip { rect } => rect.invalid_reason(),
            Self::PopClip => None,
        }
    }
}

fn circle_invalid_reason(center: CanvasPoint, radius: f64) -> Option<String> {
    if !center.is_finite() {
        return Some("circle center must be finite".to_owned());
    }
    if !radius.is_finite() || radius < 0.0 {
        return Some(format!(
            "circle radius must be finite and not negative, received {radius}"
        ));
    }
    None
}

/// Recorded, comparable display list rebuilt by the application each render.
///
/// The reconciler diffs scenes by value: rebuilding an equal scene issues no
/// native mutation, and any number of state changes folded into one rebuilt
/// scene coalesce into at most one native property patch per render.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawScene {
    commands: Vec<DrawCommand>,
}

impl DrawScene {
    /// Creates an empty scene.
    pub const fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Returns the recorded commands in draw order.
    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }

    /// Returns whether the scene records no commands.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Fills a rectangle.
    pub fn fill_rect(&mut self, rect: CanvasRect, color: CanvasColor) {
        self.commands.push(DrawCommand::FillRect { rect, color });
    }

    /// Strokes a rectangle outline.
    pub fn stroke_rect(&mut self, rect: CanvasRect, width: LineWidth, color: CanvasColor) {
        self.commands
            .push(DrawCommand::StrokeRect { rect, width, color });
    }

    /// Strokes a straight line segment.
    pub fn line(
        &mut self,
        from: CanvasPoint,
        to: CanvasPoint,
        width: LineWidth,
        color: CanvasColor,
    ) {
        self.commands.push(DrawCommand::Line {
            from,
            to,
            width,
            color,
        });
    }

    /// Fills a circle.
    pub fn fill_circle(&mut self, center: CanvasPoint, radius: f64, color: CanvasColor) {
        self.commands.push(DrawCommand::FillCircle {
            center,
            radius,
            color,
        });
    }

    /// Strokes a circle outline.
    pub fn stroke_circle(
        &mut self,
        center: CanvasPoint,
        radius: f64,
        width: LineWidth,
        color: CanvasColor,
    ) {
        self.commands.push(DrawCommand::StrokeCircle {
            center,
            radius,
            width,
            color,
        });
    }

    /// Strokes a circular arc; see [`DrawCommand::StrokeArc`] for angles.
    pub fn stroke_arc(
        &mut self,
        center: CanvasPoint,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        width: LineWidth,
        color: CanvasColor,
    ) {
        self.commands.push(DrawCommand::StrokeArc {
            center,
            radius,
            start_angle,
            end_angle,
            width,
            color,
        });
    }

    /// Draws one monospace text run; see [`DrawCommand::GlyphRun`].
    pub fn glyph_run(
        &mut self,
        origin: CanvasPoint,
        text: impl Into<String>,
        font_size: f64,
        color: CanvasColor,
    ) {
        self.commands.push(DrawCommand::GlyphRun {
            origin,
            text: text.into(),
            font_size,
            color,
        });
    }

    /// Intersects the clip region with a rectangle until the matching pop.
    pub fn push_clip(&mut self, rect: CanvasRect) {
        self.commands.push(DrawCommand::PushClip { rect });
    }

    /// Restores the clip region saved by the matching push.
    pub fn pop_clip(&mut self) {
        self.commands.push(DrawCommand::PopClip);
    }

    pub(crate) fn invalid_reason(&self) -> Option<String> {
        let mut clip_depth = 0_usize;
        for (index, command) in self.commands.iter().enumerate() {
            if let Some(reason) = command.invalid_reason() {
                return Some(format!("command {index}: {reason}"));
            }
            match command {
                DrawCommand::PushClip { .. } => clip_depth += 1,
                DrawCommand::PopClip => {
                    let Some(depth) = clip_depth.checked_sub(1) else {
                        return Some(format!("command {index}: clip pop without a matching push"));
                    };
                    clip_depth = depth;
                }
                _ => {}
            }
        }
        if clip_depth > 0 {
            return Some(format!("{clip_depth} clip push(es) without a matching pop"));
        }
        None
    }
}

/// Native monospace font measurements for canvas cell layout.
///
/// Values are logical points measured by a platform adapter from its native
/// monospace font; the common crate never fabricates them. A terminal grid
/// places the cell at row `r`, column `c` at
/// `(c * glyph_width, r * row_height)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonospaceMetrics {
    /// Vertical distance between successive cell rows.
    pub row_height: f64,
    /// Horizontal advance of one monospace glyph.
    pub glyph_width: f64,
}

/// Interaction stage of one pointer event.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PointerPhase {
    /// A button was pressed.
    Down,
    /// A button was released.
    Up,
    /// The pointer moved with no button held.
    Move,
    /// The pointer moved while a button was held.
    Drag,
    /// The scroll wheel or gesture scrolled over the element.
    Scroll,
}

/// Pointer button that produced an event.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PointerButton {
    /// No button, as during a plain move or scroll.
    None,
    /// The primary button.
    Primary,
    /// The secondary (context-menu) button.
    Secondary,
    /// The middle button or wheel press.
    Middle,
}

/// Keyboard modifiers held during a pointer event.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct PointerModifiers {
    /// Shift key.
    pub shift: bool,
    /// Control key.
    pub control: bool,
    /// Option or Alt key.
    pub option: bool,
    /// Command key on macOS, Windows/Super key elsewhere.
    pub command: bool,
}

/// One pointer interaction delivered in element-local logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    /// Interaction stage.
    pub phase: PointerPhase,
    /// Position relative to the element's top-left corner.
    pub position: CanvasPoint,
    /// Button that produced the event.
    pub button: PointerButton,
    /// Modifiers held during the event.
    pub modifiers: PointerModifiers,
    /// Scroll displacement; zero unless [`PointerPhase::Scroll`].
    pub scroll_delta: CanvasVector,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rebuilt_equal_scene_compares_equal() {
        let build = || {
            let mut scene = DrawScene::new();
            scene.fill_rect(
                CanvasRect::new(0.0, 0.0, 10.0, 10.0),
                CanvasColor::rgb(0.2, 0.4, 0.6),
            );
            scene.glyph_run(
                CanvasPoint::new(1.0, 2.0),
                "abc",
                13.0,
                CanvasColor::rgb(0.0, 0.0, 0.0),
            );
            scene
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn balanced_clips_are_valid_and_unbalanced_clips_are_rejected() {
        let mut balanced = DrawScene::new();
        balanced.push_clip(CanvasRect::new(0.0, 0.0, 4.0, 4.0));
        balanced.pop_clip();
        assert_eq!(balanced.invalid_reason(), None);

        let mut unpopped = DrawScene::new();
        unpopped.push_clip(CanvasRect::new(0.0, 0.0, 4.0, 4.0));
        assert!(
            unpopped
                .invalid_reason()
                .is_some_and(|reason| reason.contains("without a matching pop"))
        );

        let mut unpushed = DrawScene::new();
        unpushed.pop_clip();
        assert!(
            unpushed
                .invalid_reason()
                .is_some_and(|reason| reason.contains("without a matching push"))
        );
    }

    #[test]
    fn non_finite_and_non_positive_values_are_rejected() {
        let mut nan_rect = DrawScene::new();
        nan_rect.fill_rect(
            CanvasRect::new(f64::NAN, 0.0, 1.0, 1.0),
            CanvasColor::rgb(0.0, 0.0, 0.0),
        );
        assert!(nan_rect.invalid_reason().is_some());

        let mut zero_stroke = DrawScene::new();
        zero_stroke.line(
            CanvasPoint::new(0.0, 0.0),
            CanvasPoint::new(1.0, 1.0),
            LineWidth::Points(0.0),
            CanvasColor::rgb(0.0, 0.0, 0.0),
        );
        assert!(zero_stroke.invalid_reason().is_some());

        let mut out_of_range_color = DrawScene::new();
        out_of_range_color.fill_circle(
            CanvasPoint::new(0.0, 0.0),
            2.0,
            CanvasColor::rgb(1.5, 0.0, 0.0),
        );
        assert!(out_of_range_color.invalid_reason().is_some());

        let mut hairline = DrawScene::new();
        hairline.line(
            CanvasPoint::new(0.0, 0.5),
            CanvasPoint::new(8.0, 0.5),
            LineWidth::Hairline,
            CanvasColor::rgb(0.0, 0.0, 0.0),
        );
        assert_eq!(hairline.invalid_reason(), None);
    }
}
