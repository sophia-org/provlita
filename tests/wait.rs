//! The production owner wait against the private 9P fixture: readiness,
//! deadlines, the notifier-less renderer fallback and a racing stop request.
//! Own binary: the stop request is process state and must start unset here.
#[path = "support/files_wire.rs"]
mod files_wire;
#[path = "support/service_host.rs"]
#[allow(dead_code)]
mod host;
use host::*;
use provlita::{
    stop::{StopRequests, StopSignal},
    wait::{Wait, Wake},
};
use std::time::{Duration, Instant};

#[test]
fn an_idle_service_sleeps_to_its_deadline_and_wakes_on_session_bytes() {
    let (mut service, mut peer) = pair();
    drive(&mut service, &mut peer, |p| p.presented.len() == 2);
    for _ in 0..3 {
        service.step().unwrap();
        peer.pump();
    }
    let start = Instant::now();
    let wake = service
        .wait(None, Some(start + Duration::from_millis(40)))
        .unwrap();
    assert_eq!(wake, Wake::Expired);
    assert!(start.elapsed() >= Duration::from_millis(40));
    // The peer answers the client's standing events read with the catalog.
    peer.catalog(2);
    peer.pump();
    let start = Instant::now();
    let wake = service
        .wait(None, Some(start + Duration::from_secs(2)))
        .unwrap();
    assert_eq!(wake, Wake::Ready);
    assert!(start.elapsed() < Duration::from_millis(500));
}

/// The fixture renderer has no completion descriptor, so an in-flight job
/// bounds the wait by the poll interval instead of blocking on readiness.
#[test]
fn a_pending_job_without_a_notifier_bounds_the_wait() {
    let (mut service, mut peer) = pair();
    service.enable_work_stats();
    drive(&mut service, &mut peer, |p| p.presented.len() == 2);
    STALL.set(true);
    let before = service.work_stats();
    peer.catalog_with(2, true);
    let start = Instant::now();
    while service.work_stats().renders_started == before.renders_started {
        service.step().unwrap();
        peer.pump();
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    // A quiet turn: nothing observed, no result, so no local progress.
    service.step().unwrap();
    peer.pump();
    assert_eq!(
        service.work_stats().renders_completed,
        before.renders_completed
    );
    let start = Instant::now();
    let waits = service.work_stats().waits;
    assert_eq!(service.wait(None, None).unwrap(), Wake::Expired);
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(service.work_stats().waits, waits + 1);
    STALL.set(false);
    drive(&mut service, &mut peer, |p| {
        p.presented
            .values()
            .all(|(_, begin, _)| begin.catalog_generation == 2)
    });
    assert_eq!(
        service.work_stats().renders_completed,
        before.renders_completed + 2
    );
}

#[test]
fn a_stop_request_during_a_blocked_wait_is_observed_at_once_and_stays_readable() {
    let stop = StopRequests::install().unwrap();
    assert_eq!(stop.requested(), None);
    let (mut service, mut peer) = pair();
    drive(&mut service, &mut peer, |p| p.presented.len() == 2);
    for _ in 0..3 {
        service.step().unwrap();
        peer.pump();
    }
    // Thread-directed, so sibling tests' waits in this binary stay untouched.
    let waiter = unsafe { libc::pthread_self() };
    let sender = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(unsafe { libc::pthread_kill(waiter, libc::SIGTERM) }, 0);
    });
    let start = Instant::now();
    let wake = service
        .wait(Some(stop.wake_fd()), Some(start + Duration::from_secs(5)))
        .unwrap();
    assert!(matches!(wake, Wake::Ready | Wake::Interrupted), "{wake:?}");
    assert!(start.elapsed() < Duration::from_secs(2));
    sender.join().unwrap();
    assert_eq!(stop.requested(), Some(StopSignal::Terminate));
    // Never cleared: every later wait returns at once.
    let start = Instant::now();
    assert_eq!(
        service.wait(Some(stop.wake_fd()), None).unwrap(),
        Wake::Ready
    );
    assert!(start.elapsed() < Duration::from_millis(100));
}

#[test]
fn a_wait_with_nothing_to_wake_it_is_refused() {
    assert!(Wait::new().block().is_err());
    let start = Instant::now();
    assert_eq!(
        Wait::new()
            .until(Some(start + Duration::from_millis(5)))
            .block()
            .unwrap(),
        Wake::Expired
    );
}
