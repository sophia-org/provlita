//! Readiness-driven owner waits over poll(2). An owner sleeps until one of its
//! descriptors is ready, its earliest monotonic deadline passes or a signal
//! interrupts the wait; it never sleeps a fixed interval to discover work.
use std::{
    os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
    time::Instant,
};

/// Why a wait returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Wake {
    /// At least one descriptor reported readiness (or an error condition).
    Ready,
    /// The earliest deadline passed with nothing ready.
    Expired,
    /// A signal interrupted the wait; the owner re-checks its stop request.
    Interrupted,
}

/// One wait's descriptor set and earliest deadline.
#[derive(Default)]
pub struct Wait {
    fds: Vec<libc::pollfd>,
    deadline: Option<Instant>,
}
impl Wait {
    /// An empty set with no deadline.
    pub fn new() -> Self {
        Self::default()
    }
    /// Wake when `fd` is readable.
    pub fn readable(&mut self, fd: BorrowedFd<'_>) -> &mut Self {
        self.events(fd, true, false)
    }
    /// Wake on the requested readiness of `fd`; error conditions always wake.
    pub fn events(&mut self, fd: BorrowedFd<'_>, readable: bool, writable: bool) -> &mut Self {
        let mut events = 0;
        if readable {
            events |= libc::POLLIN;
        }
        if writable {
            events |= libc::POLLOUT;
        }
        self.fds.push(libc::pollfd {
            fd: fd.as_raw_fd(),
            events,
            revents: 0,
        });
        self
    }
    /// Return no later than `deadline`; the earliest of several wins.
    pub fn until(&mut self, deadline: Option<Instant>) -> &mut Self {
        self.deadline = match (self.deadline, deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self
    }
    /// Block once. With no deadline and no descriptor the wait would never
    /// end, so that combination is refused.
    pub fn block(&mut self) -> Result<Wake, String> {
        let timeout: libc::c_int = if let Some(deadline) = self.deadline {
            // Round up so a sub-millisecond remainder sleeps instead of spinning.
            deadline
                .saturating_duration_since(Instant::now())
                .as_micros()
                .div_ceil(1_000)
                .min(libc::c_int::MAX as u128) as libc::c_int
        } else if self.fds.is_empty() {
            return Err("wait without descriptors or deadline".into());
        } else {
            -1
        };
        // SAFETY: `fds` is a valid, exclusively borrowed pollfd array of the given length.
        let ready = unsafe {
            libc::poll(
                self.fds.as_mut_ptr(),
                self.fds.len() as libc::nfds_t,
                timeout,
            )
        };
        if ready > 0 {
            if self.fds.iter().any(|fd| fd.revents & libc::POLLNVAL != 0) {
                return Err("wait names a closed descriptor".into());
            }
            return Ok(Wake::Ready);
        }
        if ready == 0 {
            return Ok(Wake::Expired);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINTR) {
            return Ok(Wake::Interrupted);
        }
        Err(format!("poll: {error}"))
    }
}

/// A nonblocking, close-on-exec event counter for cross-thread and
/// signal-handler wakes. Writing is async-signal-safe.
pub fn eventfd() -> Result<OwnedFd, String> {
    // SAFETY: eventfd takes no pointers; a negative return is checked before adoption.
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if fd < 0 {
        return Err(format!("eventfd: {}", std::io::Error::last_os_error()));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}
/// Add one to the counter; the result is deliberately ignored so a saturated
/// or closed counter cannot fail a signal handler or worker thread. Safe inside
/// a signal handler: `write` is async-signal-safe and errno is preserved.
pub fn signal(fd: libc::c_int) {
    let one: u64 = 1;
    // SAFETY: `one` is a valid 8-byte buffer for the duration of the call, and
    // errno is thread-local storage that this thread may read and restore.
    unsafe {
        let errno = libc::__errno_location();
        let saved = *errno;
        while libc::write(fd, (&one as *const u64).cast(), 8) < 0 && *errno == libc::EINTR {}
        *errno = saved;
    }
}
/// Consume the counter so a later wait sleeps again. Call before draining the
/// work the counter announces, so a wake racing the drain is never lost.
pub fn clear(fd: BorrowedFd<'_>) {
    let mut value: u64 = 0;
    // SAFETY: `value` is a valid 8-byte buffer; EAGAIN on an empty counter is expected.
    unsafe {
        libc::read(fd.as_raw_fd(), (&mut value as *mut u64).cast(), 8);
    }
}
