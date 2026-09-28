//! Private 9P file-contract fixture. Only 9P bytes cross the socket; it
//! supplies allocation/pacing/native outcomes and does not run Session, a GPU,
//! WM execution or application processes.
use super::files_wire::{EPOCH, Submission, Wire};
use provlita::{
    config::Config,
    render::{RenderJobId, RenderResult},
    service::{DockService, RasterExecutor},
    ui::DockScene,
};
use sophia_shell_client::{ShellClientOptions, ShellConnection};
use sophia_shell_protocol::{shell_files::*, *};
use std::{
    collections::{BTreeMap, HashMap},
    os::unix::net::UnixListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// Sophia@bc23ee5b crates/sophia-protocol/src/packets/shell_launcher.rs. The
// standalone shell SDK requires this bit in the dock mask but does not export
// its name.
const SOPHIA_SHELL_CAPABILITY_APPLICATION_CATALOG: u64 = 1 << 5;
const CAPABILITIES: u64 = SOPHIA_SHELL_CAPABILITY_PERSISTENT_CATALOG
    | SOPHIA_SHELL_CAPABILITY_APPLICATION_CATALOG
    | SOPHIA_SHELL_CAPABILITY_CONTENT_SURFACE
    | SOPHIA_SHELL_CAPABILITY_CONTENT_DISCRETE_INPUT
    | SOPHIA_SHELL_CAPABILITY_WORK_AREA_RESERVATION;

pub fn grant() -> ContentGrant {
    ContentGrant {
        connection_epoch: EPOCH,
        content_grant_epoch: 2,
    }
}
pub fn output(id: u64) -> ContentOutputId {
    ContentOutputId { id, generation: 1 }
}
pub struct Raster {
    pending: Option<RenderResult>,
}
impl RasterExecutor for Raster {
    fn submit(&mut self, job: RenderJobId, scene: Box<DockScene>) -> Result<(), Box<DockScene>> {
        assert!(self.pending.is_none());
        self.pending = Some(RenderResult {
            job,
            bytes: [200, 100, 50, 128].repeat((scene.width * scene.height) as usize),
            width: scene.width,
            height: scene.height,
        });
        Ok(())
    }
    fn poll(&mut self) -> Result<Option<RenderResult>, String> {
        Ok(self.pending.take())
    }
}
pub struct Peer {
    wire: Wire,
    slots: HashMap<u64, String>,
    resources: BTreeMap<(u64, u64), (ContentResourceBegin, Vec<u8>)>,
    pub presented: BTreeMap<u64, (u64, CatalogCandidateBegin, ContentCandidateChunk)>,
    pub actions: Vec<CatalogActivation>,
    pub acks: Vec<ContentActionAck>,
    pub releases: Vec<(TransactionId, ContentResourceId)>,
    pub hold_releases: bool,
    /// Record activations without sending their Session outcome.
    pub hold_outcomes: bool,
    pub begins: usize,
    /// Refuse every submission of this kind with EACCES and no custody.
    pub refuse: Option<ShellFileKind>,
    /// The candidate byte budget carried by each pacing permit.
    pub permit_bytes: u32,
}
fn object_header(kind: ShellFileKind) -> ShellFileHeader {
    ShellFileHeader {
        kind,
        connection_epoch: EPOCH,
        submission_id: 0,
        sequence: 0,
    }
}
fn peer(wire: Wire) -> Peer {
    Peer {
        wire,
        slots: HashMap::new(),
        resources: BTreeMap::new(),
        presented: BTreeMap::new(),
        actions: Vec::new(),
        acks: Vec::new(),
        releases: Vec::new(),
        hold_releases: false,
        hold_outcomes: false,
        begins: 0,
        refuse: None,
        permit_bytes: 8192,
    }
}
pub fn pair() -> (DockService<Raster>, Peer) {
    let path = std::env::temp_dir().join(format!(
        "dock-service-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let listener = UnixListener::bind(&path).unwrap();
    let connected = Arc::new(AtomicBool::new(false));
    let done = connected.clone();
    // The SDK negotiates synchronously, so the peer runs on its own thread
    // until the connection exists, then returns to the single-threaded drive.
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut wire = Wire::new(stream);
        wire.object(
            "limits",
            15,
            encode_shell_file_limits(
                object_header(ShellFileKind::Limits),
                ContentLimits::prototype(grant()),
            )
            .unwrap(),
        );
        let mut peer = peer(wire);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "negotiation did not finish");
            peer.pump();
            std::thread::sleep(Duration::from_micros(100));
        }
        peer
    });
    let connection = ShellConnection::connect_files(
        &path,
        ShellClientOptions {
            minimum_revision: 8,
            maximum_revision: 8,
            required_capabilities: CAPABILITIES,
            handshake_timeout: Duration::from_secs(2),
        },
    )
    .unwrap();
    connected.store(true, Ordering::Release);
    let mut peer = server.join().unwrap();
    std::fs::remove_file(path).unwrap();
    peer.send(
        TransactionId::from_raw(900),
        ShellContentRecord::OutputFacts(ContentOutputFacts {
            grant: grant(),
            facts_generation: 1,
            outputs: (1..=2)
                .map(|id| ContentOutputFactsEntry {
                    output: output(id),
                    local_width: 800,
                    local_height: 600,
                    scale_numerator: 1,
                    scale_denominator: 1,
                    scale_generation: 1,
                })
                .collect(),
        }),
    );
    peer.catalog(1);
    let config = Config::parse(include_str!("../../examples/minimal/config.kdl")).unwrap();
    (
        DockService::new(connection, config, 64, Raster { pending: None }).unwrap(),
        peer,
    )
}
impl Peer {
    /// Publish one server record: output facts as the `outputs` object, the
    /// rest as ordered events.
    pub fn send(&mut self, transaction: TransactionId, record: ShellContentRecord) {
        if let ShellContentRecord::OutputFacts(facts) = &record {
            let generation = facts.facts_generation;
            let bytes = encode_shell_file_outputs(
                object_header(ShellFileKind::Outputs),
                &ShellFileTransactionRecord {
                    transaction,
                    record,
                },
            )
            .unwrap();
            self.wire.object("outputs", 200 + generation, bytes);
            self.wire
                .announce(ShellFileKind::Outputs, generation, 200 + generation);
            return;
        }
        let record = ShellFileTransactionRecord {
            transaction,
            record,
        };
        let (kind, body) = match &record.record {
            ShellContentRecord::AllocationResult(_) => (
                ShellFileKind::AllocationResult,
                encode_shell_file_allocation_result_body(&record).unwrap(),
            ),
            ShellContentRecord::ResourceStatus(_) => (
                ShellFileKind::ResourceStatus,
                encode_shell_file_resource_status_body(&record).unwrap(),
            ),
            ShellContentRecord::ResourceReleased(_) => (
                ShellFileKind::ResourceReleased,
                encode_shell_file_resource_released_body(&record).unwrap(),
            ),
            _ => encode_shell_file_transaction_body(&record).unwrap(),
        };
        self.wire.event(kind, &body);
    }
    /// Publish a whole catalog, identities included, as the `catalog` object.
    pub fn catalog(&mut self, generation: u64) {
        let catalog = ShellPersistentCatalog {
            catalog: ShellApplicationCatalog {
                connection_epoch: EPOCH,
                generation,
                entries: vec![ShellApplicationDescriptor {
                    slot: 1,
                    available: true,
                    label: "Terminal".into(),
                    keywords: String::new(),
                }],
            },
            identities: BTreeMap::from([(1, "registered:terminal".into())]),
        };
        let bytes = encode_shell_file_catalog(
            object_header(ShellFileKind::Catalog),
            &ShellFileCatalog {
                transaction: TransactionId::from_raw(1000 + generation),
                catalog,
            },
        )
        .unwrap();
        self.wire.object("catalog", 300 + generation, bytes);
        self.wire
            .announce(ShellFileKind::Catalog, generation, 300 + generation);
    }
    pub fn action(&mut self, id: u64, event: u64, kind: u16) -> ContentAction {
        let (epoch, begin, chunk) = &self.presented[&id];
        let target = &chunk.targets[0];
        let action = ContentAction {
            grant: grant(),
            output: output(id),
            candidate_generation: begin.content.candidate_generation,
            presentation_epoch: *epoch,
            interaction_generation: begin.content.interaction_generation,
            allocation: chunk.surfaces[0].allocation,
            target_id: target.target_id,
            target_generation: target.target_generation,
            action_id: target.action_id,
            event_id: event,
            kind,
            reason: 0,
        };
        self.send(
            TransactionId::from_raw(2000 + event),
            ShellContentRecord::Action(action.clone()),
        );
        action
    }
    pub fn release_all(&mut self) {
        for (tx, resource) in std::mem::take(&mut self.releases) {
            self.resources.remove(&(resource.id, resource.generation));
            self.send(
                tx,
                ShellContentRecord::ResourceReleased(ContentResourceReleased {
                    grant: grant(),
                    resource,
                    reason: 0,
                }),
            );
        }
    }
    pub fn release_one(&mut self, resource: ContentResourceId) {
        let index = self
            .releases
            .iter()
            .position(|(_, r)| *r == resource)
            .unwrap();
        let (tx, resource) = self.releases.remove(index);
        self.resources.remove(&(resource.id, resource.generation));
        self.send(
            tx,
            ShellContentRecord::ResourceReleased(ContentResourceReleased {
                grant: grant(),
                resource,
                reason: 0,
            }),
        );
    }
    /// Whether the client closed its connection within `limit`.
    pub fn disconnected_within(&mut self, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if !self.wire.pump() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }
    /// One bounded 9P pass, then every whole submission in order. Each is
    /// accepted (Submitted custody) before its semantic reply.
    pub fn pump(&mut self) {
        assert!(self.wire.pump(), "dock client disconnected");
        while let Some(submission) = self.wire.submissions.pop_front() {
            let kind = decode_shell_file_record(&submission.bytes, ShellFileClass::Candidate)
                .unwrap()
                .header
                .kind;
            if Some(kind) == self.refuse {
                self.wire.refuse(&submission);
                continue;
            }
            self.wire.accept(&submission, kind);
            self.submission(kind, &submission);
        }
    }
    fn submission(&mut self, kind: ShellFileKind, submission: &Submission) {
        let bytes = &submission.bytes;
        let value = match kind {
            ShellFileKind::Negotiate => {
                let hello = decode_shell_file_negotiate(bytes).unwrap();
                assert_eq!(
                    (
                        hello.minimum_revision,
                        hello.maximum_revision,
                        hello.required_capabilities
                    ),
                    (8, 8, CAPABILITIES)
                );
                let body = encode_shell_file_negotiated_body(ShellFileNegotiated {
                    welcome: ShellV1ServerWelcome {
                        selected_revision: 8,
                        connection_epoch: EPOCH,
                        capabilities: CAPABILITIES,
                        max_descriptors: 16,
                        max_label_bytes: 128,
                        max_pending_activations: 16,
                    },
                    limits_published: true,
                })
                .unwrap();
                self.wire.event(ShellFileKind::Negotiated, &body);
                return;
            }
            ShellFileKind::AllocationRequest => {
                decode_shell_file_allocation_request(bytes).unwrap()
            }
            ShellFileKind::ResourceBegin => {
                let v = decode_shell_file_resource_begin(bytes).unwrap();
                let ShellContentRecord::ResourceBegin(begin) = &v.record else {
                    unreachable!()
                };
                let name = format!("upload/{}", v.slot);
                self.wire.uploads.insert(name.clone(), Vec::new());
                self.slots.insert(begin.resource.id, name);
                ShellFileTransactionRecord {
                    transaction: v.transaction,
                    record: v.record,
                }
            }
            ShellFileKind::ResourceEnd => {
                let v = decode_shell_file_resource_end(bytes).unwrap();
                let ShellContentRecord::ResourceEnd(end) = &v.record else {
                    unreachable!()
                };
                let slot = self.slots.remove(&end.resource.id).unwrap();
                let data = self.wire.uploads.remove(&slot).unwrap();
                assert!(
                    end.total_bytes <= 137 || self.wire.short_writes > 0,
                    "control must exercise partial slot writes"
                );
                // Slot bytes arrive as file writes, not chunk records.
                self.content(
                    v.transaction,
                    ShellContentRecord::ResourceChunk(ContentResourceChunk {
                        grant: end.grant,
                        resource: end.resource,
                        ordinal: 0,
                        offset: 0,
                        bytes: data,
                    }),
                );
                v
            }
            ShellFileKind::ResourceRetire => decode_shell_file_resource_retire(bytes).unwrap(),
            ShellFileKind::FrameDemand | ShellFileKind::ActionAck => {
                decode_shell_file_transaction(bytes, kind).unwrap()
            }
            ShellFileKind::CatalogCandidate => {
                let v = decode_shell_file_catalog_candidate(bytes).unwrap();
                self.candidate(v.transaction, v.candidate);
                return;
            }
            ShellFileKind::CatalogActivate => {
                let v = decode_shell_file_catalog_action(bytes, kind).unwrap();
                let ShellCatalogActionRecord::Activate(activation) = v.record else {
                    unreachable!()
                };
                self.activate(v.transaction, activation);
                return;
            }
            _ => panic!("unexpected transaction {kind:?}"),
        };
        self.content(value.transaction, value.record);
    }
    fn activate(&mut self, tx: TransactionId, activation: CatalogActivation) {
        assert!(
            self.acks
                .iter()
                .any(|ack| ack.event_id == activation.action.event_id && ack.disposition == 1)
        );
        self.actions.push(activation.clone());
        if self.hold_outcomes {
            return;
        }
        let (kind, body) = encode_shell_file_catalog_action_body(&ShellFileCatalogActionRecord {
            transaction: tx,
            record: ShellCatalogActionRecord::ActivationOutcome(CatalogActivationOutcome {
                activation,
                status: 1,
                reason: 0,
            }),
        })
        .unwrap();
        self.wire.event(kind, &body);
    }
    fn candidate(&mut self, tx: TransactionId, value: CatalogContentCandidate) {
        let (
            ShellCatalogActionRecord::CandidateBegin(begin),
            ShellCatalogActionRecord::CandidateChunk(chunk),
            ShellContentRecord::CandidateEnd(end),
        ) = value.parts()
        else {
            panic!("catalog candidate parts")
        };
        assert_eq!(end.candidate_generation, begin.content.candidate_generation);
        let epoch = end.candidate_generation + 100;
        for kind in [1, 2] {
            self.send(
                tx,
                ShellContentRecord::CandidateOutcome(ContentCandidateOutcome {
                    grant: grant(),
                    candidate_generation: end.candidate_generation,
                    output: begin.content.output,
                    kind,
                    reason: 0,
                    presentation_epoch: if kind == 2 { epoch } else { 0 },
                    work_area_generation: 1,
                    wm_commit_generation: 1,
                }),
            );
        }
        self.presented
            .insert(begin.content.output.id, (epoch, begin, chunk));
    }
    fn content(&mut self, tx: TransactionId, record: ShellContentRecord) {
        match record {
            ShellContentRecord::AllocationRequest(request) => {
                assert_eq!((request.role, request.edge), (1, 3));
                self.send(
                    tx,
                    ShellContentRecord::AllocationResult(ContentAllocationResult {
                        grant: grant(),
                        allocation_request_id: request.allocation_request_id,
                        status: 1,
                        reason: 0,
                        output: request.output,
                        allocation: ContentAllocationId {
                            id: request.output.id,
                            generation: 1,
                        },
                        parent: ContentAllocationId::default(),
                        scale_generation: 1,
                        logical: ContentLogicalRect {
                            x: 0,
                            y: 536,
                            width: 800,
                            height: 64,
                        },
                        pixel: ContentPixelRect {
                            x: 0,
                            y: 536,
                            width: 800,
                            height: 64,
                        },
                        scale_numerator: 1,
                        scale_denominator: 1,
                        allowed_reservation_extent: 64,
                        margins: ContentMargins::default(),
                        acknowledged_anchor: ContentPixelRect::default(),
                    }),
                );
            }
            ShellContentRecord::ResourceBegin(begin) => {
                self.begins += 1;
                let resource = begin.resource;
                assert!(
                    self.resources
                        .insert((resource.id, resource.generation), (begin, Vec::new()))
                        .is_none()
                );
                self.send(
                    tx,
                    ShellContentRecord::ResourceStatus(ContentResourceStatus {
                        grant: grant(),
                        resource,
                        status: 1,
                        reason: 0,
                        next_ordinal: 0,
                        admitted_bytes: 0,
                    }),
                );
            }
            ShellContentRecord::ResourceChunk(chunk) => {
                let (_, bytes) = self
                    .resources
                    .get_mut(&(chunk.resource.id, chunk.resource.generation))
                    .unwrap();
                assert_eq!(bytes.len() as u64, chunk.offset);
                bytes.extend_from_slice(&chunk.bytes);
            }
            ShellContentRecord::ResourceEnd(end) => {
                let (begin, bytes) = &self.resources[&(end.resource.id, end.resource.generation)];
                assert_eq!(begin.total_bytes, end.total_bytes);
                assert_eq!(bytes.len() as u64, end.total_bytes);
                assert_eq!(&bytes[..4], &[25, 50, 100, 128], "wire premultiplied BGRA");
                self.send(
                    tx,
                    ShellContentRecord::ResourceStatus(ContentResourceStatus {
                        grant: grant(),
                        resource: end.resource,
                        status: 2,
                        reason: 0,
                        next_ordinal: end.chunk_count,
                        admitted_bytes: end.total_bytes,
                    }),
                );
            }
            ShellContentRecord::FrameDemand(demand) => self.send(
                tx,
                ShellContentRecord::FramePermit(ContentFramePermit {
                    grant: grant(),
                    output: demand.output,
                    demand_id: demand.demand_id,
                    permit_id: demand.demand_id,
                    state: 1,
                    reason: 0,
                    ttl_ms: 250,
                    max_candidate_bytes: self.permit_bytes,
                }),
            ),
            ShellContentRecord::ResourceRetire(retire) => {
                self.releases.push((tx, retire.resource));
                if !self.hold_releases {
                    self.release_all();
                }
            }
            ShellContentRecord::ActionAck(ack) => self.acks.push(ack),
            other => panic!("unexpected client record: {other:?}"),
        }
    }
}
pub fn drive(service: &mut DockService<Raster>, peer: &mut Peer, done: impl Fn(&Peer) -> bool) {
    let start = Instant::now();
    loop {
        service.step().unwrap();
        peer.pump();
        if done(peer) {
            service.step().unwrap();
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "fixture did not converge"
        );
        std::thread::yield_now();
    }
}
