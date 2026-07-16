//! Consumer-level contracts for the bitmap image element.

use rinka_core::{ImageContent, ImageScaling, Props, Renderer, column, image, label};
use rinka_headless::{HeadlessBackend, Operation};
use std::sync::Arc;

/// Returns a deterministic RGBA8 buffer of the requested geometry.
fn rgba_buffer(width: u32, height: u32, seed: u8) -> Arc<[u8]> {
    let mut bytes = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            bytes.extend_from_slice(&[
                (x as u8).wrapping_add(seed),
                (y as u8).wrapping_add(seed),
                seed,
                0xFF,
            ]);
        }
    }
    Arc::from(bytes)
}

fn preview(content: ImageContent, scaling: ImageScaling) -> rinka_core::Element {
    column([image(content, "Preview")
        .image_scaling(scaling)
        .with_key("preview")])
    .with_key("screen")
}

#[test]
fn mounting_an_image_records_its_content_and_geometry() {
    let bytes = rgba_buffer(4, 3, 1);
    let content = ImageContent::from_rgba8(4, 3, 16, bytes, 1).with_scale(2.0);
    let mut renderer = Renderer::new(HeadlessBackend::new());

    let stats = renderer
        .render(preview(content, ImageScaling::Fit))
        .unwrap();

    assert_eq!(stats.created, 2);
    let handle = renderer.backend().find_by_key("preview").unwrap();
    let props = renderer.backend().props_of(handle).unwrap();
    let Props::Image {
        content, scaling, ..
    } = props
    else {
        panic!("mounted node must carry image properties, got {props:?}");
    };
    assert_eq!(content.width(), 4);
    assert_eq!(content.height(), 3);
    assert_eq!(content.stride(), 16);
    assert_eq!(content.revision(), 1);
    assert_eq!(content.logical_width(), 2.0);
    assert_eq!(content.logical_height(), 1.5);
    assert_eq!(*scaling, ImageScaling::Fit);
    assert!(
        renderer
            .backend()
            .operations()
            .iter()
            .any(|operation| matches!(
                operation,
                Operation::Create {
                    props: Props::Image { .. },
                    ..
                }
            ))
    );
}

#[test]
fn identical_revision_and_geometry_issue_no_patch() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    // Two independently allocated buffers with identical pixels and the same
    // consumer-declared revision: the reconciler must treat them as the same
    // picture and leave the retained native image untouched.
    let first = ImageContent::from_rgba8(8, 8, 32, rgba_buffer(8, 8, 7), 42);
    let second = ImageContent::from_rgba8(8, 8, 32, rgba_buffer(8, 8, 7), 42);
    renderer.render(preview(first, ImageScaling::Fit)).unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer.render(preview(second, ImageScaling::Fit)).unwrap();

    assert_eq!(stats.patched, 0);
    assert_eq!(stats.replaced, 0);
    assert_eq!(stats.created, 0);
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn buffer_replacement_patches_without_remounting() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let first = ImageContent::from_rgba8(8, 8, 32, rgba_buffer(8, 8, 1), 1);
    let second = ImageContent::from_rgba8(8, 8, 32, rgba_buffer(8, 8, 2), 2);
    renderer.render(preview(first, ImageScaling::Fit)).unwrap();
    let before = renderer.backend().find_by_key("preview").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer.render(preview(second, ImageScaling::Fit)).unwrap();

    assert_eq!(stats.patched, 1);
    assert_eq!(stats.created, 0);
    assert_eq!(stats.removed, 0);
    assert_eq!(stats.replaced, 0);
    assert_eq!(renderer.backend().find_by_key("preview"), Some(before));
    assert!(matches!(
        renderer.backend().props_of(before),
        Some(Props::Image { content, .. }) if content.revision() == 2
    ));
}

