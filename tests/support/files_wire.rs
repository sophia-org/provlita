//! Scripted 9P plumbing, adapted from Lom@53d3a921 tests/support/files_wire.rs,
//! itself from sophia-desktop-sdk-rs@0da10428
//! crates/sophia-shell-client/tests/support/file_wire_peer.rs (MIT).
//! Session decisions live in service_host.rs. This is not a server-owner test.

use sophia_shell_protocol::shell_files::*;
use std::{
    collections::{HashMap, VecDeque},
    io::{ErrorKind, Read, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

pub const EPOCH: u64 = 41;
const MSIZE: usize = 8192;

pub struct Submission {
    pub tag: u16,
    pub id: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone)]
struct Object {
    qid: u64,
    bytes: Vec<u8>,
}

pub struct Wire {
    stream: UnixStream,
    input: Vec<u8>,
    fids: HashMap<u32, String>,
    objects: HashMap<String, Object>,
    pins: HashMap<u32, Object>,
    transaction: Vec<u8>,
    events: VecDeque<u8>,
    reader: Option<(u16, u32)>,
    read_offset: u64,
    sequence: u64,
    accepted: u64,
    acked: u64,
    pub submissions: VecDeque<Submission>,
    pub uploads: HashMap<String, Vec<u8>>,
    pub short_writes: usize,
    closed: bool,
}

impl Wire {
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            input: Vec::new(),
            fids: HashMap::new(),
            objects: HashMap::new(),
            pins: HashMap::new(),
            transaction: Vec::new(),
            events: VecDeque::new(),
            reader: None,
            read_offset: 0,
            sequence: 0,
            accepted: 0,
            acked: 0,
            submissions: VecDeque::new(),
            uploads: HashMap::new(),
            short_writes: 0,
            closed: false,
        }
    }

    pub fn object(&mut self, name: &str, qid: u64, bytes: Vec<u8>) {
        self.objects.insert(name.into(), Object { qid, bytes });
    }

    pub fn announce(&mut self, kind: ShellFileKind, generation: u64, qid: u64) {
        let body = encode_shell_file_object_published_body(ShellFileObjectPublished {
            object: kind,
            generation,
            qid,
        })
        .unwrap();
        self.event(ShellFileKind::ObjectPublished, &body);
    }

    pub fn event(&mut self, kind: ShellFileKind, body: &[u8]) {
        self.sequence += 1;
        self.events.extend(
            encode_shell_file_record(
                ShellFileHeader {
                    kind,
                    connection_epoch: EPOCH,
                    submission_id: 0,
                    sequence: self.sequence,
                },
                body,
            )
            .unwrap(),
        );
    }

    pub fn accept(&mut self, submission: &Submission, kind: ShellFileKind) {
        let body = encode_shell_file_submitted_body(ShellFileSubmitted {
            submission_id: submission.id,
            candidate_kind: kind,
        })
        .unwrap();
        self.event(ShellFileKind::Submitted, &body);
        self.accepted = self.sequence;
        self.write_reply(submission.tag, SHELL_FILE_SUBMIT_BYTES as u32);
    }

    pub fn refuse(&mut self, submission: &Submission) {
        self.send(7, submission.tag, &13u32.to_le_bytes());
    }

    /// One bounded pass, including partial frames. Returns false on disconnect.
    pub fn pump(&mut self) -> bool {
        if self.closed {
            return false;
        }
        self.stream.set_nonblocking(true).unwrap();
        let mut chunk = [0; MSIZE];
        match self.stream.read(&mut chunk) {
            Ok(0) => return false,
            Ok(n) => self.input.extend_from_slice(&chunk[..n]),
            // Exiting with an outstanding event read can reset a Unix socket.
            // Required lifecycle messages are checked by the scenario owner.
            Err(e) if e.kind() == ErrorKind::ConnectionReset => return false,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
            Err(e) => panic!("9P read: {e}"),
        }
        for _ in 0..64 {
            if self.input.len() < 7 {
                break;
            }
            let size = u32_at(&self.input, 0) as usize;
            assert!((7..=MSIZE).contains(&size));
            if self.input.len() < size {
                break;
            }
            let frame: Vec<_> = self.input.drain(..size).collect();
            self.handle(frame[4], u16_at(&frame, 5), &frame[7..]);
        }
        if !self.events.is_empty()
            && let Some((tag, count)) = self.reader.take()
        {
            let bytes: Vec<_> = self
                .events
                .drain(..self.events.len().min(count as usize))
                .collect();
            self.read_offset += bytes.len() as u64;
            self.read_reply(tag, &bytes);
        }
        !self.closed
    }

    fn handle(&mut self, kind: u8, tag: u16, b: &[u8]) {
        match kind {
            100 => {
                assert_eq!(&b[6..], b"9P2000.L");
                let mut response = (MSIZE as u32).to_le_bytes().to_vec();
                response.extend(8u16.to_le_bytes());
                response.extend(b"9P2000.L");
                self.send(101, tag, &response);
            }
            104 => {
                self.fids.insert(u32_at(b, 0), String::new());
                self.send(105, tag, &qid(1));
            }
            110 => {
                let mut at = 10;
                let mut names = Vec::new();
                for _ in 0..u16_at(b, 8) {
                    let len = u16_at(b, at) as usize;
                    names.push(std::str::from_utf8(&b[at + 2..at + 2 + len]).unwrap());
                    at += 2 + len;
                }
                let name = if names.is_empty() {
                    self.fids[&u32_at(b, 0)].clone()
                } else {
                    names.join("/")
                };
                let mut response = (names.len() as u16).to_le_bytes().to_vec();
                for _ in &names {
                    response.extend(self.qid(&name));
                }
                self.fids.insert(u32_at(b, 4), name);
                self.send(111, tag, &response);
            }
            12 => {
                let fid = u32_at(b, 0);
                let name = self.fids[&fid].clone();
                if name == "transaction" {
                    // A previous Submitted must be acknowledged before reopen.
                    assert!(
                        self.acked >= self.accepted,
                        "transaction opened before custody ack"
                    );
                    self.transaction.clear();
                }
                if let Some(object) = self.objects.get(&name) {
                    self.pins.insert(fid, object.clone());
                }
                let mut response = self.qid(&name).to_vec();
                response.extend(0u32.to_le_bytes());
                self.send(13, tag, &response);
            }
            116 => {
                let fid = u32_at(b, 0);
                let offset = u64_at(b, 4);
                let count = u32_at(b, 12);
                match self.fids[&fid].as_str() {
                    "events" => {
                        assert_eq!(offset, self.read_offset);
                        assert!(self.reader.is_none());
                        self.reader = Some((tag, count));
                    }
                    "api" => {
                        let api = format!(
                            "sophia-shell-files version=1 role=dock epoch={EPOCH} fd_transfer=none\n"
                        );
                        self.read_slice(tag, api.as_bytes(), offset, count);
                    }
                    _ => {
                        let bytes = self.pins[&fid].bytes.clone();
                        // Positive short object reads plus the required EOF probe.
                        self.read_slice(tag, &bytes, offset, count.min(79));
                    }
                }
            }
            118 => {
                let name = self.fids[&u32_at(b, 0)].clone();
                let offset = u64_at(b, 4) as usize;
                let count = u32_at(b, 12);
                let bytes = &b[16..];
                assert_eq!(bytes.len(), count as usize);
                match name.as_str() {
                    "transaction" => {
                        assert_eq!(offset, self.transaction.len());
                        self.transaction.extend(bytes);
                        self.write_reply(tag, count);
                    }
                    "submit" => {
                        let submit = decode_shell_file_submit(bytes).unwrap();
                        let header =
                            decode_shell_file_record(&self.transaction, ShellFileClass::Candidate)
                                .unwrap()
                                .header;
                        assert_eq!(
                            (submit.connection_epoch, submit.submission_id),
                            (EPOCH, header.submission_id)
                        );
                        self.submissions.push_back(Submission {
                            tag,
                            id: submit.submission_id,
                            bytes: self.transaction.clone(),
                        });
                    }
                    "ack" => {
                        let ack = decode_shell_file_ack(bytes).unwrap();
                        assert_eq!(ack.connection_epoch, EPOCH);
                        assert!(ack.sequence >= self.acked && ack.sequence <= self.sequence);
                        self.acked = ack.sequence;
                        self.write_reply(tag, count);
                    }
                    _ if name.starts_with("upload/") => {
                        let upload = self.uploads.get_mut(&name).expect("admitted slot only");
                        assert_eq!(offset, upload.len());
                        let stored = bytes.len().min(137);
                        self.short_writes += usize::from(stored < bytes.len());
                        upload.extend(&bytes[..stored]);
                        self.write_reply(tag, stored as u32);
                    }
                    _ => panic!("unexpected write {name}"),
                }
            }
            120 => {
                let fid = u32_at(b, 0);
                self.fids.remove(&fid);
                self.pins.remove(&fid);
                self.send(121, tag, &[]);
            }
            108 => self.send(109, tag, &[]),
            _ => panic!("unexpected 9P type {kind}"),
        }
    }

    fn qid(&self, name: &str) -> [u8; 13] {
        qid(self.objects.get(name).map_or(9, |object| object.qid))
    }

    fn read_slice(&mut self, tag: u16, bytes: &[u8], offset: u64, count: u32) {
        let start = (offset as usize).min(bytes.len());
        self.read_reply(
            tag,
            &bytes[start..(start + count as usize).min(bytes.len())],
        );
    }

    fn read_reply(&mut self, tag: u16, bytes: &[u8]) {
        let mut response = (bytes.len() as u32).to_le_bytes().to_vec();
        response.extend(bytes);
        self.send(117, tag, &response);
    }

    fn write_reply(&mut self, tag: u16, count: u32) {
        self.send(119, tag, &count.to_le_bytes());
    }

    fn send(&mut self, kind: u8, tag: u16, body: &[u8]) {
        let mut frame = ((7 + body.len()) as u32).to_le_bytes().to_vec();
        frame.push(kind);
        frame.extend(tag.to_le_bytes());
        frame.extend(body);
        self.stream.set_nonblocking(false).unwrap();
        self.stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        match self.stream.write_all(&frame) {
            Ok(()) => {}
            Err(e) if matches!(e.kind(), ErrorKind::BrokenPipe | ErrorKind::ConnectionReset) => {
                self.closed = true;
            }
            Err(e) => panic!("9P write: {e}"),
        }
    }
}

fn qid(path: u64) -> [u8; 13] {
    let mut bytes = [0; 13];
    bytes[5..].copy_from_slice(&path.to_le_bytes());
    bytes
}
fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
