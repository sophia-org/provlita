//! Explicit protected-process entry. Ordinary configuration checks never enter.
//! SIGTERM/SIGINT handlers are installed before any startup work. A stop is
//! observed at the next bounded turn: negotiation (at most its handshake
//! timeout), GPU admission, or a service turn. It closes the connection, joins
//! the GPU worker within the same bounded deadline and replays nothing.
use provlita::{
    config::Config,
    render::{GpuGrant, GpuWorker},
    service::{DockService, StopReport},
    stop::{StopRequests, StopSignal},
};
use sophia_shell_client::{ShellClientOptions, ShellConnection};
use sophia_shell_protocol::*;
use std::{
    io::Read,
    time::{Duration, Instant},
};

// Sophia@bc23ee5b crates/sophia-protocol/src/packets/shell_launcher.rs. The
// standalone shell SDK requires this bit in the dock mask but does not export
// its name.
const SOPHIA_SHELL_CAPABILITY_APPLICATION_CATALOG: u64 = 1 << 5;

pub fn run() -> Result<(), String> {
    let stop = StopRequests::install()?;
    if std::env::var_os("SOPHIA_SHELL_SOCKET").is_some() {
        return Err("SOPHIA_SHELL_SOCKET is unsupported; use SOPHIA_SHELL_9P_SOCKET".into());
    }
    let socket = std::env::var_os("SOPHIA_SHELL_9P_SOCKET")
        .filter(|path| !path.is_empty())
        .ok_or("SOPHIA_SHELL_9P_SOCKET is required")?;
    let config =
        std::env::var_os("SOPHIA_SHELL_CONFIG").ok_or("SOPHIA_SHELL_CONFIG is required")?;
    let mut text = String::new();
    std::fs::File::open(config)
        .map_err(|e| e.to_string())?
        .take(provlita::config::MAX_CONFIG_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    let config = Config::parse(&text)?;
    let allowance: u32 = std::env::var("SOPHIA_SHELL_BAR_THICKNESS")
        .map_err(|_| "explicit dock thickness grant is required")?
        .parse()
        .map_err(|_| "invalid dock thickness grant")?;
    if allowance == 0 {
        return Err("zero dock allowance".into());
    }
    if let Some(signal) = stop.requested() {
        return stopped(signal, "configuration", "none", StopReport::default(), None);
    }
    let connected = ShellConnection::connect_files(
        socket,
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
    );
    // A stop during the blocking handshake takes precedence over its outcome;
    // any established connection is dropped (closed) before reporting.
    if let Some(signal) = stop.requested() {
        if let Err(error) = &connected {
            eprintln!("provlita: negotiation ended by stop: {error}");
        }
        let connection = if connected.is_ok() { "closed" } else { "none" };
        drop(connected);
        return stopped(
            signal,
            "negotiation",
            connection,
            StopReport::default(),
            None,
        );
    }
    let mut connection = connected.map_err(|e| format!("dock negotiation: {e}"))?;
    println!(
        "provlita_shell_transport schema=1 wire=9p2000.L revision={} epoch={}",
        connection.welcome().selected_revision,
        connection.connection_epoch()
    );
    let mut worker = GpuWorker::start(GpuGrant::from_environment(connection.connection_epoch())?)?;
    let startup = (|| {
        loop {
            if let Some(signal) = stop.requested() {
                return Ok(Some(signal));
            }
            connection.poll_io().map_err(|e| e.to_string())?;
            if let Some(evidence) = worker.poll_ready()? {
                println!("{}", evidence.record("provlita")?);
                return Ok::<_, String>(None);
            }
            std::thread::sleep(Duration::from_millis(4));
        }
    })();
    match startup {
        Ok(None) => {}
        Ok(Some(signal)) => {
            drop(connection);
            return stopped(
                signal,
                "gpu-admission",
                "closed",
                StopReport::default(),
                Some(worker),
            );
        }
        Err(error) => return finish(worker, error),
    }
    let mut service = DockService::new(connection, config, allowance, worker)?;
    let error = loop {
        if let Some(signal) = stop.requested() {
            let (report, worker) = service.stop();
            return stopped(signal, "serving", "closed", report, Some(worker));
        }
        match service.step() {
            Ok(_) => std::thread::sleep(service.idle_wait()),
            Err(error) => break error,
        }
    };
    finish(service.into_renderer(), error)
}
/// Report a requested stop after any connection is closed. Success is claimed
/// only when the GPU worker thread is joined (or never started).
fn stopped(
    signal: StopSignal,
    phase: &str,
    connection: &str,
    report: StopReport,
    worker: Option<GpuWorker>,
) -> Result<(), String> {
    let gpu = match worker {
        None => "none",
        Some(worker) => match join(worker) {
            Ok(()) => "joined",
            Err(error) => {
                return Err(format!("stopped by {} in {phase}; {error}", signal.name()));
            }
        },
    };
    println!(
        "provlita_stop schema=1 signal={} phase={phase} connection={connection} gpu={gpu} sent_activations={} unsent_replies={} unsettled_submissions={} rendering={} replay=none",
        signal.name(),
        report.sent_activations,
        report.unsent_replies,
        report.unsettled_submissions,
        u8::from(report.rendering),
    );
    Ok(())
}
fn finish(worker: GpuWorker, error: String) -> Result<(), String> {
    match join(worker) {
        Ok(()) => Err(error),
        Err(join) => Err(format!("{error}; {join}")),
    }
}
fn join(mut worker: GpuWorker) -> Result<(), String> {
    worker.request_shutdown();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        match worker.poll_shutdown() {
            Ok(true) => return Ok(()),
            Ok(false) => std::thread::sleep(Duration::from_millis(4)),
            Err(join) => return Err(format!("GPU shutdown: {join}")),
        }
    }
    Err("GPU worker still unresolved at shutdown deadline; no clean completion claimed".into())
}
