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
    collections::BTreeMap,
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
// Negotiated capability bit 13: sealed image channel (ISB).
const SOPHIA_SHELL_CAPABILITY_SEALED_IMAGE: u64 = 1 << 13;
const CAPABILITIES: u64 = SOPHIA_SHELL_CAPABILITY_PERSISTENT_CATALOG
    | SOPHIA_SHELL_CAPABILITY_APPLICATION_CATALOG
    | SOPHIA_SHELL_CAPABILITY_CONTENT_SURFACE
    | SOPHIA_SHELL_CAPABILITY_CONTENT_DISCRETE_INPUT
    | SOPHIA_SHELL_CAPABILITY_WORK_AREA_RESERVATION
    | SOPHIA_SHELL_CAPABILITY_SEALED_IMAGE;

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
    resources: BTreeMap<(u64, u64), (ShellImageMetadata, Vec<u8>)>,
    image_listener: Option<std::os::fd::OwnedFd>,
    image_socket: Option<std::os::fd::OwnedFd>,
    hello_seen: bool,
    /// Send ChannelClosed right after ChannelReady is consumed.
    pub close_after_ready: bool,
    channel_closed_sent: bool,
    /// Verify Sources and send Accepted, but never SourceReleased.
    pub hold_image_settle: bool,
    next_offer: u64,
    admit_queue: Vec<(TransactionId, ShellImageMetadata, bool)>,
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
    /// The pixel width every granted allocation gets.
    pub width: u32,
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
        resources: BTreeMap::new(),
        image_listener: None,
        image_socket: None,
        hello_seen: false,
        close_after_ready: false,
        channel_closed_sent: false,
        hold_image_settle: false,
        next_offer: 0,
        admit_queue: Vec::new(),
        presented: BTreeMap::new(),
        actions: Vec::new(),
        acks: Vec::new(),
        releases: Vec::new(),
        hold_releases: false,
        hold_outcomes: false,
        begins: 0,
        refuse: None,
        permit_bytes: 8192,
        width: 800,
    }
}
/// Bind a SEQPACKET listener for the sealed image endpoint.
fn seqpacket_listener(path: &std::path::Path) -> std::os::fd::OwnedFd {
    use std::os::fd::FromRawFd;
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0);
        assert!(fd >= 0, "seqpacket socket");
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        let bytes = c_path.as_bytes();
        assert!(bytes.len() < addr.sun_path.len());
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            addr.sun_path.as_mut_ptr() as *mut u8,
            bytes.len(),
        );
        let len = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len()) as u32;
        assert_eq!(
            libc::bind(fd, &addr as *const _ as *const libc::sockaddr, len),
            0,
            "seqpacket bind"
        );
        assert_eq!(libc::listen(fd, 4), 0, "seqpacket listen");
        std::os::fd::OwnedFd::from_raw_fd(fd)
    }
}

fn fd_ready(fd: std::os::fd::RawFd) -> bool {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut pfd, 1, 0) > 0 && pfd.revents & libc::POLLIN != 0 }
}

