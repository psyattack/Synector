use std::cell::Cell;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::ipc::{IpcRequest, IpcResponse, MAX_PAYLOAD_BYTES, PIPE_NAME};

/// Bound for a liveness probe. A daemon that owns the pipe but does not answer within this
/// bound is still running, so a timeout is not treated as "not running".
const PING_TIMEOUT: Duration = Duration::from_millis(750);
/// Bound for a request that is expected to answer immediately (state sync, mutations).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// `TestStep` executes the action inline on the daemon, so it gets room for the default wait
/// timeout (30 s) before the client gives up.
const TEST_STEP_TIMEOUT: Duration = Duration::from_secs(60);
/// A busy pipe is transient: another process may be connecting to the one listening instance.
const CONNECT_ATTEMPTS: u32 = 10;
/// A missing pipe is re-checked briefly: the server creates the replacement listening instance
/// right after accepting a connection, so "not found" for a few milliseconds does not prove the
/// daemon is gone.
const ABSENT_RECHECKS: u32 = 3;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(25);
/// A timed-out worker is actively cancelled; this is how long the caller waits for it to leave its
/// blocking pipe read and be joined before detaching it.
const WORKER_CANCEL_SETTLE: Duration = Duration::from_millis(250);
/// Re-issue interval while waiting for a cancelled worker to finish, so a cancellation that
/// arrived just before the worker entered its read still lands.
const WORKER_CANCEL_POLL: Duration = Duration::from_millis(5);
/// The worker publishes its thread id before its first blocking call; this is the longest the
/// timeout path waits for that id.
const WORKER_ID_WAIT: Duration = Duration::from_millis(50);
/// Total time the GUI exit path may spend on IPC round-trips. Each call is capped to what is left
/// of this budget minus [`WORKER_CANCEL_SETTLE`], and no worker is started once it is spent, so a
/// hung daemon cannot hold the exit for the per-request bound.
const SHUTDOWN_BUDGET: Duration = Duration::from_millis(1500);

thread_local! {
    /// Deadline of the calling thread's GUI-exit budget; `None` outside a shutdown.
    static SHUTDOWN_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PATH_NOT_FOUND: i32 = 3;
const ERROR_PIPE_BUSY: i32 = 231;
const ERROR_SEM_TIMEOUT: i32 = 121;

/// Typed transport failure of one IPC round-trip.
///
/// The distinction between "no daemon" and "daemon busy or slow" matters to callers: only
/// [`IpcError::Unavailable`] is authoritative proof that the daemon is not running, while
/// [`IpcError::Busy`] and [`IpcError::Timeout`] mean it is alive but not answering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcError {
    /// The daemon pipe does not exist (or cannot be opened for the same reason).
    Unavailable(String),
    /// The pipe exists but every connect attempt found it busy.
    Busy(String),
    /// The round-trip exceeded the client bound (or the calling thread's exit budget was spent
    /// before it could start).
    Timeout { timeout: Duration },
    /// A framed payload is larger than [`MAX_PAYLOAD_BYTES`].
    TooLarge(usize),
    /// I/O, encoding or protocol failure.
    Io(String),
}

impl IpcError {
    /// Whether this error is authoritative proof that no daemon is running.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(detail) | Self::Io(detail) => formatter.write_str(detail),
            Self::Busy(detail) => write!(formatter, "The daemon pipe stayed busy: {detail}"),
            Self::Timeout { timeout } => {
                write!(formatter, "The daemon did not answer within {timeout:?}")
            }
            Self::TooLarge(length) => write!(
                formatter,
                "IPC payload is {length} bytes, over the {MAX_PAYLOAD_BYTES} byte limit"
            ),
        }
    }
}

impl std::error::Error for IpcError {}

pub struct IpcClient;

impl IpcClient {
    pub fn is_daemon_running() -> bool {
        match Self::call_typed(&IpcRequest::Ping) {
            Ok(_) => true,
            // A pipe that is busy or accepted the request but did not answer still belongs to a
            // running daemon; only a missing pipe is proof that none is running.
            Err(error) => !error.is_unavailable(),
        }
    }

    /// Starts the calling thread's GUI-exit budget: every later round-trip on this thread is
    /// capped to the remaining budget (minus the cancellation settle window), and once it is spent
    /// no worker is started, so a hung daemon cannot hold a shutdown for the per-request bound.
    ///
    /// The budget is per thread and the first call wins, so `on_exit` and `Drop` share one budget
    /// instead of starting a fresh one each time.
    pub fn begin_shutdown_budget() {
        begin_shutdown_budget_for(SHUTDOWN_BUDGET);
    }

