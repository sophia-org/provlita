//! One owned GPU job; the caller retains Xilem and protocol ownership.
use super::{DockRenderer, GpuAdmissionEvidence, GpuGrant};
use crate::ui::DockScene;
use std::{
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(2);

/// Signals the owner's wake counter when dropped, after every other local.
struct WakeOnDrop(Arc<OwnedFd>);
impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        crate::wait::signal(self.0.as_raw_fd());
    }
}

/// Opaque checked service job identity, separate from candidate/native epochs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderJobId(pub u64);

/// Pixels retain their originating request; callers must reject superseded work.
pub struct RenderResult {
    /// Original submitted job.
    pub job: RenderJobId,
    /// Packed straight-alpha RGBA raster from the shared GPU adapter.
    pub bytes: Vec<u8>,
    /// Raster width.
    pub width: u32,
    /// Raster height.
    pub height: u32,
}
struct Job {
    id: RenderJobId,
    scene: Box<DockScene>,
}
type Raster = Box<dyn FnMut(&mut DockScene) -> Result<Vec<u8>, String> + Send>;

/// Capacity-one executor. Startup and completion are polled, never awaited on
/// the protocol thread. Timeout poisons this owner; it does not release a GPU
/// job or authorize reuse. Shutdown joins only a finished thread. Dropping an
/// unfinished owner detaches the thread and is not evidence of clean shutdown.
pub struct GpuWorker {
    requests: Option<SyncSender<Job>>,
    ready: Receiver<Result<GpuAdmissionEvidence, String>>,
    results: Receiver<Result<RenderResult, String>>,
    thread: Option<JoinHandle<()>>,
    started: Instant,
    admitted: bool,
    pending: Option<(RenderJobId, Instant)>,
    high_water: u64,
    failed: bool,
    /// Readable after admission, each completion and thread exit.
    wake: Arc<OwnedFd>,
}
impl GpuWorker {
    /// Start explicit GPU admission; no device work happens on the caller.
    pub fn start(grant: GpuGrant) -> Result<Self, String> {
        Self::spawn(move || {
            let (mut renderer, evidence) = DockRenderer::new(&grant)?;
            Ok((
                Box::new(move |scene: &mut DockScene| renderer.render(scene)) as Raster,
                evidence,
            ))
        })
    }
    fn spawn(
        factory: impl FnOnce() -> Result<(Raster, GpuAdmissionEvidence), String> + Send + 'static,
    ) -> Result<Self, String> {
        let (requests, incoming) = mpsc::sync_channel::<Job>(1);
        let (ready, readiness) = mpsc::sync_channel(1);
        let (results, completed) = mpsc::sync_channel(1);
        let wake = Arc::new(crate::wait::eventfd()?);
        let exit = WakeOnDrop(wake.clone());
        let thread = std::thread::Builder::new()
            .name("provlita-gpu".into())
            .spawn(move || {
                // Declared first so it drops last: the exit wake follows the
                // rasterizer's own teardown.
                let exit = exit;
                // Bound the senders after `exit` so they drop (disconnecting
                // the owner's receivers) before the exit wake, on unwind too.
                let (ready, results) = (ready, results);
                let mut raster = match factory() {
                    Ok((raster, evidence)) => {
                        if ready.send(Ok(evidence)).is_err() {
                            return;
                        }
                        crate::wait::signal(exit.0.as_raw_fd());
                        raster
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                while let Ok(mut job) = incoming.recv() {
                    let result = raster(&mut job.scene).and_then(|bytes| {
                        let expected = u64::from(job.scene.width) * u64::from(job.scene.height) * 4;
                        if bytes.len() as u64 != expected {
                            return Err("GPU raster size mismatch".into());
                        }
                        Ok(RenderResult {
                            job: job.id,
                            bytes,
                            width: job.scene.width,
                            height: job.scene.height,
                        })
                    });
                    let failed = result.is_err();
                    if results.send(result).is_err() {
                        break;
                    }
                    crate::wait::signal(exit.0.as_raw_fd());
                    if failed {
                        // Keep the failed rasterizer (including a timed-out
                        // readback's real resources) quarantined until explicit
                        // shutdown disconnects the producer. No retry is run.
                        while incoming.recv().is_ok() {}
                        break;
                    }
                }
            })
            .map_err(|e| format!("start dock GPU worker: {e}"))?;
        Ok(Self {
            requests: Some(requests),
            ready: readiness,
            results: completed,
            thread: Some(thread),
            started: Instant::now(),
            admitted: false,
            pending: None,
            high_water: 0,
            failed: false,
            wake,
        })
    }
    /// Readable when admission, a completion or thread exit awaits a poll.
    /// `poll_ready` and `poll` clear it before draining; `poll_shutdown` never
    /// does, so a join wait returns until the thread is observed finished.
    pub fn wake_fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }
    /// Earliest instant at which a poll reports a deadline failure.
    pub fn wake_deadline(&self) -> Option<Instant> {
        if self.failed {
            return None;
        }
        if !self.admitted {
            return Some(self.started + DEADLINE);
        }
        self.pending.map(|(_, started)| started + DEADLINE)
    }
    /// Poll exact startup evidence once. Deadline refusal stays sticky.
    pub fn poll_ready(&mut self) -> Result<Option<GpuAdmissionEvidence>, String> {
        if self.failed {
            return Err("GPU worker is quarantined".into());
        }
        if self.admitted {
            return Ok(None);
        }
        crate::wait::clear(self.wake.as_fd());
        if self.started.elapsed() >= DEADLINE {
            self.failed = true;
            return Err("GPU startup deadline expired".into());
        }
        match self.ready.try_recv() {
            Ok(Ok(evidence)) => {
                self.admitted = true;
                Ok(Some(evidence))
            }
            Ok(Err(error)) => {
                self.failed = true;
                Err(error)
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.failed = true;
                Err("GPU worker stopped before admission".into())
            }
        }
    }
    /// Transfer one scene. A returned refusal gives the exact source back.
    pub fn submit(&mut self, id: RenderJobId, scene: Box<DockScene>) -> Result<(), Box<DockScene>> {
        if !self.admitted
            || self.failed
            || self.pending.is_some()
            || id.0 <= self.high_water
            || scene.width == 0
            || scene.height == 0
            || scene.width > 8192
            || scene.height > 1024
            || u64::from(scene.width) * u64::from(scene.height) > 8 * 1024 * 1024
        {
            return Err(scene);
        }
        let Some(sender) = &self.requests else {
            return Err(scene);
        };
        match sender.try_send(Job { id, scene }) {
            Ok(()) => {
                self.high_water = id.0;
                self.pending = Some((id, Instant::now()));
                Ok(())
            }
            Err(mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job)) => {
                Err(job.scene)
            }
        }
    }
    /// Poll one correlated completion. A late result cannot reverse a timeout.
    pub fn poll(&mut self) -> Result<Option<RenderResult>, String> {
        if self.failed {
            return Err("GPU worker is quarantined".into());
        }
        // Clear before draining, also with no job: a completion or admission
        // wake that landed after its message was consumed must not stay
        // readable, and an unrequested exit must be observed.
        crate::wait::clear(self.wake.as_fd());
        let Some((id, started)) = self.pending else {
            return match self.results.try_recv() {
                Err(TryRecvError::Empty) => Ok(None),
                Err(TryRecvError::Disconnected) => {
                    self.failed = true;
                    Err("GPU worker exited without a shutdown request".into())
                }
                Ok(_) => {
                    self.failed = true;
                    Err("GPU completion without a pending job".into())
                }
            };
        };
        if started.elapsed() >= DEADLINE {
            self.failed = true;
            return Err("GPU job deadline expired; owner retained".into());
        }
        match self.results.try_recv() {
            Ok(Ok(result)) if result.job == id => {
                self.pending = None;
                Ok(Some(result))
            }
            Err(TryRecvError::Empty) => Ok(None),
            result => {
                self.failed = true;
                Err(match result {
                    Ok(Err(error)) => error,
                    _ => "GPU completion identity or worker failure".into(),
                })
            }
        }
    }
    /// Stop further work. In-flight GPU ownership remains with the worker.
    pub fn request_shutdown(&mut self) {
        self.requests = None;
    }
    /// Join only once finished. This establishes thread exit, not server credit
    /// reclamation or native presentation. Pending output is discarded only here.
    pub fn poll_shutdown(&mut self) -> Result<bool, String> {
        if self.requests.is_some() {
            return Err("GPU shutdown was not requested".into());
        }
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            return Ok(false);
        }
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| "GPU worker panicked")?;
        }
        while self.results.try_recv().is_ok() {}
        self.pending = None;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "../../tests/support/worker.rs"]
mod tests;
