//! Real OS signals against the protected serve entry. A private listener
//! accepts the 9P connection and never answers, holding the process inside
//! negotiation; no Session, GPU or display is involved. The bwrap cases make
//! the dock the init of a new PID namespace, which ignores ancestor signals
//! unless a handler is installed.
use std::{
    io::{ErrorKind, Read},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const LIMIT: Duration = Duration::from_secs(8);

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "provlita-stop-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Owns the child so a failed assertion still kills and reaps it.
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bwrap(dir: &Path) -> Command {
    let mut command = Command::new("bwrap");
    command
        .args([
            "--unshare-user",
            "--unshare-pid",
            "--unshare-net",
            "--unshare-ipc",
            "--unshare-uts",
            "--as-pid-1",
            "--die-with-parent",
            "--ro-bind",
            "/",
            "/",
        ])
        .arg("--bind")
        .arg(dir)
        .arg(dir)
        .arg("--");
    command
}

fn serve(mut command: Command, dir: &Path) -> Command {
    let socket = dir.join("shell-9p.sock");
    command
        .arg(env!("CARGO_BIN_EXE_provlita"))
        .arg("--serve")
        .env_clear()
        .env("SOPHIA_SHELL_9P_SOCKET", socket)
        .env(
            "SOPHIA_SHELL_CONFIG",
            concat!(env!("CARGO_MANIFEST_DIR"), "/examples/minimal/config.kdl"),
        )
        .env("SOPHIA_SHELL_BAR_THICKNESS", "48")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn accept(listener: &UnixListener, child: &mut Owned) -> UnixStream {
    listener.set_nonblocking(true).unwrap();
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => panic!("accept: {e}"),
        }
        assert!(child.0.try_wait().unwrap().is_none(), "dock exited early");
        assert!(start.elapsed() < LIMIT, "dock never connected");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Host PID of the process that bwrap started, and its PID in its namespace.
fn sandboxed(bwrap: u32) -> (i32, String) {
    let start = Instant::now();
    loop {
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
                continue;
            };
            let field = |name: &str| {
                status
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_owned()
            };
            if field("PPid:") == bwrap.to_string() {
                let pid = field("Pid:").parse().unwrap();
                let namespace = field("NSpid:").rsplit('\t').next().unwrap().to_owned();
                return (pid, namespace);
            }
        }
        assert!(start.elapsed() < LIMIT, "bwrap child not found");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn signal(pid: i32, signal: libc::c_int) {
    assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
}

fn wait(child: &mut Owned) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    while start.elapsed() < LIMIT {
        if let Some(status) = child.0.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

fn output(child: &mut Owned) -> (String, String) {
    let mut out = String::new();
    let mut err = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    (out, err)
}

fn closed(mut stream: UnixStream) -> bool {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let start = Instant::now();
    let mut chunk = [0; 512];
    while start.elapsed() < LIMIT {
        match stream.read(&mut chunk) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::ConnectionReset => return true,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => panic!("read: {e}"),
        }
    }
    false
}

fn stop_record(out: &str, signal: &str) {
    let expected = format!(
        "provlita_stop schema=1 signal={signal} phase=negotiation connection=none gpu=none"
    );
    assert!(out.contains(&expected), "{out}");
    assert!(out.contains("replay=none"), "{out}");
    assert!(!out.contains("provlita_shell_transport"), "{out}");
}

#[test]
fn sigint_during_negotiation_stops_cleanly_and_closes_the_connection() {
    let dir = Scratch::new("int");
    let listener = UnixListener::bind(dir.0.join("shell-9p.sock")).unwrap();
    let mut child = Owned(serve(Command::new("/usr/bin/env"), &dir.0).spawn().unwrap());
    let stream = accept(&listener, &mut child);
    signal(child.0.id() as i32, libc::SIGINT);
    let status = wait(&mut child).expect("stop exceeded its bound");
    assert!(closed(stream));
    let (out, err) = output(&mut child);
    assert_eq!(status.code(), Some(0), "{err}");
    stop_record(&out, "SIGINT");
}

#[test]
fn sigterm_reaches_the_dock_as_namespace_init_and_stops_it() {
    let dir = Scratch::new("term");
    let listener = UnixListener::bind(dir.0.join("shell-9p.sock")).unwrap();
    let mut child = Owned(serve(bwrap(&dir.0), &dir.0).spawn().unwrap());
    let stream = accept(&listener, &mut child);
    let (pid, namespace) = sandboxed(child.0.id());
    assert_eq!(namespace, "1", "dock is not its namespace's init");
    signal(pid, libc::SIGTERM);
    let status = wait(&mut child).expect("stop exceeded its bound");
    assert!(closed(stream));
    let (out, err) = output(&mut child);
    assert_eq!(status.code(), Some(0), "{err}");
    stop_record(&out, "SIGTERM");
}

/// Negative control: the same delivery path does not stop a namespace init
/// that has no handler, so the case above depends on Provlita's handler.
#[test]
fn namespace_init_without_a_handler_ignores_sigterm() {
    let dir = Scratch::new("control");
    let mut command = bwrap(&dir.0);
    command.args(["sleep", "30"]).stdin(Stdio::null());
    let mut child = Owned(command.spawn().unwrap());
    let (pid, namespace) = sandboxed(child.0.id());
    assert_eq!(namespace, "1");
    signal(pid, libc::SIGTERM);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "SIGTERM stopped init"
    );
    signal(pid, libc::SIGKILL);
    assert!(wait(&mut child).is_some());
}