    /// One request/response round-trip. The string error keeps the historical API for the
    /// many call sites that only report it.
    pub fn call(req: &IpcRequest) -> Result<IpcResponse, String> {
        Self::call_typed(req).map_err(|error| error.to_string())
    }

    /// Typed round-trip: [`IpcError::Timeout`] instead of a blocking read that never returns,
    /// and [`IpcError::Unavailable`] versus [`IpcError::Busy`] instead of one opaque string.
    pub fn call_typed(req: &IpcRequest) -> Result<IpcResponse, IpcError> {
        Self::call_pipe(PIPE_NAME, req, timeout_for(req))
    }

    /// Opens one fresh pipe connection per request (the one-connection-per-request contract)
    /// and bounds the whole round-trip with `timeout`.
    ///
    /// The round-trip runs on a worker thread so the caller can stop waiting. When the bound
    /// expires the worker is actively cancelled — its thread id is published before the first
    /// blocking call, `CancelSynchronousIo` aborts the pending pipe I/O, and the caller waits for
    /// the join — so the pipe handle is dropped and a hung daemon cannot accumulate one blocked
    /// thread and handle per timed-out call.
    fn call_pipe(
        pipe_name: &str,
        req: &IpcRequest,
        timeout: Duration,
    ) -> Result<IpcResponse, IpcError> {
        let payload = serde_json::to_vec(req)
            .map_err(|error| IpcError::Io(format!("Cannot encode the IPC request: {error}")))?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(IpcError::TooLarge(payload.len()));
        }
        let Some(timeout) = cap_to_shutdown_budget(timeout, shutdown_remaining()) else {
            return Err(IpcError::Timeout { timeout });
        };

        let pipe_name = pipe_name.to_string();
        let worker_pipe_name = pipe_name.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        let (thread_sender, thread_receiver) = mpsc::sync_channel(1);
        let worker_cancelled = Arc::clone(&cancelled);
        #[cfg(test)]
        let probe_pipe_name = pipe_name.clone();
        let worker = std::thread::Builder::new()
            .name("synector-ipc-client".into())
            .spawn(move || {
                #[cfg(test)]
                let _probe = test_probe::WorkerGuard::new(probe_pipe_name);
                // Publish this thread's identity before the first blocking call, so a timeout on
                // the caller can cancel the read that would otherwise keep the thread stuck.
                let _ = thread_sender.send(worker_thread_id());
                // The receiver may already be gone after a timeout; the result is then dropped.
                let _ = result_sender.send(round_trip(
                    &worker_pipe_name,
                    &payload,
                    &worker_cancelled,
                ));
            })
            .map_err(|error| IpcError::Io(format!("Cannot start the IPC client thread: {error}")))?;

        match result_receiver.recv_timeout(timeout) {
            Ok(result) => {
                join_finished_worker(worker, &pipe_name);
                result
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // The answer can land exactly at the bound; never throw it away.
                if let Ok(result) = result_receiver.try_recv() {
                    join_finished_worker(worker, &pipe_name);
                    return result;
                }
                cancelled.store(true, Ordering::SeqCst);
                let thread = thread_receiver
                    .try_recv()
                    .ok()
                    .or_else(|| thread_receiver.recv_timeout(WORKER_ID_WAIT).ok())
                    .and_then(open_worker_thread);
                settle_cancelled_worker(worker, thread, &pipe_name);
                if let Some(thread) = thread {
                    close_worker_thread(thread);
                }
                Err(IpcError::Timeout { timeout })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                join_finished_worker(worker, &pipe_name);
                Err(IpcError::Io("The IPC client thread ended without a result".into()))
            }
        }
    }
}

fn timeout_for(req: &IpcRequest) -> Duration {
    match req {
        IpcRequest::Ping => PING_TIMEOUT,
        IpcRequest::TestStep { .. } => TEST_STEP_TIMEOUT,
        _ => REQUEST_TIMEOUT,
    }
}

/// Starts the calling thread's exit budget; the first call wins.
fn begin_shutdown_budget_for(budget: Duration) {
    SHUTDOWN_DEADLINE.with(|deadline| {
        if deadline.get().is_none() {
            deadline.set(Some(Instant::now() + budget));
        }
    });
}

#[cfg(test)]
fn clear_shutdown_budget() {
    SHUTDOWN_DEADLINE.with(|deadline| deadline.set(None));
}

