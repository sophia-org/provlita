//! Private socket fixture. It supplies allocation/pacing/native outcomes; it
//! does not run Session, a GPU, WM execution or application processes.
use provlita::{
    config::Config,
    render::{RenderJobId, RenderResult},
    service::{DockService, RasterExecutor},
    ui::DockScene,
};
use sophia_protocol::*;
use sophia_shell_client::{ShellClientOptions, ShellConnection};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub fn grant() -> ContentGrant {
    ContentGrant {
        connection_epoch: 1,
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
    stream: UnixStream,
    input: Vec<u8>,
    resources: BTreeMap<(u64, u64), (ContentResourceBegin, Vec<u8>)>,
    candidates: BTreeMap<
        u64,
        (
            TransactionId,
            CatalogCandidateBegin,
            Option<ContentCandidateChunk>,
        ),
    >,
    pub presented: BTreeMap<u64, (u64, CatalogCandidateBegin, ContentCandidateChunk)>,
    pub actions: Vec<CatalogActivation>,
    pub acks: Vec<ContentActionAck>,
    pub releases: Vec<(TransactionId, ContentResourceId)>,
    pub hold_releases: bool,
    pub begins: usize,
}
fn frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut header = [0; SOPHIA_IPC_HEADER_LEN];
    stream.read_exact(&mut header).unwrap();
    let len = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
    let mut bytes = header.to_vec();
    bytes.resize(bytes.len() + len, 0);
    stream
        .read_exact(&mut bytes[SOPHIA_IPC_HEADER_LEN..])
        .unwrap();
    bytes
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
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let hello = decode_shell_v1_client_hello_frame(&frame(&mut stream)).unwrap();
        assert_eq!(hello.minimum_revision, 8);
        stream
            .write_all(
                &encode_shell_v1_server_welcome_frame(ShellV1ServerWelcome {
                    selected_revision: 8,
                    connection_epoch: 1,
                    capabilities: hello.required_capabilities,
                    max_descriptors: 16,
                    max_label_bytes: 128,
                    max_pending_activations: 16,
                })
                .unwrap(),
            )
            .unwrap();
        stream
    });
    let connection = ShellConnection::connect(
        &path,
        ShellClientOptions {
            minimum_revision: 8,
            maximum_revision: 8,
            required_capabilities: SOPHIA_SHELL_CAPABILITY_PERSISTENT_CATALOG
                | SOPHIA_SHELL_CAPABILITY_APPLICATION_CATALOG
                | SOPHIA_SHELL_CAPABILITY_CONTENT_SURFACE
                | SOPHIA_SHELL_CAPABILITY_CONTENT_DISCRETE_INPUT
                | SOPHIA_SHELL_CAPABILITY_WORK_AREA_RESERVATION,
            handshake_timeout: Duration::from_secs(2),
        },
    )
    .unwrap();
    let stream = server.join().unwrap();
    std::fs::remove_file(path).unwrap();
    stream.set_nonblocking(true).unwrap();
    let mut peer = Peer {
        stream,
        input: Vec::new(),
        resources: BTreeMap::new(),
        candidates: BTreeMap::new(),
        presented: BTreeMap::new(),
        actions: Vec::new(),
        acks: Vec::new(),
        releases: Vec::new(),
        hold_releases: false,
        begins: 0,
    };
    peer.send(
        TransactionId::from_raw(0),
        ShellContentRecord::Limits(ContentLimits::prototype(grant())),
    );
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
    pub fn send(&mut self, tx: TransactionId, record: ShellContentRecord) {
        self.stream
            .write_all(&encode_shell_content_frame(tx, &record).unwrap())
            .unwrap();
    }
    pub fn catalog(&mut self, generation: u64) {
        let tx = TransactionId::from_raw(1000 + generation);
        let mut frames = encode_shell_application_catalog(
            tx,
            &ShellApplicationCatalog {
                connection_epoch: 1,
                generation,
                entries: vec![ShellApplicationDescriptor {
                    slot: 1,
                    available: true,
                    label: "Terminal".into(),
                    keywords: String::new(),
                }],
            },
        )
        .unwrap();
        let end = frames.pop().unwrap();
        frames.push(
            encode_shell_catalog_action_frame(
                tx,
                &ShellCatalogActionRecord::Identity(ShellCatalogIdentity {
                    connection_epoch: 1,
                    catalog_generation: generation,
                    slot: 1,
                    identity: "registered:terminal".into(),
                }),
            )
            .unwrap(),
        );
        frames.push(end);
        for frame in frames {
            self.stream.write_all(&frame).unwrap();
        }
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
    pub fn pump(&mut self) {
        let mut buf = [0; 8192];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.input.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("peer read: {e}"),
            }
        }
        while self.input.len() >= SOPHIA_IPC_HEADER_LEN {
            let len = SOPHIA_IPC_HEADER_LEN
                + u32::from_le_bytes(self.input[16..20].try_into().unwrap()) as usize;
            if self.input.len() < len {
                break;
            }
            let bytes = self.input.drain(..len).collect::<Vec<_>>();
            let (header, _) = decode_frame(&bytes).unwrap();
            let tx = header.transaction;
            match header.message_kind {
                IpcMessageKind::ShellCatalogCandidateBegin => {
                    let (_, ShellCatalogActionRecord::CandidateBegin(begin)) =
                        decode_shell_catalog_action_frame(&bytes).unwrap()
                    else {
                        panic!()
                    };
                    self.candidates
                        .insert(begin.content.candidate_generation, (tx, begin, None));
                }
                IpcMessageKind::ShellCatalogCandidateChunk => {
                    let (_, ShellCatalogActionRecord::CandidateChunk(chunk)) =
                        decode_shell_catalog_action_frame(&bytes).unwrap()
                    else {
                        panic!()
                    };
                    let generation = chunk.candidate_generation;
                    self.candidates.get_mut(&generation).unwrap().2 = Some(chunk);
                }
                IpcMessageKind::ShellCatalogActivate => {
                    let (_, ShellCatalogActionRecord::Activate(activation)) =
                        decode_shell_catalog_action_frame(&bytes).unwrap()
                    else {
                        panic!()
                    };
                    assert!(
                        self.acks
                            .iter()
                            .any(|ack| ack.event_id == activation.action.event_id
                                && ack.disposition == 1)
                    );
                    self.actions.push(activation.clone());
                    self.stream
                        .write_all(
                            &encode_shell_catalog_action_frame(
                                tx,
                                &ShellCatalogActionRecord::ActivationOutcome(
                                    CatalogActivationOutcome {
                                        activation,
                                        status: 1,
                                        reason: 0,
                                    },
                                ),
                            )
                            .unwrap(),
                        )
                        .unwrap();
                }
                _ => {
                    let (_, record) = decode_shell_content_frame(&bytes).unwrap();
                    self.content(tx, record);
                }
            }
        }
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
                    max_candidate_bytes: 8192,
                }),
            ),
            ShellContentRecord::CandidateEnd(end) => {
                let (begin_tx, begin, chunk) =
                    self.candidates.remove(&end.candidate_generation).unwrap();
                assert_eq!(begin_tx, tx);
                let chunk = chunk.unwrap();
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
