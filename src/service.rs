//! Bounded persistent catalog client. Xilem owns views; this owner sequences
//! protocol effects and exact resource obligations, not a second UI reducer.
mod actions;
mod custody;
mod observe;
mod upload;

use crate::{
    config::{Config, Outputs},
    render::{GpuWorker, RenderJobId, RenderResult},
    ui::DockScene,
    views::{DockViews, ViewIdentity, ViewMetadata},
    wait::{Wait, Wake},
};
use sophia_shell_client::{
    CatalogInbox, CatalogObservation, ContentLifecycle, ProducerRoleBounds, ShellClientError,
    ShellConnection, ShellImageConfig,
};
use sophia_shell_protocol::*;
use std::{
    collections::VecDeque,
    os::fd::{AsFd, BorrowedFd},
    path::PathBuf,
    time::{Duration, Instant},
};

/// Absolute CLOCK_MONOTONIC time in `per_second` units (1_000 for
/// milliseconds, 1_000_000 for microseconds).
fn monotonic(per_second: u64) -> Result<u64, String> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid out-pointer for the duration of the call.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return Err("CLOCK_MONOTONIC unavailable".into());
    }
    (ts.tv_sec as u64)
        .checked_mul(per_second)
        .and_then(|units| units.checked_add(ts.tv_nsec as u64 / (1_000_000_000 / per_second)))
        .ok_or_else(|| "CLOCK_MONOTONIC overflow".into())
}
/// Absolute CLOCK_MONOTONIC milliseconds, paired with an `Instant`
/// taken at the same moment for the image adapter's clock domain.
fn monotonic_ms() -> Result<u64, String> {
    monotonic(1_000)
}
use upload::{Pending, Phase};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
/// Poll interval for a renderer that has a job in flight but no completion
/// notifier (test executors). Production waits are readiness-driven.
const IDLE_POLL: Duration = Duration::from_millis(4);

/// Async raster effect boundary. Production uses the admitted GPU executor;
/// private protocol tests may supply pixels without claiming GPU acceptance.
pub trait RasterExecutor {
    /// Transfer one source, or return it unchanged on refusal.
    fn submit(&mut self, id: RenderJobId, scene: Box<DockScene>) -> Result<(), Box<DockScene>>;
    /// Poll one exact completion without waiting.
    fn poll(&mut self) -> Result<Option<RenderResult>, String>;
    /// Readable when `poll` has something to report; `None` means the owner
    /// must poll an in-flight job at a bounded interval instead.
    fn completion_fd(&self) -> Option<BorrowedFd<'_>> {
        None
    }
    /// Earliest instant at which `poll` can report a deadline failure.
    fn wake_deadline(&self) -> Option<Instant> {
        None
    }
}
impl RasterExecutor for GpuWorker {
    fn submit(&mut self, id: RenderJobId, scene: Box<DockScene>) -> Result<(), Box<DockScene>> {
        self.submit(id, scene)
    }
    fn poll(&mut self) -> Result<Option<RenderResult>, String> {
        self.poll()
    }
    fn completion_fd(&self) -> Option<BorrowedFd<'_>> {
        Some(self.wake_fd())
    }
    fn wake_deadline(&self) -> Option<Instant> {
        self.wake_deadline()
    }
}
/// Cumulative saturating client work counters, shared vocabulary with Lom.
/// Nothing is counted or recorded unless the CLI enabled it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkStats {
    /// Set by the CLI opt-in; nothing counts while false.
    pub enabled: bool,
    /// `step` calls.
    pub service_turns: u64,
    /// `wait` calls.
    pub waits: u64,
    /// Waits that ended at their deadline with nothing ready.
    pub wait_expirations: u64,
    /// Scenes submitted to the renderer.
    pub renders_started: u64,
    /// Matching results consumed from the renderer.
    pub renders_completed: u64,
    /// Ring offers written.
    pub uploads: u64,
    /// Pixel bytes those offers carried.
    pub upload_bytes: u64,
    /// Candidates enqueued on already-resident pixels without a render.
    pub raster_reuses: u64,
}
impl WorkStats {
    /// Saturating increment, counted only while enabled.
    pub fn count(&mut self, counter: fn(&mut Self) -> &mut u64, by: u64) {
        if self.enabled {
            let value = counter(self);
            *value = value.saturating_add(by);
        }
    }
    /// One `sophia_shell_work_stats` record; `final_` marks the shutdown record.
    pub fn record(&self, client: &str, final_: bool) -> Result<String, String> {
        Ok(format!(
            "sophia_shell_work_stats schema=1 client={client} final={} monotonic_usec={} service_turns={} waits={} wait_expirations={} renders_started={} renders_completed={} uploads={} upload_bytes={} raster_reuses={}",
            u8::from(final_),
            monotonic(1_000_000)?,
            self.service_turns,
            self.waits,
            self.wait_expirations,
            self.renders_started,
            self.renders_completed,
            self.uploads,
            self.upload_bytes,
            self.raster_reuses,
        ))
    }
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum ResourceState {
    Free,
    Staging,
    Resident,
    RetireNeeded,
    Retiring,
}
struct Slot {
    resource: ContentResourceId,
    state: ResourceState,
    bytes: u64,
    deadline: Option<Instant>,
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            resource: ContentResourceId {
                id: 0,
                generation: 1,
            },
            state: ResourceState::Free,
            bytes: 0,
            deadline: None,
        }
    }
}
struct Output {
    facts: ContentOutputFactsEntry,
    request: Option<TransactionId>,
    allocation: Option<ContentAllocationResult>,
    slots: [Slot; 2],
    current: Option<usize>,
    pending: Option<Pending>,
    presented: Option<ViewMetadata>,
    dirty: bool,
}
struct Rendering {
    job: RenderJobId,
    output: usize,
    slot: usize,
    view: ViewMetadata,
}
struct ActionReply {
    transaction: TransactionId,
    ack: ContentActionAck,
    activation: Option<CatalogActivation>,
    sent: bool,
    deadline: Instant,
}

