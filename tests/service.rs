//! Production client scheduler over private sockets with simulated native outcomes.
#[path = "support/service_host.rs"]
mod host;
use host::*;
use sophia_protocol::*;

#[test]
fn two_outputs_present_and_exact_xilem_clicks_reply_once_without_a_gpu() {
    let (mut service, mut peer) = pair();
    drive(&mut service, &mut peer, |p| p.presented.len() == 2);
    assert_eq!(peer.begins, 2);
    assert_ne!(
        peer.presented[&1].1.content.candidate_generation,
        peer.presented[&2].1.content.candidate_generation
    );
    let first = peer.action(1, 1, 1);
    drive(&mut service, &mut peer, |p| p.actions.len() == 1);
    peer.action(2, 2, 1);
    drive(&mut service, &mut peer, |p| p.actions.len() == 2);
    assert_eq!(peer.actions[0].action, first);
    peer.send(
        TransactionId::from_raw(2999),
        ShellContentRecord::Action(first),
    );
    drive(&mut service, &mut peer, |p| p.acks.len() == 3);
    assert_eq!(peer.actions.len(), 2);
    assert_eq!(peer.acks[2].disposition, 2);
    // Idling does not create extra jobs/uploads on either output.
    for _ in 0..20 {
        service.step().unwrap();
        peer.pump();
    }
    assert_eq!(peer.begins, 2);
}

#[test]
fn withheld_old_release_keeps_actions_live_and_exact_release_enables_reuse() {
    let (mut service, mut peer) = pair();
    drive(&mut service, &mut peer, |p| p.presented.len() == 2);
    let first_resource = peer.presented[&1].2.placements[0].resource;
    peer.hold_releases = true;
    peer.catalog(2);
    drive(&mut service, &mut peer, |p| p.releases.len() == 2);
    assert_eq!(peer.begins, 4);
    peer.action(1, 1, 1);
    drive(&mut service, &mut peer, |p| p.actions.len() == 1);
    peer.catalog(3);
    for _ in 0..20 {
        service.step().unwrap();
        peer.pump();
    }
    assert_eq!(peer.begins, 4, "both second slots remain retained");
    peer.release_one(first_resource);
    drive(&mut service, &mut peer, |p| {
        p.presented[&1].1.catalog_generation == 3
    });
    assert_eq!(peer.presented[&2].1.catalog_generation, 2);
    assert_eq!(
        peer.begins, 5,
        "one output progresses while its neighbor retains an old slot"
    );
    assert_eq!(
        peer.presented[&1].2.placements[0].resource.id,
        first_resource.id
    );
    assert_eq!(
        peer.presented[&1].2.placements[0].resource.generation,
        first_resource.generation + 1
    );
    peer.release_all();
    drive(&mut service, &mut peer, |p| {
        p.presented
            .values()
            .all(|(_, b, _)| b.catalog_generation == 3)
    });
    assert_eq!(peer.begins, 6);
}