/// Remaining exit budget of the calling thread, if a shutdown started one.
fn shutdown_remaining() -> Option<Duration> {
    SHUTDOWN_DEADLINE.with(|deadline| {
        deadline
            .get()
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    })
}

/// Caps a round-trip bound to the calling thread's remaining exit budget, reserving
/// [`WORKER_CANCEL_SETTLE`] for the cancellation of a timed-out worker so the total exit time
/// stays inside the budget. `None` means the budget cannot fund another round-trip.
fn cap_to_shutdown_budget(requested: Duration, remaining: Option<Duration>) -> Option<Duration> {
    let Some(remaining) = remaining else {
        return Some(requested);
    };
    let usable = remaining.saturating_sub(WORKER_CANCEL_SETTLE);
    (!usable.is_zero()).then(|| requested.min(usable))
}

/// Opens a handle to the worker thread with the access right `CancelSynchronousIo` requires.
/// The open handle keeps the thread object pinned, so its id cannot be reused while the caller
/// still may cancel it.
fn open_worker_thread(thread_id: u32) -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::System::Threading::{OpenThread, THREAD_TERMINATE};

    // SAFETY: the id belongs to this process's live worker (it is published before the worker
    // blocks) and the returned handle is closed by the caller.
    unsafe { OpenThread(THREAD_TERMINATE, false, thread_id).ok() }
}

/// Cancels the synchronous pipe I/O the worker is blocked in, so it leaves its read and drops the
/// pipe handle. A thread with no pending I/O is a no-op.
fn cancel_blocking_read(thread: windows::Win32::Foundation::HANDLE) {
    // SAFETY: `thread` is a live thread handle with THREAD_TERMINATE access.
    unsafe {
        let _ = windows::Win32::System::IO::CancelSynchronousIo(thread);
    }
}

/// Closes a handle returned by [`open_worker_thread`].
fn close_worker_thread(thread: windows::Win32::Foundation::HANDLE) {
    // SAFETY: the handle is owned by the caller and closed exactly once.
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(thread);
    }
}

/// Waits for a cancelled worker to leave its blocking I/O and joins it. The cancellation is
/// re-issued because the worker may have been between steps when the first request arrived; the
/// wait is bounded, so a worker that cannot be stopped is detached instead of being awaited.
fn settle_cancelled_worker(
    worker: std::thread::JoinHandle<()>,
    thread: Option<windows::Win32::Foundation::HANDLE>,
    pipe_name: &str,
) {
    let deadline = Instant::now() + WORKER_CANCEL_SETTLE;
    while !worker.is_finished() {
        if Instant::now() >= deadline {
            crate::logging::warn(
                "IPC",
                "A cancelled IPC client worker did not stop in time; detaching one thread",
            );
            probe_worker_detached(pipe_name);
            return;
        }
        if let Some(thread) = thread {
            cancel_blocking_read(thread);
        }
        std::thread::sleep(WORKER_CANCEL_POLL);
    }
    let _ = worker.join();
    probe_worker_joined(pipe_name);
}

/// Joins a worker that has already delivered its result (or ended without one).
fn join_finished_worker(worker: std::thread::JoinHandle<()>, pipe_name: &str) {
    let _ = worker.join();
    probe_worker_joined(pipe_name);
}

#[cfg(test)]
fn probe_worker_joined(pipe_name: &str) {
    test_probe::worker_joined(pipe_name);
}

#[cfg(not(test))]
fn probe_worker_joined(_pipe_name: &str) {}

#[cfg(test)]
fn probe_worker_detached(pipe_name: &str) {
    test_probe::worker_detached(pipe_name);
}

#[cfg(not(test))]
fn probe_worker_detached(_pipe_name: &str) {}

fn worker_thread_id() -> u32 {
    // SAFETY: `GetCurrentThreadId` takes no arguments and cannot fail.
    unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }
}

/// Whether the caller cancelled this round-trip while it was still waiting.
fn round_trip_cancelled(cancelled: &AtomicBool) -> Result<(), IpcError> {
    if cancelled.load(Ordering::SeqCst) {
        Err(IpcError::Io("The IPC round-trip was cancelled after the bound expired".into()))
    } else {
        Ok(())
    }
}