#[test]
fn scaling_mode_change_patches_in_place() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    let content = || ImageContent::from_rgba8(8, 8, 32, rgba_buffer(8, 8, 3), 5);
    renderer
        .render(preview(content(), ImageScaling::Fit))
        .unwrap();
    let before = renderer.backend().find_by_key("preview").unwrap();
    renderer.backend_mut().clear_operations();

    let stats = renderer
        .render(preview(content(), ImageScaling::Center))
        .unwrap();

    assert_eq!(stats.patched, 1);
    assert_eq!(stats.created, 0);
    assert_eq!(renderer.backend().find_by_key("preview"), Some(before));
    assert!(matches!(
        renderer.backend().props_of(before),
        Some(Props::Image {
            scaling: ImageScaling::Center,
            ..
        })
    ));
}

#[test]
fn unmounting_destroys_the_node_and_releases_the_buffer() {
    let bytes = rgba_buffer(8, 8, 4);
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(preview(
            ImageContent::from_rgba8(8, 8, 32, bytes.clone(), 1),
            ImageScaling::Fit,
        ))
        .unwrap();
    let handle = renderer.backend().find_by_key("preview").unwrap();
    renderer.backend_mut().clear_operations();

    renderer
        .render(column([label("no preview").with_key("empty")]).with_key("screen"))
        .unwrap();

    assert!(renderer.backend().operations().iter().any(
        |operation| matches!(operation, Operation::Destroy { handle: destroyed } if *destroyed == handle)
    ));
    renderer.backend_mut().clear_operations();
    // Only the test's own handle keeps the pixel buffer alive once the node
    // and the recorded operations are gone.
    assert_eq!(Arc::strong_count(&bytes), 1);
}

#[test]
fn a_thousand_reconciles_swapping_two_buffers_stay_bounded() {
    let first = rgba_buffer(16, 16, 1);
    let second = rgba_buffer(16, 16, 2);
    let mut renderer = Renderer::new(HeadlessBackend::new());
    renderer
        .render(preview(
            ImageContent::from_rgba8(16, 16, 64, first.clone(), 1),
            ImageScaling::Fit,
        ))
        .unwrap();
    renderer.backend_mut().clear_operations();

    for cycle in 0..1000 {
        let (bytes, revision) = if cycle % 2 == 0 {
            (second.clone(), 2)
        } else {
            (first.clone(), 1)
        };
        let stats = renderer
            .render(preview(
                ImageContent::from_rgba8(16, 16, 64, bytes, revision),
                ImageScaling::Fit,
            ))
            .unwrap();
        assert_eq!(stats.patched, 1, "cycle {cycle} must patch in place");
        assert_eq!(stats.created, 0, "cycle {cycle} must not mount");
        assert_eq!(stats.removed, 0, "cycle {cycle} must not destroy");
        renderer.backend_mut().clear_operations();

        // Exactly one retained tree exists: the mounted buffer is held by
        // the retained descriptor and the modeled native node, the swapped
        // out buffer only by this test. Any growth here is a leak.
        let (mounted, released) = if cycle % 2 == 0 {
            (&second, &first)
        } else {
            (&first, &second)
        };
        assert_eq!(
            Arc::strong_count(mounted),
            3,
            "cycle {cycle} retained an unexpected number of mounted buffers"
        );
        assert_eq!(
            Arc::strong_count(released),
            1,
            "cycle {cycle} leaked the replaced buffer"
        );
    }
}

#[test]
fn invalid_image_geometry_is_rejected_before_native_mutation() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    // Stride shorter than one row: 8 pixels require 32 bytes per row.
    let content = ImageContent::from_rgba8(8, 8, 16, rgba_buffer(8, 8, 1), 1);

    let error = renderer
        .render(preview(content, ImageScaling::Fit))
        .unwrap_err();

    assert!(error.to_string().contains("invalid image"));
    assert!(renderer.backend().operations().is_empty());
}

#[test]
fn undersized_buffers_are_rejected_before_native_mutation() {
    let mut renderer = Renderer::new(HeadlessBackend::new());
    // The geometry declares 16 rows but the buffer holds 8.
    let content = ImageContent::from_rgba8(16, 16, 64, rgba_buffer(16, 8, 1), 1);

    let error = renderer
        .render(preview(content, ImageScaling::Fit))
        .unwrap_err();

    assert!(error.to_string().contains("invalid image"));
    assert!(renderer.backend().operations().is_empty());
}
