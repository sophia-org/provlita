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
};
use sophia_shell_client::{
    CatalogInbox, CatalogObservation, ContentLifecycle, ShellClientError, ShellConnection,
};
use sophia_shell_protocol::*;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use upload::{Pending, Phase};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_POLL: Duration = Duration::from_millis(4);

/// Async raster effect boundary. Production uses the admitted GPU executor;
/// private protocol tests may supply pixels without claiming GPU acceptance.
pub trait RasterExecutor {
    /// Transfer one source, or return it unchanged on refusal.
    fn submit(&mut self, id: RenderJobId, scene: Box<DockScene>) -> Result<(), Box<DockScene>>;
    /// Poll one exact completion without waiting.
    fn poll(&mut self) -> Result<Option<RenderResult>, String>;
}
impl RasterExecutor for GpuWorker {
    fn submit(&mut self, id: RenderJobId, scene: Box<DockScene>) -> Result<(), Box<DockScene>> {
        self.submit(id, scene)
    }
    fn poll(&mut self) -> Result<Option<RenderResult>, String> {
        self.poll()
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

/// One exact connection, up to sixteen outputs, two resource slots per output,
/// one GPU job and four upload chunks per turn. All bounds remain subordinate
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
}
impl<R: RasterExecutor> DockService<R> {
    /// Construct only after negotiation and explicit GPU admission. No socket
    /// access or resource allocation happens in this constructor.
    pub fn new(
        connection: ShellConnection,
        config: Config,
        allowance: u32,
        renderer: R,
    ) -> Result<Self, String> {
        if connection.welcome().selected_revision != 8 || allowance == 0 {
            return Err("dock requires revision 8 and an explicit edge allowance".into());
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
        })
    }
    /// Maximum idle sleep before the next turn: the SDK's retry or readiness
    /// deadline when one is due sooner, otherwise the bounded poll interval.
    pub fn idle_wait(&self) -> Duration {
        self.connection
            .wake_deadline()
            .map_or(IDLE_POLL, |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(IDLE_POLL)
            })
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
        if let Some(rendering) = &self.rendering
            && let Some(result) = self.renderer.poll()?
        {
            if result.job != rendering.job {
                return Err("GPU completion names another job".into());
            }
            let rendering = self.rendering.take().expect("checked rendering owner");
            self.finish_render(rendering, result)?;
        }
        let mut chunks = 4;
        let count = self.outputs.len();
        for n in 0..count {
            let index = (self.cursor + n) % count;
            self.advance(index, &mut chunks)?;
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
        let Some(slot) = output
            .slots
            .iter()
            .position(|slot| slot.state == ResourceState::Free)
        else {
            return Ok(false);
        };
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
            || self.outputs.iter().filter(|o| o.pending.is_some()).count()
                >= limits
                    .max_pending_candidates_total
                    .min(limits.max_open_candidates_total) as usize
            || (output.slots[slot].resource.id == 0
                && self.resource > u64::from(limits.max_resource_ids))
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
        let job = RenderJobId(self.job);
        let next = self.job.checked_add(1).ok_or("render job IDs exhausted")?;
        if self.renderer.submit(job, view.scene).is_err() {
            return Err("idle renderer refused owned scene".into());
        }
        self.job = next;
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