fn round_trip(
    pipe_name: &str,
    payload: &[u8],
    cancelled: &AtomicBool,
) -> Result<IpcResponse, IpcError> {
    let mut pipe = connect(pipe_name, cancelled)?;

    let length = (payload.len() as u32).to_le_bytes();
    round_trip_cancelled(cancelled)?;
    pipe.write_all(&length)
        .map_err(|error| IpcError::Io(format!("Cannot write the IPC request length: {error}")))?;
    round_trip_cancelled(cancelled)?;
    pipe.write_all(payload)
        .map_err(|error| IpcError::Io(format!("Cannot write the IPC request: {error}")))?;
    round_trip_cancelled(cancelled)?;
    pipe.flush()
        .map_err(|error| IpcError::Io(format!("Cannot flush the IPC request: {error}")))?;

    let mut length_buf = [0u8; 4];
    round_trip_cancelled(cancelled)?;
    pipe.read_exact(&mut length_buf)
        .map_err(|error| IpcError::Io(format!("Cannot read the IPC response length: {error}")))?;
    let response_length = u32::from_le_bytes(length_buf) as usize;
    if response_length > MAX_PAYLOAD_BYTES {
        return Err(IpcError::TooLarge(response_length));
    }

    let mut response_buf = vec![0u8; response_length];
    round_trip_cancelled(cancelled)?;
    pipe.read_exact(&mut response_buf)
        .map_err(|error| IpcError::Io(format!("Cannot read the IPC response: {error}")))?;
    serde_json::from_slice(&response_buf)
        .map_err(|error| IpcError::Io(format!("Cannot decode the IPC response: {error}")))
}

/// Opens the daemon pipe, retrying the transient `ERROR_PIPE_BUSY` that a single listening
/// instance can produce when two clients connect at once. A missing pipe is re-checked briefly
/// before it is reported as [`IpcError::Unavailable`], so only a sustained absence selects the
/// offline fallback in callers. A round-trip cancelled by its own timeout stops retrying.
fn connect(pipe_name: &str, cancelled: &AtomicBool) -> Result<std::fs::File, IpcError> {
    let mut absent_rechecks = 0;
    let mut last_error = None;
    for attempt in 0..CONNECT_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(CONNECT_RETRY_DELAY);
        }
        round_trip_cancelled(cancelled)?;
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe_name)
        {
            Ok(pipe) => return Ok(pipe),
            Err(error) if is_pipe_busy(&error) => last_error = Some(error),
            Err(error) if is_daemon_absent(&error) && absent_rechecks < ABSENT_RECHECKS => {
                absent_rechecks += 1;
                last_error = Some(error);
            }
            Err(error) => {
                let detail = format!("Cannot connect to daemon pipe: {error}");
                return if is_daemon_absent(&error) {
                    Err(IpcError::Unavailable(detail))
                } else {
                    Err(IpcError::Io(detail))
                };
            }
        }
    }
    let detail = last_error.map_or_else(
        || "every connect attempt failed".to_string(),
        |error| error.to_string(),
    );
    Err(IpcError::Busy(detail))
}

fn is_pipe_busy(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_PIPE_BUSY | ERROR_SEM_TIMEOUT)
    )
}

fn is_daemon_absent(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
        || matches!(
            error.raw_os_error(),
            Some(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND)
        )
}

/// Live/joined/detached counters for the client workers, keyed by pipe. The cancellation test
/// uses them to pin that a timed-out worker is joined (not detached or leaked) and that a loop of
/// timeouts leaves no live worker behind. Test-only: production builds carry no counters.
#[cfg(test)]
mod test_probe {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    #[derive(Default)]
    struct Counters {
        live: i64,
        joined: u64,
        detached: u64,
    }

    static COUNTERS: OnceLock<Mutex<HashMap<String, Counters>>> = OnceLock::new();

    fn with<T>(pipe_name: &str, update: impl FnOnce(&mut Counters) -> T) -> T {
        let mut counters = COUNTERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(counters.entry(pipe_name.to_string()).or_default())
    }

    /// Marks one worker of `pipe_name` as live for as long as it is held.
    pub(super) struct WorkerGuard(String);

    impl WorkerGuard {
        pub(super) fn new(pipe_name: String) -> Self {
            with(&pipe_name, |counters| counters.live += 1);
            Self(pipe_name)
        }
    }

    impl Drop for WorkerGuard {
        fn drop(&mut self) {
            with(&self.0, |counters| counters.live -= 1);
        }
    }

    pub(super) fn worker_joined(pipe_name: &str) {
        with(pipe_name, |counters| counters.joined += 1);
    }

    pub(super) fn worker_detached(pipe_name: &str) {
        with(pipe_name, |counters| counters.detached += 1);
    }

