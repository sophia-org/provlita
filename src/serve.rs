//! Explicit protected-process entry. Ordinary configuration checks never enter.
use provlita::{
    config::Config,
    render::{GpuGrant, GpuWorker},
    service::DockService,
};
use sophia_protocol::*;
use sophia_shell_client::{ShellClientOptions, ShellConnection};
use std::{
    io::Read,
    time::{Duration, Instant},
};

pub fn run() -> Result<(), String> {
    let socket =
        std::env::var_os("SOPHIA_SHELL_SOCKET").ok_or("SOPHIA_SHELL_SOCKET is required")?;
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
    let mut connection = ShellConnection::connect(
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
    )
    .map_err(|e| format!("dock negotiation: {e}"))?;
    let mut worker = GpuWorker::start(GpuGrant::from_environment(connection.connection_epoch())?)?;
    let startup = (|| {
        loop {
            connection.poll_io().map_err(|e| e.to_string())?;
            if let Some(evidence) = worker.poll_ready()? {
                println!("{}", evidence.record("provlita")?);
                return Ok::<_, String>(());
            }
            std::thread::sleep(Duration::from_millis(4));
        }
    })();
    if let Err(error) = startup {
        return finish(worker, error);
    }
    let mut service = DockService::new(connection, config, allowance, worker)?;
    let error = loop {
        match service.step() {
            Ok(_) => std::thread::sleep(Duration::from_millis(4)),
            Err(error) => break error,
        }
    };
    finish(service.into_renderer(), error)
}
fn finish(mut worker: GpuWorker, error: String) -> Result<(), String> {
    worker.request_shutdown();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        match worker.poll_shutdown() {
            Ok(true) => return Err(error),
            Ok(false) => std::thread::sleep(Duration::from_millis(4)),
            Err(join) => return Err(format!("{error}; GPU shutdown: {join}")),
        }
    }
    Err(format!(
        "{error}; GPU worker still unresolved at shutdown deadline; no clean completion claimed"
    ))
}
