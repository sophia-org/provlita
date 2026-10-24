use super::*;

fn evidence() -> GpuAdmissionEvidence {
    GpuAdmissionEvidence {
        grant_epoch: 1,
        render_node: "/dev/dri/renderD128".into(),
        device_major: 226,
        device_minor: 128,
        pci_bus_id: None,
        pci_vendor_id: None,
        pci_device_id: None,
        adapter_render_major: 226,
        adapter_render_minor: 128,
        visible_dri_entries: vec!["renderD128".into()],
        adapter: wgpu::AdapterInfo {
            name: "fixture".into(),
            vendor: 0,
            device: 0,
            device_type: wgpu::DeviceType::DiscreteGpu,
            device_pci_bus_id: String::new(),
            driver: String::new(),
            driver_info: String::new(),
            backend: wgpu::Backend::Vulkan,
            subgroup_min_size: 1,
            subgroup_max_size: 1,
            transient_saves_memory: false,
        },
    }
}
fn scene() -> Box<DockScene> {
    Box::new(DockScene {
        scene: masonry::imaging::record::Scene::new(),
        width: 4,
        height: 2,
    })
}
fn wait(mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(1));
        std::thread::yield_now();
    }
}
fn ready(worker: &mut GpuWorker) {
    wait(|| worker.poll_ready().unwrap().is_some());
}
fn shutdown(worker: &mut GpuWorker) {
    worker.request_shutdown();
    wait(|| worker.poll_shutdown().unwrap());
}

#[test]
fn actual_worker_owns_one_correlated_scene_and_returns_refused_source() {
    let (release, hold) = mpsc::channel();
    let mut worker = GpuWorker::spawn(move || {
        Ok((
            Box::new(move |_| {
                hold.recv_timeout(Duration::from_secs(1)).unwrap();
                Ok(vec![7; 32])
            }),
            evidence(),
        ))
    })
    .unwrap();
    ready(&mut worker);
    assert!(worker.submit(RenderJobId(1), scene()).is_ok());
    let refused = worker.submit(RenderJobId(2), scene()).err().unwrap();
    assert_eq!((refused.width, refused.height), (4, 2));
    assert!(
        worker.poll().unwrap().is_none(),
        "pending GPU never blocks the owner"
    );
    let config =
        crate::config::Config::parse(include_str!("../../examples/minimal/config.kdl")).unwrap();
    let mut host = crate::ui::DockHost::new(&config, 1).unwrap();
    host.install_catalog(1, &[("registered:terminal".into(), 2, true)])
        .unwrap();
    host.scene();
    let widget = host.tile_layout()[0].0;
    assert!(host.dispatch_widget_action(
        widget,
        xilem_masonry::core::DynMessage(Box::new(masonry::widgets::ButtonPress { button: None }))
    ));
    assert_eq!(
        host.take_intent().unwrap().slot,
        2,
        "real Xilem callback progresses before GPU completion"
    );
    release.send(()).unwrap();
    wait(|| match worker.poll().unwrap() {
        Some(result) => {
            assert_eq!(result.job, RenderJobId(1));
            assert_eq!(result.bytes, vec![7; 32]);
            true
        }
        None => false,
    });
    assert!(
        worker.submit(RenderJobId(1), scene()).is_err(),
        "no job ID reuse"
    );
    shutdown(&mut worker);
    assert!(worker.submit(RenderJobId(2), scene()).is_err());
}

#[test]
fn expired_job_retains_identity_and_late_result_cannot_revive_worker() {
    let (release, hold) = mpsc::channel();
    let mut worker = GpuWorker::spawn(move || {
        Ok((
            Box::new(move |_| {
                hold.recv_timeout(Duration::from_secs(1)).unwrap();
                Ok(vec![0; 32])
            }),
            evidence(),
        ))
    })
    .unwrap();
    ready(&mut worker);
    assert!(worker.submit(RenderJobId(1), scene()).is_ok());
    worker.pending.as_mut().unwrap().1 = Instant::now() - DEADLINE;
    assert!(worker.poll().is_err());
    assert_eq!(worker.pending.unwrap().0, RenderJobId(1));
    release.send(()).unwrap();
    assert!(worker.poll().is_err());
    assert!(worker.submit(RenderJobId(2), scene()).is_err());
    shutdown(&mut worker);
}

#[test]
fn failed_startup_and_wrong_sized_result_are_not_success() {
    let mut failed = GpuWorker::spawn(|| Err("fixture startup failure".into())).unwrap();
    wait(|| failed.poll_ready().is_err());
    assert!(failed.submit(RenderJobId(1), scene()).is_err());
    shutdown(&mut failed);
    let custody = std::sync::Arc::new(());
    let owner = custody.clone();
    let mut malformed = GpuWorker::spawn(move || {
        Ok((
            Box::new(move |_| {
                let _retained = &owner;
                Ok(vec![0; 4])
            }),
            evidence(),
        ))
    })
    .unwrap();
    ready(&mut malformed);
    assert!(malformed.submit(RenderJobId(1), scene()).is_ok());
    wait(|| malformed.poll().is_err());
    assert_eq!(
        std::sync::Arc::strong_count(&custody),
        2,
        "failed rasterizer remains owned"
    );
    assert!(malformed.submit(RenderJobId(2), scene()).is_err());
    shutdown(&mut malformed);
    assert_eq!(std::sync::Arc::strong_count(&custody), 1);
}

