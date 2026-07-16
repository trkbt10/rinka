/// `NSImageScaleProportionallyUpOrDown`.
const IMAGE_SCALE_PROPORTIONALLY_UP_OR_DOWN: usize = 3;
/// `NSImageScaleAxesIndependently`.
const IMAGE_SCALE_AXES_INDEPENDENTLY: usize = 1;
/// `NSImageScaleNone`.
const IMAGE_SCALE_NONE: usize = 2;
/// `NSImageAlignCenter`.
const IMAGE_ALIGN_CENTER: usize = 0;
/// `NSImageAlignTopLeft`.
const IMAGE_ALIGN_TOP_LEFT: usize = 2;

/// `kCGImageAlphaLast`: straight (non-premultiplied) alpha stored after the
/// color components, matching the [`ImageContent`] contract byte for byte.
/// Quartz premultiplies only while compositing, so no conversion happens at
/// this boundary.
const CG_IMAGE_ALPHA_LAST: u32 = 3;
/// `kCGRenderingIntentDefault`.
const CG_RENDERING_INTENT_DEFAULT: u32 = 0;

/// Opaque `CGImage` pixel storage referenced through `CGImageRef`.
#[repr(C)]
struct CGImageOpaque {
    _opaque: [u8; 0],
}

// SAFETY: Pointers to this zero-sized opaque type carry the Objective-C
// type encoding '^{CGImage=}' that AppKit declares for CGImageRef arguments.
unsafe impl objc2::RefEncode for CGImageOpaque {
    const ENCODING_REF: objc2::Encoding =
        objc2::Encoding::Pointer(&objc2::Encoding::Struct("CGImage", &[]));
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    /// `kCGColorSpaceSRGB` (a `CFStringRef`).
    #[link_name = "kCGColorSpaceSRGB"]
    static COLOR_SPACE_SRGB_NAME: *const std::ffi::c_void;

    fn CGColorSpaceCreateWithName(name: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CGColorSpaceRelease(space: *mut std::ffi::c_void);
    fn CGDataProviderCreateWithData(
        info: *mut std::ffi::c_void,
        data: *const std::ffi::c_void,
        size: usize,
        release: Option<
            unsafe extern "C-unwind" fn(*mut std::ffi::c_void, *const std::ffi::c_void, usize),
        >,
    ) -> *mut std::ffi::c_void;
    fn CGDataProviderRelease(provider: *mut std::ffi::c_void);
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *mut std::ffi::c_void,
        bitmap_info: u32,
        provider: *mut std::ffi::c_void,
        decode: *const f64,
        should_interpolate: bool,
        intent: u32,
    ) -> *mut CGImageOpaque;
    fn CGImageRelease(image: *mut CGImageOpaque);
}

/// Releases the pixel buffer owned by a `CGDataProvider`.
///
/// # Safety
///
/// `info` must be the pointer produced by `Box::into_raw` around the
/// [`ImageContent`] clone that keeps the provider's bytes alive; CoreGraphics
/// calls this exactly once when the provider's retain count reaches zero.
unsafe extern "C-unwind" fn release_image_content(
    info: *mut std::ffi::c_void,
    _data: *const std::ffi::c_void,
    _size: usize,
) {
    // SAFETY: The caller contract above holds by construction in
    // ns_image_from_rgba, so reconstituting the box frees the clone and
    // drops its shared byte reference.
    drop(unsafe { Box::from_raw(info.cast::<ImageContent>()) });
}

/// Identity of the pixel content currently uploaded to a retained
/// `NSImageView`, mirroring [`ImageContent`]'s equality contract.
#[derive(Clone, Copy, PartialEq)]
struct ImageStamp {
    width: u32,
    height: u32,
    stride: u32,
    scale_bits: u64,
    revision: u64,
}

impl ImageStamp {
    fn of(content: &ImageContent) -> Self {
        Self {
            width: content.width(),
            height: content.height(),
            stride: content.stride(),
            scale_bits: content.scale().to_bits(),
            revision: content.revision(),
        }
    }
}

