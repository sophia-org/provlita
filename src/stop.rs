//! Process stop requests. The handler only records the first signal; owners
//! poll it at their bounded turn boundaries. A PID-namespace init (bwrap
//! `--as-pid-1`) receives SIGTERM or SIGINT from ancestor namespaces only once a
//! handler exists, so serving installs these before any startup work.
use std::sync::atomic::{AtomicI32, Ordering};

static REQUESTED: AtomicI32 = AtomicI32::new(0);

extern "C" fn record(signal: libc::c_int) {
    // Async-signal-safe: one atomic store, first request wins.
    let _ = REQUESTED.compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst);
}

/// The signal that requested this process stop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopSignal {
    /// SIGTERM, as sent by Session's component supervisor.
    Terminate,
    /// SIGINT, as sent by an interactive terminal.
    Interrupt,
}
impl StopSignal {
    /// Stable record name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Terminate => "SIGTERM",
            Self::Interrupt => "SIGINT",
        }
    }
}

/// Evidence that this process records SIGTERM and SIGINT instead of taking
/// their default action. A stop request never interrupts a turn; it is
/// observed at the next turn boundary.
pub struct StopRequests(());
impl StopRequests {
    /// Install both handlers and unblock both signals on the calling thread.
    /// Call on the main thread before spawning others so they inherit the mask.
    pub fn install() -> Result<Self, String> {
        // SAFETY: the handler performs one atomic operation. The sigaction and
        // sigset values are fully initialized before the kernel reads them.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            for signal in [libc::SIGTERM, libc::SIGINT] {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = record as extern "C" fn(libc::c_int) as libc::sighandler_t;
                // Restart interruptible calls in GPU and file code; every
                // owner loop already polls at bounded intervals.
                action.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut action.sa_mask);
                if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                    return Err(format!(
                        "install stop handler: {}",
                        std::io::Error::last_os_error()
                    ));
                }
                libc::sigaddset(&mut set, signal);
            }
            let error = libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
            if error != 0 {
                return Err(format!(
                    "unblock stop signals: {}",
                    std::io::Error::from_raw_os_error(error)
                ));
            }
        }
        Ok(Self(()))
    }
    /// The first recorded stop request, if any.
    pub fn requested(&self) -> Option<StopSignal> {
        match REQUESTED.load(Ordering::SeqCst) {
            libc::SIGTERM => Some(StopSignal::Terminate),
            libc::SIGINT => Some(StopSignal::Interrupt),
            _ => None,
        }
    }
}
