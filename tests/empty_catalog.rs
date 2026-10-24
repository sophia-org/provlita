//! The first catalog a real Session publishes to a dock can be empty
//! (`sophia_shell_component_catalog ... entries=0`): no pin resolves. This
//! drives the production service over the private 9P fixture with that exact
//! shape, one 1920x1080-class output pair, thickness 64 and the shipped
//! minimal configuration, and records what the client does first. No GPU,
//! Session or native presentation is involved.
#[path = "support/files_wire.rs"]
mod files_wire;
#[path = "support/service_host.rs"]
#[allow(dead_code)]
mod host;
use host::*;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

/// Steps until the service fails, both outputs present, or `limit` passes.
fn first_outcome(
    service: &mut provlita::service::DockService<Raster>,
    peer: &mut Peer,
    limit: Duration,
) -> Result<usize, String> {
    let start = Instant::now();
    loop {
        service.step()?;
        peer.pump();
        if peer.presented.len() == 2 || start.elapsed() >= limit {
            return Ok(peer.presented.len());
        }
        service
            .wait(None, Some(Instant::now() + Duration::from_millis(20)))
            .unwrap();
    }
}

#[test]
fn an_empty_first_catalog_is_installed_and_the_dock_presents_unavailable_pins() {
    let (mut service, mut peer) = pair_custom(1920, 1080, None, false);
    peer.catalog_entries(1, Vec::new(), BTreeMap::new());
    let outcome = first_outcome(&mut service, &mut peer, Duration::from_secs(3));
    assert_eq!(outcome, Ok(2), "first outcome with an empty catalog");
    assert_eq!(peer.begins, 2);
    for (_, begin, _) in peer.presented.values() {
        assert_eq!(begin.catalog_generation, 1);
    }
}

#[test]
fn a_catalog_whose_pins_are_all_unavailable_still_presents() {
    use sophia_shell_protocol::ShellApplicationDescriptor;
    let (mut service, mut peer) = pair_custom(1920, 1080, None, false);
    peer.catalog_entries(
        1,
        vec![ShellApplicationDescriptor {
            slot: 1,
            available: false,
            label: "Terminal".into(),
            keywords: String::new(),
        }],
        BTreeMap::from([(1, "registered:terminal".into())]),
    );
    let outcome = first_outcome(&mut service, &mut peer, Duration::from_secs(3));
    assert_eq!(outcome, Ok(2), "first outcome with unavailable pins");
}

/// Control: with no catalog at all the service waits, then refuses at its
/// startup bound instead of presenting.
#[test]
fn no_catalog_at_all_times_out_at_the_startup_bound() {
    let (mut service, mut peer) = pair_custom(1920, 1080, None, false);
    let start = Instant::now();
    let outcome = first_outcome(&mut service, &mut peer, Duration::from_secs(7));
    assert_eq!(
        outcome,
        Err("dock startup publication timed out".into()),
        "after {:?}",
        start.elapsed()
    );
    assert!(peer.presented.is_empty());
}

/// A capture-enabled Session can advertise owned ring plus DMA-BUF
/// (source kinds 6) to a GPU-direct role and grant this client the ring
/// alone. Before sophia-desktop-sdk-rs 0706ea2 the client refused that
/// discovery outright (shell-client-work-01 static-b1: dock_failed
/// NotConnected); it must present like the ring-only case.
#[test]
fn a_mixed_owned_ring_and_dma_buf_discovery_still_presents() {
    let (mut service, mut peer) = pair_mixed_discovery(1920, 1080);
    peer.catalog_entries(1, Vec::new(), BTreeMap::new());
    let outcome = first_outcome(&mut service, &mut peer, Duration::from_secs(3));
    assert_eq!(outcome, Ok(2), "first outcome with a mixed discovery");
    assert_eq!(peer.begins, 2);
}