#[test]
fn shutdown_does_not_claim_pending_worker_has_exited() {
    let (release, hold) = mpsc::channel();
    let mut worker = GpuWorker::spawn(move || {
        Ok((
            Box::new(move |_| {
                hold.recv_timeout(Duration::from_secs(1)).unwrap();
                Ok(vec![0; 32])
            }),
            evidence(),
        ))
    })
    .unwrap();
    ready(&mut worker);
    assert!(worker.submit(RenderJobId(1), scene()).is_ok());
    worker.request_shutdown();
    assert!(!worker.poll_shutdown().unwrap());
    assert!(worker.pending.is_some());
    release.send(()).unwrap();
    wait(|| worker.poll_shutdown().unwrap());
    assert!(worker.pending.is_none());
}

fn readable(worker: &GpuWorker) -> bool {
    crate::wait::Wait::new()
        .readable(worker.wake_fd())
        .until(Some(Instant::now()))
        .block()
        .unwrap()
        == crate::wait::Wake::Ready
}
/// A rasterizer that reports when it starts a job, then waits for release.
/// Once a job has started, the admission wake is certainly written, so the
/// owner's next poll consumes it and the following wake is the job's own.
fn gated(
    outcome: impl Fn() -> Result<Vec<u8>, String> + Send + 'static,
) -> (mpsc::Sender<()>, mpsc::Receiver<()>, GpuWorker) {
    let (release, hold) = mpsc::channel();
    let (started, running) = mpsc::channel();
    let worker = GpuWorker::spawn(move || {
        Ok((
            Box::new(move |_| {
                started.send(()).unwrap();
                hold.recv_timeout(Duration::from_secs(1)).unwrap();
                outcome()
            }),
            evidence(),
        ))
    })
    .unwrap();
    (release, running, worker)
}
fn job_started(worker: &mut GpuWorker, running: &mpsc::Receiver<()>) {
    ready(worker);
    assert!(worker.submit(RenderJobId(1), scene()).is_ok());
    running.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(worker.poll().unwrap().is_none(), "job in flight");
    assert!(!readable(worker), "admission wake consumed");
}
fn woken(worker: &GpuWorker) {
    assert_eq!(
        crate::wait::Wait::new()
            .readable(worker.wake_fd())
            .until(Some(Instant::now() + Duration::from_secs(1)))
            .block()
            .unwrap(),
        crate::wait::Wake::Ready,
        "wake within bound"
    );
}

/// Admission and completion set the wake; the next poll clears it before
/// draining, and a stale wake with no job in flight is consumed, not spun on.
#[test]
fn wakes_are_cleared_before_each_drain_and_a_stale_wake_does_not_persist() {
    let (release, running, mut worker) = gated(|| Ok(vec![7; 32]));
    job_started(&mut worker, &running);
    release.send(()).unwrap();
    woken(&worker);
    assert!(worker.poll().unwrap().is_some());
    assert!(!readable(&worker), "completion wake consumed");
    crate::wait::signal(worker.wake_fd().as_raw_fd());
    assert!(worker.poll().unwrap().is_none(), "stale wake, no job");
    assert!(!readable(&worker), "stale wake consumed");
    shutdown(&mut worker);
}

/// The result channel disconnects before the exit wake, so the first poll
/// after the wake reports the failure instead of an empty channel.
#[test]
fn an_unwinding_rasterizer_is_disconnected_before_its_exit_wake() {
    let (release, running, mut worker) = gated(|| panic!("fixture raster unwind"));
    job_started(&mut worker, &running);
    release.send(()).unwrap();
    woken(&worker);
    assert!(worker.poll().is_err(), "disconnect visible at the wake");
    worker.request_shutdown();
    wait(|| worker.poll_shutdown().is_err());
}

/// A startup failure is announced before the owner can poll it, and an exit
/// with no job in flight is a failure, not idleness.
#[test]
fn startup_failure_and_unrequested_exit_are_announced_and_fatal() {
    let mut failed = GpuWorker::spawn(|| Err("fixture startup failure".into())).unwrap();
    woken(&failed);
    assert!(failed.poll_ready().is_err());
    shutdown(&mut failed);
    let (release, running, mut quit) = gated(|| Err("fixture raster failure".into()));
    job_started(&mut quit, &running);
    release.send(()).unwrap();
    woken(&quit);
    assert!(quit.poll().is_err(), "failure visible at the wake");
    shutdown(&mut quit);
}
