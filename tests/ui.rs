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
