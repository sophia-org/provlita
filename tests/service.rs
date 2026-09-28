//! Production client scheduler over the private 9P file contract with
//! simulated native outcomes.
#[path = "support/files_wire.rs"]
mod files_wire;
#[path = "support/service_host.rs"]
mod host;
use host::*;
use provlita::{
    service::StopReport,
    stop::{StopRequests, StopSignal},
};
use sophia_shell_protocol::*;

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

#[test]
fn refused_submission_is_fatal_and_never_replayed() {
    let (mut service, mut peer) = pair();
    peer.refuse = Some(shell_files::ShellFileKind::FrameDemand);
    let start = std::time::Instant::now();
    let error = loop {
        if let Err(error) = service.step() {
            break error;
        }
        peer.pump();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "refusal was not observed"
        );
        std::thread::yield_now();
    };
    assert!(error.contains("refused"), "{error}");
    assert!(peer.presented.is_empty(), "no candidate follows a refusal");
}

#[test]
fn candidate_over_the_permit_budget_is_refused_before_submission() {
    let (mut service, mut peer) = pair();
    // Header fields and one surface/placement row exceed 100 bytes natively.
    peer.permit_bytes = 100;
    let start = std::time::Instant::now();
    let error = loop {
        if let Err(error) = service.step() {
            break error;
        }
        peer.pump();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "budget refusal was not observed"
        );
        std::thread::yield_now();
    };
    assert_eq!(error, "dock candidate exceeds negotiated byte budget");
    assert!(peer.presented.is_empty(), "no candidate reached the peer");
}

#[test]
fn sigterm_closes_the_connection_and_reports_an_unresolved_launch_without_replay() {
    let stop = StopRequests::install().unwrap();
    let (mut service, mut peer) = pair();
    drive(&mut service, &mut peer, |p| p.presented.len() == 2);
    peer.hold_outcomes = true;
    peer.action(1, 1, 1);
    drive(&mut service, &mut peer, |p| p.actions.len() == 1);
    // Settle the reply's file custody so only the launch outcome stays open.
    for _ in 0..3 {
        peer.pump();
        service.step().unwrap();
    }
    // A real process-directed signal; the handler only records it.
    assert_eq!(unsafe { libc::raise(libc::SIGTERM) }, 0);
    assert_eq!(stop.requested(), Some(StopSignal::Terminate));
    let (report, _raster) = service.stop();
    assert_eq!(
        report,
        StopReport {
            sent_activations: 1,
            unsent_replies: 0,
            unsettled_submissions: 0,
            rendering: false,
        }
    );
    assert!(peer.disconnected_within(std::time::Duration::from_secs(2)));
    // The one activation Session received is the only one: nothing replayed.
    assert_eq!(peer.actions.len(), 1);
}