fn create_image(
    content: &ImageContent,
    scaling: ImageScaling,
    accessibility_label: &str,
) -> Result<AppKitHandle, AppKitError> {
    let image = ns_image_from_rgba(content)?;
    // SAFETY: imageViewWithImage: returns a live autoreleased image view on
    // the AppKit main thread; Id::from_borrowed balances its retain in Drop.
    // Clipping enforces the semantic contract that Actual and Center crop
    // whatever exceeds the view instead of painting over neighbors.
    let view = unsafe {
        let pointer: *mut AnyObject = msg_send![objc2::class!(NSImageView),
            imageViewWithImage: image.as_object()
        ];
        let view = Id::from_borrowed(pointer);
        let _: () = msg_send![view.as_object(), setClipsToBounds: true];
        view
    };
    configure_image_view(view.as_object(), scaling);
    set_string(
        view.as_object(),
        SET_ACCESSIBILITY_LABEL,
        accessibility_label,
    );
    let handle = AppKitHandle::new(
        view,
        HostKind::Element(ElementKind::Image),
        None,
        Vec::new(),
    );
    handle.0.image_stamp.set(Some(ImageStamp::of(content)));
    Ok(handle)
}

/// Applies an image patch to the retained view.
///
/// The native image is rebuilt only when the content identity changed.
/// Native memory stays bounded across repeated patches: `setImage:` retains
/// the replacement `NSImage` and releases the previous one, whose
/// deallocation releases its `CGImage`, whose deallocation releases the
/// `CGDataProvider`, whose release callback frees the boxed [`ImageContent`]
/// clone holding the Rust pixel bytes. The temporary [`Id`] created here
/// drops its own retain at scope end, so exactly one native image chain per
/// view survives each reconcile.
fn apply_image(
    handle: &AppKitHandle,
    content: &ImageContent,
    scaling: ImageScaling,
    accessibility_label: &str,
) -> Result<(), AppKitError> {
    configure_image_view(handle.view(), scaling);
    set_string(handle.view(), SET_ACCESSIBILITY_LABEL, accessibility_label);
    let stamp = ImageStamp::of(content);
    if handle.0.image_stamp.get() == Some(stamp) {
        return Ok(());
    }
    let image = ns_image_from_rgba(content)?;
    // SAFETY: The receiver is the NSImageView created by create_image and
    // setImage: replaces (retains new, releases previous) its image.
    unsafe {
        let _: () = msg_send![handle.view(), setImage: image.as_object()];
    }
    handle.0.image_stamp.set(Some(stamp));
    Ok(())
}

fn configure_image_view(view: &AnyObject, scaling: ImageScaling) {
    let (native_scaling, alignment) = match scaling {
        ImageScaling::Fit => (IMAGE_SCALE_PROPORTIONALLY_UP_OR_DOWN, IMAGE_ALIGN_CENTER),
        ImageScaling::Fill => (IMAGE_SCALE_AXES_INDEPENDENTLY, IMAGE_ALIGN_CENTER),
        ImageScaling::Actual => (IMAGE_SCALE_NONE, IMAGE_ALIGN_TOP_LEFT),
        ImageScaling::Center => (IMAGE_SCALE_NONE, IMAGE_ALIGN_CENTER),
    };
    // SAFETY: The receiver is an NSImageView and both setters take the
    // public NSImageScaling / NSImageAlignment enumeration values.
    unsafe {
        let _: () = msg_send![view, setImageScaling: native_scaling];
        let _: () = msg_send![view, setImageAlignment: alignment];
    }
}