    /// `(live, joined, detached)` workers for `pipe_name`.
    pub(super) fn snapshot(pipe_name: &str) -> (i64, u64, u64) {
        with(pipe_name, |counters| {
            (counters.live, counters.joined, counters.detached)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc as std_mpsc;
    use std::time::Instant;

    static NEXT_PIPE: AtomicU64 = AtomicU64::new(0);

    fn test_pipe_name(tag: &str) -> String {
        format!(
            r"\\.\pipe\Synector_ClientTest_{}_{}_{tag}",
            process::id(),
            NEXT_PIPE.fetch_add(1, Ordering::Relaxed)
        )
    }

    #[test]
    fn test_silent_handler_times_out_within_the_bound() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        const TIME_OUTS: usize = 5;
        let pipe = test_pipe_name("silent");
        let (ready_sender, ready_receiver) = std_mpsc::channel();
        let server_pipe = pipe.clone();

        // A hung daemon: it accepts a connection, reads the request and never answers, and keeps a
        // listening instance ready for the next request (one connection per request).
        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build the test runtime");
            runtime.block_on(async move {
                let mut server = tokio::net::windows::named_pipe::ServerOptions::new()
                    .first_pipe_instance(true)
                    .create(&server_pipe)
                    .expect("create the test pipe");
                ready_sender.send(()).expect("signal readiness");

                for attempt in 0..=TIME_OUTS {
                    server.connect().await.expect("accept the client");
                    let replacement = tokio::net::windows::named_pipe::ServerOptions::new()
                        .create(&server_pipe)
                        .expect("create the replacement pipe instance");

                    let mut length = [0u8; 4];
                    server.read_exact(&mut length).await.expect("read the request length");
                    let request_length = u32::from_le_bytes(length) as usize;
                    let mut request = vec![0u8; request_length];
                    server.read_exact(&mut request).await.expect("read the request");

                    if attempt < TIME_OUTS {
                        // Never answer this request. The read returns once the client cancels its
                        // blocked read and drops the pipe handle, which is what makes the next
                        // connection possible.
                        let mut byte = [0u8; 1];
                        let _ = server.read(&mut byte).await;
                    } else {
                        // The call after the timeouts must work on a fresh connection.
                        let body = serde_json::to_vec(&IpcResponse::Pong).expect("encode Pong");
                        server
                            .write_all(&(body.len() as u32).to_le_bytes())
                            .await
                            .expect("write the response length");
                        server.write_all(&body).await.expect("write the response");
                        server.flush().await.expect("flush the response");
                    }
                    server = replacement;
                }
            });
        });

        ready_receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the test pipe is ready");

        let timeout = Duration::from_millis(150);
        for attempt in 0..TIME_OUTS {
            let started = Instant::now();
            let error = IpcClient::call_pipe(&pipe, &IpcRequest::Ping, timeout)
                .expect_err("a silent daemon must time out");
            let elapsed = started.elapsed();
            assert!(matches!(error, IpcError::Timeout { .. }), "attempt {attempt}: {error:?}");
            assert!(
                elapsed < Duration::from_secs(1),
                "attempt {attempt}: the timeout bound was not enforced: {elapsed:?}"
            );
            // The timed-out worker was cancelled and joined: neither its thread nor its pipe
            // handle may survive the call.
            assert_eq!(
                test_probe::snapshot(&pipe),
                (0, attempt as u64 + 1, 0),
                "attempt {attempt}: the timed-out worker must be joined, not left running"
            );
        }
        // A loop of timeouts must not leave one permanently blocked thread per timeout behind.
        assert_eq!(test_probe::snapshot(&pipe).0, 0, "timeouts leaked live worker threads");

        let response = IpcClient::call_pipe(&pipe, &IpcRequest::Ping, Duration::from_secs(2))
            .expect("the request after the timeouts must succeed on a fresh connection");
        assert!(
            matches!(response, IpcResponse::Pong),
            "the fresh connection must answer the ping"
        );
        assert_eq!(
            test_probe::snapshot(&pipe),
            (0, TIME_OUTS as u64 + 1, 0),
            "every worker, including the successful one, must be joined"
        );

        server.join().expect("join the test server");
    }

