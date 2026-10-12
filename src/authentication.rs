//! Adopt the supervisor's exclusive generation channel before opening a role.
use sophia_9p_client::pipeline::Pipeline;
use sophia_shell_client::{ShellClientOptions, ShellConnection};
use std::{ffi::OsStr, os::fd::FromRawFd, os::unix::net::UnixStream, path::Path, time::Duration};

fn descriptor(value: &OsStr) -> Result<i32, String> {
    let value = value.to_str().ok_or("non-UTF-8 SOPHIA_LAUNCH_AUTH_FD")?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalid SOPHIA_LAUNCH_AUTH_FD".into());
    }
    value
        .parse::<i32>()
        .ok()
        .filter(|fd| *fd >= 3)
        .ok_or_else(|| "SOPHIA_LAUNCH_AUTH_FD must name a non-stdio descriptor".into())
}

fn adopt(fd: i32) -> Result<UnixStream, String> {
    // The launch contract transfers this fd exclusively to the process.
    // Validate before opening anything that could reuse a closed descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(format!(
            "launch channel: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: the validated inherited fd is exclusively owned by this startup
    // path. The environment advertises custody, never principal authority.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(format!(
            "launch channel CLOEXEC: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(stream)
}

pub(super) fn connect(
    path: impl AsRef<Path>,
    options: ShellClientOptions,
    mut stopping: impl FnMut() -> bool,
) -> Result<(ShellConnection, Option<Pipeline>), String> {
    let Some(value) = std::env::var_os("SOPHIA_LAUNCH_AUTH_FD") else {
        // Supplied standalone conformance transport only. Protected Session
        // launches always provision the channel; no error reaches this branch.
        return ShellConnection::connect_files(path, options)
            .map(|connection| (connection, None))
            .map_err(|error| error.to_string());
    };
    let relay = adopt(descriptor(&value)?)?;
    if stopping() {
        return Err("shell stopping".into());
    }
    let role = UnixStream::connect(path).map_err(|error| error.to_string())?;
    let mut auth = sophia_launch_client::Client::with_timeout(
        role,
        relay,
        b"shell",
        b"dock",
        options.handshake_timeout,
    )
    .map_err(|error| error.to_string())?;
    while !auth.ready() {
        if stopping() {
            return Err("shell stopping".into());
        }
        auth.poll().map_err(|error| error.to_string())?;
        if !auth.ready() {
            auth.wait_for_io(Duration::from_millis(50))
                .map_err(|error| error.to_string())?;
        }
    }
    if stopping() {
        return Err("shell stopping".into());
    }
    let admitted = auth
        .into_authenticated()
        .map_err(|error| error.to_string())?;
    let connection = ShellConnection::from_attached(admitted.role, admitted.root, options)
        .map_err(|error| error.to_string())?;
    Ok((connection, Some(admitted.relay)))
}

#[cfg(test)]
#[path = "../tests/support/authentication.rs"]
mod tests;