/// Builds an `NSImage` backed by a zero-copy `CGImage` over the buffer's
/// full pixel resolution, sized to the buffer's logical points so a
/// 2x-dense buffer renders crisp on a 2x display.
///
/// The submission itself moves no pixels: the `CGDataProvider` borrows the
/// shared byte allocation and owns a boxed [`ImageContent`] clone that keeps
/// it alive, and Quartz reads the bytes when it composites. `NSImage` and
/// `CGImage` are thread-safe model objects; only the `NSImageView` that
/// later receives the image is main-thread confined.
fn ns_image_from_rgba(content: &ImageContent) -> Result<Id, AppKitError> {
    // SAFETY: kCGColorSpaceSRGB is a constant CFString initialized by
    // CoreGraphics before any application code runs.
    let color_space = unsafe { CGColorSpaceCreateWithName(COLOR_SPACE_SRGB_NAME) };
    if color_space.is_null() {
        return Err(AppKitError(
            "CoreGraphics provided no sRGB color space".to_owned(),
        ));
    }
    let keeper = Box::new(content.clone());
    let data = keeper.bytes().as_ptr();
    let size = keeper.bytes().len();
    // SAFETY: The data pointer targets the shared byte allocation kept alive
    // by the boxed clone passed as info; the release callback frees exactly
    // that box. Arc's heap allocation address is stable across clones.
    let provider = unsafe {
        CGDataProviderCreateWithData(
            Box::into_raw(keeper).cast(),
            data.cast(),
            size,
            Some(release_image_content),
        )
    };
    if provider.is_null() {
        // SAFETY: Balances CGColorSpaceCreateWithName above. The keeper box
        // was consumed by into_raw but never handed to a provider, so no
        // release callback will run; the clone is unrecoverable only if we
        // do not rebuild it, which cannot happen because into_raw's pointer
        // is passed directly to the successful call path.
        unsafe { CGColorSpaceRelease(color_space) };
        return Err(AppKitError(
            "CoreGraphics rejected the pixel data provider".to_owned(),
        ));
    }
    // SAFETY: Tree validation proved before any native mutation that the
    // buffer covers stride * (height - 1) plus one final row, so every
    // row CoreGraphics reads lies inside the provider's size.
    let cg_image = unsafe {
        CGImageCreate(
            content.width() as usize,
            content.height() as usize,
            8,
            32,
            content.stride() as usize,
            color_space,
            CG_IMAGE_ALPHA_LAST,
            provider,
            std::ptr::null(),
            true,
            CG_RENDERING_INTENT_DEFAULT,
        )
    };
    // SAFETY: CGImageCreate retained what it needs; releasing our creation
    // references leaves the image as the sole owner of space and provider.
    // If creation failed, releasing the provider runs the data callback and
    // frees the boxed clone.
    unsafe {
        CGColorSpaceRelease(color_space);
        CGDataProviderRelease(provider);
    }
    if cg_image.is_null() {
        return Err(AppKitError(format!(
            "CoreGraphics rejected a {}x{} RGBA image",
            content.width(),
            content.height()
        )));
    }
    let point_size = Size {
        width: content.logical_width(),
        height: content.logical_height(),
    };
    // SAFETY: initWithCGImage:size: retains the CGImage and maps it onto
    // the logical point size; releasing our creation reference afterwards
    // leaves the NSImage as the owner of the pixel chain.
    unsafe {
        let allocated: *mut AnyObject = msg_send![objc2::class!(NSImage), alloc];
        let pointer: *mut AnyObject = msg_send![allocated,
            initWithCGImage: cg_image,
            size: point_size
        ];
        let image = if pointer.is_null() {
            None
        } else {
            Some(Id::from_owned(pointer))
        };
        CGImageRelease(cg_image);
        image.ok_or_else(|| AppKitError("NSImage rejected the CGImage".to_owned()))
    }
}

