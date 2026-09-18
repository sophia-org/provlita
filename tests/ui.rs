//! Actual Xilem reconciliation and message dispatch; CPU raster fixture only.
//! These controls do not claim Sophia input authority or GPU presentation.
use imaging_vello_cpu::VelloCpuRenderer;
use masonry::{imaging::render::ImageRenderer, widgets::ButtonPress};
use provlita::{config::Config, ui::DockHost};
use xilem_masonry::core::DynMessage;

fn config() -> Config {
    Config::parse(include_str!("../examples/minimal/config.kdl")).unwrap()
}
fn entries() -> Vec<(String, u16, bool)> {
    vec![
        ("registered:terminal".into(), 3, true),
        ("registered:browser".into(), 1, true),
        ("registered:files".into(), 2, false),
    ]
}
fn pixels(host: &mut DockHost) -> Vec<u8> {
    let mut frame = host.scene();
    frame.scene.validate().unwrap();
    let mut raster = VelloCpuRenderer::new(frame.width as u16, frame.height as u16);
    raster
        .render_source(&mut frame.scene, frame.width, frame.height)
        .unwrap()
        .data
}

#[test]
fn first_frame_contains_distinct_vector_artwork_and_opaque_tiles() {
    let mut host = DockHost::new(&config(), 1).unwrap();
    let width = config().logical_size().0 as usize;
    let first = pixels(&mut host);
    let mut crops = Vec::new();
    for index in 0..3 {
        let mut colors = std::collections::BTreeSet::new();
        let mut crop = Vec::new();
        // Exclude the caption and border: plain text and background alone
        // cannot satisfy the vector-artwork control.
        for y in 12..34 {
            for x in (18 + index * 54)..(46 + index * 54) {
                let pixel = &first[(y * width + x) * 4..(y * width + x + 1) * 4];
                assert_eq!(pixel[3], 255, "tile interior must be opaque");
                colors.insert(pixel.to_vec());
                crop.extend_from_slice(pixel);
            }
        }
        assert!(colors.len() >= 3, "icon must contain actual artwork");
        crops.push(crop);
    }
    assert_ne!(crops[0], crops[1]);
    assert_ne!(crops[1], crops[2]);
    assert_ne!(crops[0], crops[2]);
    assert_eq!(first, pixels(&mut host), "idle redraw preserves the scene");
    assert!(
        host.take_intent().is_none(),
        "layout messages cannot launch"
    );
}

#[test]
fn vector_tiles_preserve_configured_extent_at_both_size_limits() {
    for size in [16, 32, 48, 128] {
        let mut config = config();
        config.tile_size = size;
        let mut host = DockHost::new(&config, 1).unwrap();
        let raster = pixels(&mut host);
        let (width, height) = config.logical_size();
        assert_eq!(raster.len(), (width * height * 4) as usize);
        for (_, rect) in host.tile_layout() {
            assert_eq!(rect.width(), f64::from(size));
            assert_eq!(rect.height(), f64::from(size));
        }
        assert!(
            raster[..width as usize * 4].iter().all(|byte| *byte == 0),
            "artwork must not overflow into transparent top padding"
        );
    }
}
#[test]
fn catalog_reconciles_real_pixels_without_replacing_widgets() {
    let mut host = DockHost::new(&config(), 1).unwrap();
    let unavailable = pixels(&mut host);
    let old = host.tile_layout();
    assert_eq!(old.len(), 3);
    for (index, (_, bounds)) in old.iter().enumerate() {
        assert_eq!(bounds.x0, 8.0 + index as f64 * 54.0);
        assert_eq!(bounds.y0, 8.0);
        assert_eq!(bounds.width(), 48.0);
        assert_eq!(bounds.height(), 48.0);
    }
    host.install_catalog(1, &entries()).unwrap();
    let available = pixels(&mut host);
    assert_ne!(unavailable, available);
    assert_eq!(host.tile_layout(), old);
    let before = host.state().tiles().to_vec();
    let mut malformed = entries();
    malformed[1].1 = 3;
    assert!(host.install_catalog(2, &malformed).is_err());
    assert_eq!(host.state().tiles(), before);
    assert_eq!(pixels(&mut host), available);
    assert!(host.install_catalog(1, &entries()).is_err());
}
#[test]
fn xilem_callback_updates_state_directly_and_preserves_captured_catalog() {
    let mut host = DockHost::new(&config(), 1).unwrap();
    host.install_catalog(1, &entries()).unwrap();
    host.scene();
    let buttons = host.tile_layout();
    let click = || DynMessage(Box::new(ButtonPress { button: None }));
    assert!(host.dispatch_widget_action(buttons[2].0, click()));
    assert!(host.take_intent().is_none(), "unavailable tile");
    assert!(host.dispatch_widget_action(buttons[0].0, click()));
    // Reconciliation does not rewrite an already emitted intent's origin.
    let mut next = entries();
    next[0].1 = 4;
    host.install_catalog(2, &next).unwrap();
    let old = host.take_intent().unwrap();
    assert_eq!(old.identity, "registered:terminal");
    assert_eq!(old.catalog_generation, 1);
    assert_eq!(old.slot, 3);
    host.scene();
    assert_eq!(host.tile_layout(), buttons);
    assert!(host.dispatch_widget_action(buttons[0].0, click()));
    let current = host.take_intent().unwrap();
    assert_eq!(current.catalog_generation, 2);
    assert_eq!(current.slot, 4);
    assert!(host.take_intent().is_none());
}

#[test]
fn full_width_allocation_centers_the_same_fractional_scale_tree_and_targets() {
    let mut host = DockHost::with_scale(&config(), 1.5).unwrap();
    host.install_catalog(1, &entries()).unwrap();
    let (mut scene, targets) = host.allocated_scene(1200, 96).unwrap();
    let widgets = host.tile_layout();
    assert_eq!(targets.len(), 3);
    let center = f64::from((1200 - (config().logical_size().0 as f64 * 1.5).ceil() as u32) / 2);
    for (index, (id, rect)) in targets.iter().enumerate() {
        assert_eq!(*id, widgets[index].0);
        assert_eq!(rect.x0, center + widgets[index].1.x0 * 1.5);
        assert_eq!(rect.width(), 72.0);
    }
    let mut raster = VelloCpuRenderer::new(1200, 96);
    let pixels = raster
        .render_source(&mut scene.scene, 1200, 96)
        .unwrap()
        .data;
    assert_eq!(
        &pixels[..4],
        &[0, 0, 0, 0],
        "outside dock remains transparent"
    );
    assert!(pixels.iter().any(|byte| *byte != 0));
    assert!(host.allocated_scene(20, 96).is_err());
    assert_eq!(
        host.tile_layout(),
        widgets,
        "refusal does not replace widgets"
    );
    assert!(DockHost::with_scale(&config(), f64::NAN).is_err());
}
