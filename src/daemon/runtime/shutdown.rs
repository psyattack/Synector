use std::sync::{Condvar, Mutex as StdMutex};

/// Coordinates a daemon shutdown request with the main thread.
///
/// When the tray cannot be created the daemon runs headless, so there is no message loop that can
/// end and no place to fall through to the shutdown sequence. The main thread waits on this
/// signal instead of parking forever; `request_shutdown()` makes it proceed to On App Close rules
/// and the final runtime/overlay persistence.
pub(crate) struct ShutdownSignal {
    requested: StdMutex<bool>,
    signal: Condvar,
}

impl ShutdownSignal {
    pub(crate) const fn new() -> Self {
        Self {
            requested: StdMutex::new(false),
            signal: Condvar::new(),
        }
    }

    pub(crate) fn request(&self) {
        let mut requested = self
            .requested
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *requested = true;
        self.signal.notify_all();
    }

    pub(crate) fn wait(&self) {
        let mut requested = self
            .requested
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*requested {
            requested = self
                .signal
                .wait(requested)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

static DAEMON_SHUTDOWN: ShutdownSignal = ShutdownSignal::new();

/// Requests a coordinated daemon shutdown. Safe to call from any thread and more than once; the
/// main thread then runs the On App Close rules and final persistence before the process exits.
pub fn request_shutdown() {
    DAEMON_SHUTDOWN.request();
    // The tray message loop blocks in `GetMessageW` and only re-checks its run flag after a
    // message, so a shutdown requested from another thread (the IPC `ShutdownDaemon` handler) has
    // to stop the flag and wake the loop. Hiding the icon posts that wake-up message and is a
    // no-op for a process that is already exiting; in headless mode there is no window at all.
    crate::daemon::tray::RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    crate::daemon::tray::set_tray_visible(false);
}

pub(super) fn wait_for_shutdown_request() {
    DAEMON_SHUTDOWN.wait();
}

/// Runs the post-loop graceful shutdown steps in their fixed order: On App Close rules stop the
/// engine, then the overlays close synchronously (so their fade-out plays instead of the
/// kill-on-close Job Object terminating them), and only then the coalesced runtime counters and
/// overlay positions are flushed. Kept as one sequence so the order cannot drift.
pub(crate) fn run_shutdown_sequence(
    stop_engine: impl FnOnce(),
    hide_overlays: impl FnOnce(),
    flush_state: impl FnOnce(),
) {
    stop_engine();
    hide_overlays();
    flush_state();
}