/// Receive one packet plus any descriptors; `None` when none is ready.
fn recv_packet(fd: std::os::fd::RawFd) -> Option<(Vec<u8>, Vec<std::os::fd::OwnedFd>)> {
    use std::os::fd::FromRawFd;
    if !fd_ready(fd) {
        return None;
    }
    let mut buf = [0u8; 512];
    let mut control = [0u8; 128];
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr() as *mut libc::c_void,
        iov_len: buf.len(),
    };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = control.len();
    let n = unsafe { libc::recvmsg(fd, &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n <= 0 {
        // Closed or raced readiness; the service observes the
        // channel outcome through its own error paths.
        return None;
    }
    let mut fds = Vec::new();
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let count = ((*cmsg).cmsg_len - libc::CMSG_LEN(0) as usize) / 4;
                let data = libc::CMSG_DATA(cmsg) as *const i32;
                for i in 0..count {
                    fds.push(std::os::fd::OwnedFd::from_raw_fd(*data.add(i)));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }
    Some((buf[..n as usize].to_vec(), fds))
}

fn discovery_text(endpoint: &str) -> Vec<u8> {
    ShellImageDiscovery {
        connection_epoch: EPOCH,
        channel_generation: 3,
        endpoint: endpoint.to_owned(),
        source_kinds: 1,
        max_offers: 4,
        max_packets_per_turn: 1,
        hello_timeout_ms: 250,
        validation_timeout_ms: 2_000,
        validation_chunk_bytes: 65_536,
        capture_interest_timeout_ms: 500,
        formats_generation: 11,
        limits_generation: 9,
    }
    .encode()
    .unwrap()
    .into_bytes()
}

fn image_limits_record() -> Vec<u8> {
    let body = ShellImageLimits {
        channel_generation: 3,
        limits_generation: 9,
        max_resource_bytes: 4_194_304,
        max_sealed_backing_bytes: 16_777_216,
        max_dma_source_bytes: 0,
        max_renderer_allocation_bytes: 0,
        max_renderer_entries: 0,
        max_offers: 4,
        max_validation_jobs: 4,
        max_capture_jobs: 0,
        max_admitted_outputs: 16,
        max_format_rows_per_output: 1,
        max_width: 8192,
        max_height: 4096,
        validation_timeout_ms: 2_000,
        capture_interest_timeout_ms: 500,
        validation_chunk_bytes: 65_536,
        max_packets_per_turn: 1,
    }
    .encode()
    .unwrap();
    encode_shell_file_record_at_revision(object_header(ShellFileKind::ImageLimits), &body, 9)
        .unwrap()
}

fn image_formats_record() -> Vec<u8> {
    let body = ShellImageFormats {
        channel_generation: 3,
        formats_generation: 11,
        limits_generation: 9,
        rows: Vec::new(),
    }
    .encode()
    .unwrap();
    encode_shell_file_record_at_revision(object_header(ShellFileKind::ImageFormats), &body, 9)
        .unwrap()
}

pub fn pair() -> (DockService<Raster>, Peer) {
    pair_with_width(800)
}
/// Both outputs and every allocation are `width` pixels wide.
pub fn pair_with_width(width: u32) -> (DockService<Raster>, Peer) {
    let path = std::env::temp_dir().join(format!(
        "dock-service-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let listener = UnixListener::bind(&path).unwrap();
    let endpoint_name = format!("{}.image", path.file_name().unwrap().to_str().unwrap());
    let endpoint_dir = path.parent().unwrap().to_path_buf();
    let image_listener = seqpacket_listener(&endpoint_dir.join(&endpoint_name));
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
        wire.object("image-channel", 41, discovery_text(&endpoint_name));
        wire.object("image-limits", 42, image_limits_record());
        wire.object("image-formats", 43, image_formats_record());
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
            minimum_revision: 9,
            maximum_revision: 9,
            required_capabilities: CAPABILITIES,
            handshake_timeout: Duration::from_secs(2),
        },
    )
    .unwrap();
    connected.store(true, Ordering::Release);
    let mut peer = server.join().unwrap();
    peer.image_listener = Some(image_listener);
    peer.width = width;
    std::fs::remove_file(path).unwrap();
    peer.send(
        TransactionId::from_raw(900),
        ShellContentRecord::OutputFacts(ContentOutputFacts {
            grant: grant(),
            facts_generation: 1,
            outputs: (1..=2)
                .map(|id| ContentOutputFactsEntry {
                    output: output(id),
                    local_width: width,
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
        DockService::new(
            connection,
            config,
            64,
            Raster { pending: None },
            endpoint_dir,
        )
        .unwrap(),
        peer,
    )
}
impl Peer {
    /// Every sealed image the dock offered, as the SDK decoded it.
    pub fn declared_begins(&self) -> Vec<ShellImageMetadata> {
        self.resources.values().map(|(begin, _)| *begin).collect()
    }
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
            let kind = decode_shell_file_record_at_revision(
                &submission.bytes,
                ShellFileClass::Candidate,
                9,
            )
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
        self.image_pump();
    }

    /// Drive the sealed endpoint: Hello, ChannelReady, ImageAdmitted,
    /// then verify each Source descriptor before Accepted + Settled 1.
    fn image_pump(&mut self) {
        use std::os::fd::{AsRawFd, FromRawFd};
        if self.image_socket.is_none()
            && let Some(listener) = &self.image_listener
            && fd_ready(listener.as_raw_fd())
        {
            let fd = unsafe {
                libc::accept4(
                    listener.as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    libc::SOCK_CLOEXEC,
                )
            };
            assert!(fd >= 0, "image endpoint accept");
            self.image_socket = Some(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) });
        }
        let Some(socket) = &self.image_socket else {
            return;
        };
        let raw = socket.as_raw_fd();
        if !self.hello_seen {
            if let Some((packet, fds)) = recv_packet(raw) {
                assert_eq!(packet.len(), ShellImagePacket::HELLO_BYTES);
                assert!(fds.is_empty(), "Hello carries no descriptors");
                self.hello_seen = true;
                let body = ShellChannelReady {
                    channel_generation: 3,
                    granted_source_kinds: 1,
                }
                .encode()
                .unwrap();
                self.wire.event_image(ShellFileKind::ChannelReady, &body);
            }
            return;
        }
        if self.close_after_ready && !self.channel_closed_sent {
            self.channel_closed_sent = true;
            let body = ShellChannelClosed {
                channel_generation: 3,
                reason: ShellChannelClosedReason::PeerDisconnect,
            }
            .encode()
            .unwrap();
            self.wire.event_image(ShellFileKind::ChannelClosed, &body);
            return;
        }
        if let Some(entry) = self.admit_queue.iter_mut().find(|(_, _, sent)| !sent) {
            entry.2 = true;
            let body = entry.1.encode().unwrap();
            self.wire.event_image(ShellFileKind::ImageAdmitted, &body);
        }
        let Some((packet, fds)) = recv_packet(raw) else {
            return;
        };
        assert_eq!(packet.len(), 200, "Source packet size");
        assert_eq!(fds.len(), 1, "Source carries exactly one descriptor");
        let ShellImagePacket::Source { metadata, .. } = ShellImagePacket::decode(&packet).unwrap()
        else {
            panic!("expected Source packet");
        };
        let (tx, admitted) = self
            .admit_queue
            .iter()
            .find(|(_, m, _)| m.offer_id == metadata.offer_id)
            .map(|(tx, m, _)| (*tx, *m))
            .expect("Source names an admitted offer");
        assert_eq!(admitted, metadata);
        let mut bytes = vec![0u8; metadata.declared_bytes as usize];
        let read = unsafe {
            libc::pread(
                fds[0].as_raw_fd(),
                bytes.as_mut_ptr() as *mut libc::c_void,
                bytes.len(),
                0,
            )
        };
        assert_eq!(read as usize, bytes.len(), "sealed bytes readable");
        assert_eq!(
            &bytes[..4],
            &[25, 50, 100, 128],
            "wire premultiplied BGRA in sealed source"
        );
        assert!(
            self.resources
                .insert(
                    (metadata.resource, metadata.resource_generation),
                    (metadata, bytes)
                )
                .is_none()
        );
        self.admit_queue
            .retain(|(_, m, _)| m.offer_id != metadata.offer_id);
        // The status transaction names the offer, per the image
        // channel's status-routing rule.
        self.send(
            TransactionId::from_raw(metadata.offer_id),
            ShellContentRecord::ResourceStatus(ContentResourceStatus {
                grant: grant(),
                resource: ContentResourceId {
                    id: metadata.resource,
                    generation: metadata.resource_generation,
                },
                status: 2,
                reason: 0,
                next_ordinal: 0,
                admitted_bytes: metadata.declared_bytes,
            }),
        );
        let _ = tx;
        if !self.hold_image_settle {
            let body = ShellSourceReleased {
                offer_id: metadata.offer_id,
                resource: metadata.resource,
                resource_generation: metadata.resource_generation,
                disposition: ShellSourceDisposition::Retained,
            }
            .encode()
            .unwrap();
            self.wire.event_image(ShellFileKind::SourceReleased, &body);
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
                    (9, 9, CAPABILITIES)
                );
                let body = encode_shell_file_negotiated_body(ShellFileNegotiated {
                    welcome: ShellV1ServerWelcome {
                        selected_revision: 9,
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
            ShellFileKind::ImageBegin => {
                let record =
                    decode_shell_file_record_at_revision(bytes, ShellFileClass::Candidate, 9)
                        .unwrap();
                let mut metadata = ShellImageMetadata::decode(record.body).unwrap();
                assert_eq!(
                    metadata.admission_epoch,
                    grant().content_grant_epoch,
                    "admission epoch must name the content grant, not the connection"
                );
                self.begins += 1;
                self.next_offer += 1;
                metadata.offer_id = self.next_offer;
                self.admit_queue.push((
                    TransactionId::from_raw(record.header.submission_id),
                    metadata,
                    false,
                ));
                return;
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
                            width: self.width,
                            height: 64,
                        },
                        pixel: ContentPixelRect {
                            x: 0,
                            y: 536,
                            width: self.width,
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