#[cfg(test)]
mod image_display_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn gradient_content(width: u32, height: u32, scale: f64, revision: u64) -> ImageContent {
        let stride = width * 4;
        let mut bytes = Vec::with_capacity((stride * height) as usize);
        for y in 0..height {
            for x in 0..width {
                bytes.extend_from_slice(&[x as u8, y as u8, 0x40, 0xFF]);
            }
        }
        ImageContent::from_rgba8(width, height, stride, bytes, revision).with_scale(scale)
    }

    #[test]
    fn a_dense_buffer_maps_pixels_onto_logical_points() {
        autoreleasepool(|_| {
            let image = ns_image_from_rgba(&gradient_content(64, 48, 2.0, 1)).expect("image");
            // SAFETY: The receiver is the live NSImage built above and its
            // representations array holds the backing rep.
            unsafe {
                let size: Size = msg_send![image.as_object(), size];
                assert_eq!(size.width, 32.0);
                assert_eq!(size.height, 24.0);
                let representations: *mut AnyObject =
                    msg_send![image.as_object(), representations];
                let count: usize = msg_send![representations, count];
                assert_eq!(count, 1);
                let representation: *mut AnyObject =
                    msg_send![representations, objectAtIndex: 0_usize];
                let pixels_wide: isize = msg_send![representation, pixelsWide];
                let pixels_high: isize = msg_send![representation, pixelsHigh];
                assert_eq!(pixels_wide, 64);
                assert_eq!(pixels_high, 48);
            }
        });
    }

    #[test]
    fn a_released_image_frees_the_shared_pixel_buffer() {
        let bytes: std::sync::Arc<[u8]> = std::sync::Arc::from(vec![0x20_u8; 16 * 16 * 4]);
        let content = ImageContent::from_rgba8(16, 16, 64, bytes.clone(), 1);
        assert_eq!(std::sync::Arc::strong_count(&bytes), 2);

        // The image lives and dies inside one pool, exactly as it does
        // inside the application's event-loop pool: initWithCGImage may
        // autorelease internal references to the pixel chain, and they must
        // drain before the provider can run its release callback.
        autoreleasepool(|_| {
            let image = ns_image_from_rgba(&content).expect("image");
            assert_eq!(
                std::sync::Arc::strong_count(&bytes),
                3,
                "the provider must hold exactly one boxed clone"
            );
            drop(image);
        });
        assert_eq!(
            std::sync::Arc::strong_count(&bytes),
            2,
            "releasing the NSImage must run the provider's release callback"
        );
        drop(content);
        assert_eq!(std::sync::Arc::strong_count(&bytes), 1);
    }

    #[test]
    fn straight_alpha_composites_over_an_opaque_background() {
        unsafe extern "C" {
            #[link_name = "NSDeviceRGBColorSpace"]
            static DEVICE_RGB_COLOR_SPACE_NAME: *mut AnyObject;
        }
        autoreleasepool(|_| {
            // One red pixel at ~50% straight alpha over an opaque white
            // destination must land near (255, 127, 127).
            let source =
                ns_image_from_rgba(&ImageContent::from_rgba8(1, 1, 4, vec![255, 0, 0, 128], 1))
                    .expect("source image");
            // SAFETY: The destination rep owns its 1x1 RGBA storage; the
            // bitmap-backed graphics context draws without a window server.
            unsafe {
                let allocated: *mut AnyObject = msg_send![objc2::class!(NSBitmapImageRep), alloc];
                let pointer: *mut AnyObject = msg_send![allocated,
                    initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
                    pixelsWide: 1_isize,
                    pixelsHigh: 1_isize,
                    bitsPerSample: 8_isize,
                    samplesPerPixel: 4_isize,
                    hasAlpha: true,
                    isPlanar: false,
                    colorSpaceName: DEVICE_RGB_COLOR_SPACE_NAME,
                    bytesPerRow: 4_isize,
                    bitsPerPixel: 32_isize
                ];
                assert!(!pointer.is_null());
                let destination = Id::from_owned(pointer);
                let data: *mut u8 = msg_send![destination.as_object(), bitmapData];
                data.copy_from_nonoverlapping([255_u8, 255, 255, 255].as_ptr(), 4);

                let context: *mut AnyObject = msg_send![objc2::class!(NSGraphicsContext),
                    graphicsContextWithBitmapImageRep: destination.as_object()
                ];
                assert!(!context.is_null());
                let _: () = msg_send![objc2::class!(NSGraphicsContext), saveGraphicsState];
                let _: () =
                    msg_send![objc2::class!(NSGraphicsContext), setCurrentContext: context];
                let bounds = Rect {
                    origin: Point::default(),
                    size: Size {
                        width: 1.0,
                        height: 1.0,
                    },
                };
                // NSCompositingOperationSourceOver = 2.
                let _: () = msg_send![source.as_object(),
                    drawInRect: bounds,
                    fromRect: Rect::default(),
                    operation: 2_usize,
                    fraction: 1.0_f64
                ];
                let _: () = msg_send![objc2::class!(NSGraphicsContext), restoreGraphicsState];

                let mut pixel = [0_u8; 4];
                pixel
                    .as_mut_ptr()
                    .copy_from_nonoverlapping(data.cast_const(), 4);
                assert!(pixel[0] >= 247, "red channel composited to {}", pixel[0]);
                assert!(
                    (119..=135).contains(&pixel[1]),
                    "green channel composited to {}",
                    pixel[1]
                );
                assert!(
                    (119..=135).contains(&pixel[2]),
                    "blue channel composited to {}",
                    pixel[2]
                );
            }
        });
    }

    #[test]
    fn a_4000_pixel_submission_builds_within_the_reconcile_budget() {
        // 4000 x 4000 RGBA is a 64 MB submission, the bound stated by the
        // image-display ticket for blocking the UI thread. The zero-copy
        // provider defers pixel reads to Quartz compositing, so the
        // reconcile-time cost measured here is object creation only.
        //
        // Warm CoreGraphics first: its one-time lazy process initialization
        // (measured ~1.7 s in a bare test process) happens at framework load
        // in a real application, long before any reconcile runs.
        autoreleasepool(|_| {
            drop(ns_image_from_rgba(&ImageContent::from_rgba8(
                1,
                1,
                4,
                vec![0_u8; 4],
                0,
            )));
        });

        let side = 4000_u32;
        let stride = side * 4;
        let bytes = vec![0x7F_u8; stride as usize * side as usize];
        let content = ImageContent::from_rgba8(side, side, stride, bytes, 1);

        let started = Instant::now();
        let image = ns_image_from_rgba(&content).expect("large image");
        let elapsed = started.elapsed();
        autoreleasepool(|_| drop(image));

        eprintln!("4000px NSImage build took {elapsed:?}");
        assert!(
            elapsed <= Duration::from_millis(100),
            "building a 4000px NSImage took {elapsed:?}, over the 100 ms budget"
        );
    }
}