    #[test]
    fn test_shutdown_budget_caps_round_trip_bounds() {
        let requested = Duration::from_secs(5);
        assert_eq!(
            cap_to_shutdown_budget(requested, None),
            Some(requested),
            "outside a shutdown the per-request bound is unchanged"
        );
        assert_eq!(
            cap_to_shutdown_budget(requested, Some(WORKER_CANCEL_SETTLE + requested + Duration::from_secs(1))),
            Some(requested),
            "a budget larger than the request does not extend or shorten it"
        );
        assert_eq!(
            cap_to_shutdown_budget(requested, Some(WORKER_CANCEL_SETTLE + Duration::from_millis(300))),
            Some(Duration::from_millis(300)),
            "a tight budget caps the round-trip"
        );
        assert_eq!(
            cap_to_shutdown_budget(requested, Some(WORKER_CANCEL_SETTLE)),
            None,
            "the cancellation settle window is reserved out of the budget"
        );
        assert_eq!(cap_to_shutdown_budget(requested, Some(Duration::ZERO)), None);
    }

    #[test]
    fn test_spent_shutdown_budget_returns_timeout_without_a_worker() {
        // The budget is per calling thread, so this test cannot affect the other client tests.
        begin_shutdown_budget_for(Duration::ZERO);
        let pipe = test_pipe_name("budget");
        let started = Instant::now();
        let error = IpcClient::call_pipe(&pipe, &IpcRequest::Ping, Duration::from_secs(5))
            .expect_err("a spent shutdown budget must not wait for the bound");
        let elapsed = started.elapsed();
        clear_shutdown_budget();
        assert!(matches!(error, IpcError::Timeout { .. }), "{error:?}");
        assert!(
            elapsed < Duration::from_millis(100),
            "a spent exit budget must not block the caller: {elapsed:?}"
        );
        assert_eq!(
            test_probe::snapshot(&pipe),
            (0, 0, 0),
            "no worker may be started once the exit budget is spent"
        );
    }

    #[test]
    fn test_missing_pipe_is_unavailable() {
        let pipe = test_pipe_name("absent");
        let error = IpcClient::call_pipe(&pipe, &IpcRequest::Ping, Duration::from_millis(200))
            .expect_err("a missing pipe must fail");
        assert!(matches!(error, IpcError::Unavailable(_)), "{error:?}");
        assert!(error.is_unavailable());
        assert!(error.to_string().contains("Cannot connect to daemon pipe"));
    }

    #[test]
    fn test_connect_failures_are_classified() {
        let missing = std::io::Error::from_raw_os_error(ERROR_FILE_NOT_FOUND);
        assert!(is_daemon_absent(&missing));
        assert!(!is_pipe_busy(&missing));

        let busy = std::io::Error::from_raw_os_error(ERROR_PIPE_BUSY);
        assert!(is_pipe_busy(&busy));
        assert!(!is_daemon_absent(&busy));

        let denied = std::io::Error::from_raw_os_error(5);
        assert!(!is_daemon_absent(&denied), "access denied does not prove the daemon is gone");
    }

    #[test]
    fn test_oversized_response_length_is_rejected_before_reading_the_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let pipe = test_pipe_name("oversize");
        let (ready_sender, ready_receiver) = std_mpsc::channel();
        let (stop_sender, stop_receiver) = std_mpsc::channel::<()>();
        let server_pipe = pipe.clone();

        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build the test runtime");
            runtime.block_on(async move {
                let mut server = tokio::net::windows::named_pipe::ServerOptions::new()
                    .first_pipe_instance(true)
                    .create(&server_pipe)
                    .expect("create the test pipe");
                ready_sender.send(()).expect("signal readiness");
                server.connect().await.expect("accept the client");

                let mut length = [0u8; 4];
                server.read_exact(&mut length).await.expect("read the request length");
                let request_length = u32::from_le_bytes(length) as usize;
                let mut request = vec![0u8; request_length];
                server.read_exact(&mut request).await.expect("read the request");

                // Announce a body over the framing cap and never send it: the client must
                // reject the prefix instead of allocating and blocking on the body.
                let oversized = (MAX_PAYLOAD_BYTES as u32 + 1).to_le_bytes();
                server.write_all(&oversized).await.expect("write the response prefix");
                while stop_receiver.try_recv().is_err() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });
        });

        ready_receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the test pipe is ready");
        let error = IpcClient::call_pipe(&pipe, &IpcRequest::Ping, Duration::from_secs(2))
            .expect_err("an oversized response must be rejected");
        assert!(matches!(error, IpcError::TooLarge(_)), "{error:?}");
        assert!(error.to_string().contains("16"));

        stop_sender.send(()).expect("stop the test server");
        server.join().expect("join the test server");
    }
}