/// One exact connection, up to sixteen outputs, two resource slots per output
/// and one GPU job per turn. All bounds remain subordinate
/// to negotiated aggregate limits. Fatal protocol errors require disconnect;
/// this object never retries an unknown launch effect in a fresh connection.
pub struct DockService<R> {
    connection: ShellConnection,
    custody: custody::CustodyWatch,
    inbox: CatalogInbox,
    config: Config,
    views: DockViews,
    renderer: R,
    limits: Option<ContentLimits>,
    lifecycle: Option<ContentLifecycle>,
    facts: Option<ContentOutputFacts>,
    catalog_generation: u64,
    outputs: Vec<Output>,
    rendering: Option<Rendering>,
    replies: VecDeque<ActionReply>,
    allowance: u32,
    next: u64,
    candidate: u64,
    resource: u64,
    job: u64,
    cursor: usize,
    started: Instant,
    endpoint_dir: PathBuf,
    stats: WorkStats,
    /// This turn changed local state that the next turn can act on without
    /// any descriptor or deadline; the next wait returns at once.
    progress: bool,
}
impl<R: RasterExecutor> DockService<R> {
    /// Construct only after negotiation and explicit GPU admission. No socket
    /// access or resource allocation happens in this constructor.
    pub fn new(
        connection: ShellConnection,
        config: Config,
        allowance: u32,
        renderer: R,
        endpoint_dir: PathBuf,
    ) -> Result<Self, String> {
        if connection.welcome().selected_revision != 10 || allowance == 0 {
            return Err("dock requires revision 10 and an explicit edge allowance".into());
        }
        let inbox = CatalogInbox::new(connection.connection_epoch()).map_err(|e| e.to_string())?;
        Ok(Self {
            views: DockViews::new(config.clone()),
            config,
            connection,
            custody: Default::default(),
            inbox,
            renderer,
            limits: None,
            lifecycle: None,
            facts: None,
            catalog_generation: 0,
            outputs: Vec::new(),
            rendering: None,
            replies: VecDeque::new(),
            allowance,
            next: 1,
            candidate: 1,
            resource: 1,
            job: 1,
            cursor: 0,
            started: Instant::now(),
            endpoint_dir,
            stats: WorkStats::default(),
            progress: false,
        })
    }
    /// Enable cumulative work counters and their diagnostic record.
    pub fn enable_work_stats(&mut self) {
        self.stats.enabled = true;
    }
    /// Current counters; the record is `None` unless enabled.
    pub fn work_stats(&self) -> WorkStats {
        self.stats
    }
    /// The record when enabled; `final_` marks the shutdown record.
    pub fn work_stats_record(&self, final_: bool) -> Result<Option<String>, String> {
        if !self.stats.enabled {
            return Ok(None);
        }
        self.stats.record("provlita", final_).map(Some)
    }
    /// Earliest owner deadline: startup publication and first presentation,
    /// every pending upload/presentation response, held pacing permits,
    /// resource retirements and action replies.
    fn next_deadline(&self) -> Option<Instant> {
        let mut earliest: Option<Instant> = None;
        let mut consider = |deadline: Instant| {
            earliest = Some(earliest.map_or(deadline, |e| e.min(deadline)));
        };
        if self.limits.is_none()
            || self.facts.is_none()
            || self.catalog_generation == 0
            || self.outputs.iter().any(|output| output.presented.is_none())
        {
            consider(self.started + RESPONSE_TIMEOUT);
        }
        for reply in &self.replies {
            consider(reply.deadline);
        }
        for output in &self.outputs {
            if let Some(pending) = &output.pending {
                consider(pending.deadline);
                if let Some(permit) = pending.permit_deadline() {
                    consider(permit);
                }
            }
            for slot in &output.slots {
                if let Some(deadline) = slot.deadline {
                    consider(deadline);
                }
            }
        }
        earliest
    }
    /// Sleep until the next turn has something to do: session or image socket
    /// readiness, GPU completion, `wake` (the stop request), or the earliest
    /// of the SDK's, the renderer's, this owner's and the caller's deadlines.
    pub fn wait(
        &mut self,
        wake: Option<BorrowedFd<'_>>,
        until: Option<Instant>,
    ) -> Result<Wake, String> {
        let (session, image) = match self
            .connection
            .image_readiness()
            .map_err(|e| e.to_string())?
        {
            Some(readiness) => (
                readiness.session,
                readiness.image_fd.map(|fd| (fd, readiness.image_events)),
            ),
            None => (
                self.connection.io_interest().map_err(|e| e.to_string())?,
                None,
            ),
        };
        // Runnable work needs no poll: the next turn proceeds at once, and
        // this is not counted as a wait.
        if session.immediate || self.progress {
            return Ok(Wake::Ready);
        }
        let mut wait = Wait::new();
        wait.events(self.connection.as_fd(), session.readable, session.writable);
        if let Some((fd, events)) = image {
            use sophia_image_producer::ReadyEvents;
            wait.events(
                fd,
                events.contains(ReadyEvents::IN),
                events.contains(ReadyEvents::OUT),
            );
        }
        match self.renderer.completion_fd() {
            Some(fd) => {
                wait.readable(fd);
            }
            None if self.rendering.is_some() => {
                wait.until(Some(Instant::now() + IDLE_POLL));
            }
            None => {}
        }
        if let Some(fd) = wake {
            wait.readable(fd);
        }
        wait.until(self.connection.wake_deadline())
            .until(self.renderer.wake_deadline())
            .until(self.next_deadline())
            .until(until);
        self.stats.count(|s| &mut s.waits, 1);
        let wake = wait.block()?;
        self.stats.count(
            |s| &mut s.wait_expirations,
            u64::from(wake == Wake::Expired),
        );
        Ok(wake)
    }
    fn tx(&mut self) -> Result<TransactionId, String> {
        let id = self.next;
        self.next = id.checked_add(1).ok_or("transaction IDs exhausted")?;
        Ok(TransactionId::from_raw(id))
    }
    fn enqueue(&mut self, tx: TransactionId, record: ShellContentRecord) -> Result<bool, String> {
        match self.enqueue_content(tx, &record) {
            Ok(()) => Ok(true),
            Err(ShellClientError::QueueSaturated) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
    /// One bounded owner turn. Its return counts actual matching Presented
    /// observations, not queue admission or GPU completion.
    pub fn step(&mut self) -> Result<usize, String> {
        self.stats.count(|s| &mut s.service_turns, 1);
        self.progress = false;
        self.custody.observe(&self.connection)?;
        self.connection.poll_io().map_err(|e| e.to_string())?;
        self.custody.observe(&self.connection)?;
        let mut presented = 0;
        for _ in 0..64 {
            let Some(observation) = self
                .connection
                .take_catalog_observation(&mut self.inbox)
                .map_err(|e| e.to_string())?
            else {
                break;
            };
            presented += self.observe(observation)?;
            self.progress = true;
        }
        // Polled every turn, job or not, so the renderer clears its wake and
        // reports an unexpected exit, before startup publication too.
        if let Some(result) = self.renderer.poll()? {
            let rendering = self
                .rendering
                .take()
                .ok_or("GPU completion without an owner")?;
            if result.job != rendering.job {
                return Err("GPU completion names another job".into());
            }
            self.stats.count(|s| &mut s.renders_completed, 1);
            self.finish_render(rendering, result)?;
            self.progress = true;
        }
        self.flush_actions()?;
        if self.limits.is_none() || self.facts.is_none() || self.catalog_generation == 0 {
            if self.started.elapsed() >= RESPONSE_TIMEOUT {
                return Err("dock startup publication timed out".into());
            }
            return Ok(presented);
        }
        self.allocate()?;
        if self.outputs.iter().any(|output| output.presented.is_none())
            && self.started.elapsed() >= RESPONSE_TIMEOUT
        {
            return Err("initial dock presentation timed out".into());
        }
        let count = self.outputs.len();
        for n in 0..count {
            let index = (self.cursor + n) % count;
            self.advance(index)?;
            self.retire(index)?;
        }
        if self.rendering.is_none() {
            for n in 0..count {
                let index = (self.cursor + n) % count;
                if self.start_render(index)? {
                    self.cursor = (index + 1) % count;
                    break;
                }
            }
        }
        Ok(presented)
    }
    fn allocate(&mut self) -> Result<(), String> {
        let limits = self.limits.as_ref().expect("limits").clone();
        let mut pending = self
            .outputs
            .iter()
            .filter(|o| o.request.is_some() && o.allocation.is_none())
            .count();
        for index in 0..self.outputs.len() {
            if self.outputs[index].request.is_some()
                || pending >= limits.max_pending_allocation_requests as usize
            {
                continue;
            }
            let tx = self.tx()?;
            let output = &self.outputs[index];
            let request = ContentAllocationRequest {
                grant: limits.grant,
                output: output.facts.output,
                allocation_request_id: index as u64 + 1,
                operation: 1,
                role: 1,
                edge: 3,
                prior: ContentAllocationId::default(),
                parent: ContentAllocationId::default(),
                parent_presentation_epoch: 0,
                anchor_parent_rect: ContentPixelRect::default(),
                desired_width: output.facts.local_width,
                desired_height: self.config.logical_size().1,
                margins: ContentMargins::default(),
            };
            if self.enqueue(tx, ShellContentRecord::AllocationRequest(request))? {
                self.outputs[index].request = Some(tx);
                pending += 1;
            }
        }
        Ok(())
    }
    fn start_render(&mut self, index: usize) -> Result<bool, String> {
        let limits = self.limits.as_ref().expect("limits");
        let output = &self.outputs[index];
        let Some(allocation) = &output.allocation else {
            return Ok(false);
        };
        if !output.dirty || output.pending.is_some() {
            return Ok(false);
        }
        // The aggregate candidate cap binds reused and fresh pixels alike.
        if self.outputs.iter().filter(|o| o.pending.is_some()).count()
            >= limits
                .max_pending_candidates_total
                .min(limits.max_open_candidates_total) as usize
        {
            return Ok(false);
        }
        let identity = ViewIdentity {
            grant: limits.grant,
            output: output.facts.output,
            allocation: allocation.allocation,
            scale_generation: allocation.scale_generation,
        };
        let view = self.views.prepare(
            identity,
            allocation.pixel.width,
            allocation.pixel.height,
            f64::from(allocation.scale_numerator) / f64::from(allocation.scale_denominator),
        )?;
        let output = &mut self.outputs[index];
        // The resident pixels already show this exact view: present them
        // again under the new catalog generation with a fresh demand/permit
        // and no render, upload or slot change.
        if let Some(current) = output.current
            && output.slots[current].state == ResourceState::Resident
            && output
                .presented
                .as_ref()
                .is_some_and(|presented| presented.same_pixels(&view.metadata))
        {
            output.dirty = false;
            output.pending = Some(Pending::new(
                current,
                view.metadata,
                Vec::new(),
                Phase::Demand,
            ));
            self.progress = true;
            return Ok(false);
        }
        let Some(slot) = output
            .slots
            .iter()
            .position(|slot| slot.state == ResourceState::Free)
        else {
            return Ok(false);
        };
        let output = &self.outputs[index];
        let allocation = output.allocation.as_ref().expect("checked allocation");
        let bytes = u64::from(allocation.pixel.width) * u64::from(allocation.pixel.height) * 4;
        if bytes == 0
            || bytes > limits.max_resource_bytes
            || bytes > limits.max_staging_bytes
            || bytes > limits.max_resident_bytes
            || bytes > limits.max_retiring_bytes
            || allocation.pixel.width > limits.max_width_px
            || allocation.pixel.height > limits.max_height_px
        {
            return Err("dock raster exceeds the negotiated resource size".into());
        }
        let all = || self.outputs.iter().flat_map(|o| &o.slots);
        let staging: u64 = all()
            .filter(|s| s.state == ResourceState::Staging)
            .map(|s| s.bytes)
            .sum();
        let resident: u64 = all()
            .filter(|s| {
                matches!(
                    s.state,
                    ResourceState::Staging | ResourceState::Resident | ResourceState::RetireNeeded
                )
            })
            .map(|s| s.bytes)
            .sum();
        if staging + bytes > limits.max_staging_bytes
            || resident + bytes > limits.max_resident_bytes
            || all().filter(|s| s.state != ResourceState::Free).count()
                >= limits.max_live_resources as usize
            || all().filter(|s| s.state == ResourceState::Staging).count()
                >= limits.max_open_transfers as usize
            || (output.slots[slot].resource.id == 0
                && self.resource > u64::from(limits.max_resource_ids))
        {
            return Ok(false);
        }
        let job = RenderJobId(self.job);
        let next = self.job.checked_add(1).ok_or("render job IDs exhausted")?;
        if self.renderer.submit(job, view.scene).is_err() {
            return Err("idle renderer refused owned scene".into());
        }
        self.job = next;
        self.stats.count(|s| &mut s.renders_started, 1);
        let output = &mut self.outputs[index];
        output.dirty = false;
        output.slots[slot].state = ResourceState::Staging;
        output.slots[slot].bytes = bytes;
        self.rendering = Some(Rendering {
            job,
            output: index,
            slot,
            view: view.metadata,
        });
        Ok(true)
    }
    /// Return the renderer for explicit shutdown after dropping this connection.
    /// Disconnect settles no server resource by itself; Session owns that drain.
    pub fn into_renderer(self) -> R {
        self.renderer
    }
    /// Requested stop: take no further turn, close the connection and return
    /// the renderer for bounded shutdown. Outstanding obligations are reported,
    /// never retried; a sent activation's launch outcome remains with Session.
    pub fn stop(self) -> (StopReport, R) {
        let report = StopReport {
            sent_activations: self.replies.iter().filter(|reply| reply.sent).count(),
            unsent_replies: self.replies.iter().filter(|reply| !reply.sent).count(),
            unsettled_submissions: self.custody.unsettled(),
            rendering: self.rendering.is_some(),
        };
        drop(self.connection);
        (report, self.renderer)
    }
}

/// Obligations still open when a requested stop closed the connection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StopReport {
    /// Activations written to Session whose launch outcome is unknown here.
    pub sent_activations: usize,
    /// Action replies never admitted; Session saw no activation for them.
    pub unsent_replies: usize,
    /// Admitted submissions not yet observed as Submitted or Stored.
    pub unsettled_submissions: usize,
    /// A GPU job was in flight; its pixels are discarded at shutdown.
    pub rendering: bool,
}
