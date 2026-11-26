//! One bounded subprocess runner and one outcome type for short-lived helper processes.
//!
//! Action helpers, the Lua `syn.run_cmd` bridge and the updater's external tools all spawn a
//! helper program with the same requirements: a wall-clock deadline, captured output that cannot
//! grow the daemon's heap without bound, and a result the caller can turn into its own message.
//! The runner keeps draining a pipe after the capture cap so the child never blocks on a full
//! buffer, kills the child (and optionally its process tree) at the deadline, and reports
//! spawn/wait failures separately so each caller keeps its exact error text.

use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject,
};

/// Captured bytes per stream of one helper subprocess. The reader keeps draining the
/// pipe after the cap so the child never blocks on a full buffer, but the excess is
/// dropped instead of being buffered in the daemon.
pub(crate) const MAX_CAPTURED_OUTPUT: usize = 64 * 1024;

/// Wall-clock ceiling for short system utilities (taskkill, powercfg, netsh).
pub(crate) const UTILITY_TIMEOUT: Duration = Duration::from_secs(20);

/// Wall-clock ceiling for the updater's external extraction tools (`tar.exe`,
/// `powershell.exe`). These helpers used to run unbounded; a stuck tool must not wedge the
/// daemon's update task.
pub(crate) const TOOL_TIMEOUT: Duration = Duration::from_secs(600);

/// Per-execution kill switch for a script's whole process tree.
///
/// A tracked command runs inside its own Windows Job Object. Every process the script starts
/// joins that job — a grandchild keeps its own name and executable and is still a member — so one
/// [`Self::cancel`] terminates the entire tree. [`ProcessJob`] deliberately does not use
/// kill-on-close: a script that starts a background program and exits keeps the previous
/// behaviour, and only an explicit stop or the action's own deadline terminates the tree.
pub(crate) struct CommandCancellation {
    state: Mutex<CancellationState>,
}

struct CancellationState {
    cancelled: bool,
    job: Option<Arc<ProcessJob>>,
}

impl CommandCancellation {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(CancellationState {
                cancelled: false,
                job: None,
            }),
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// Marks the run stopped and terminates the job immediately. The runner polls the same flag,
    /// so a script that never joined a job still stops at its next poll.
    pub(crate) fn cancel(&self) {
        let mut state = self.lock();
        state.cancelled = true;
        if let Some(job) = &state.job {
            job.terminate();
        }
    }

    /// Publishes the job to the stop path. `Err(job)` means a stop already happened, so the
    /// caller terminates the freshly created job instead of running an unkillable tree.
    fn attach(&self, job: Arc<ProcessJob>) -> Result<(), Arc<ProcessJob>> {
        let mut state = self.lock();
        if state.cancelled {
            return Err(job);
        }
        state.job = Some(job);
        Ok(())
    }

    /// Drops the job reference once the runner has collected the outcome.
    fn detach(&self) {
        self.lock().job = None;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CancellationState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One Windows Job Object owned by a tracked command run.
struct ProcessJob {
    handle: usize,
}

impl ProcessJob {
    fn create() -> Option<Self> {
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.ok()?;
        Some(Self {
            handle: job.0 as usize,
        })
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.handle as *mut std::ffi::c_void)
    }

    /// Puts the freshly spawned direct child into the job. Every later descendant inherits the
    /// membership, so no name or executable matching is involved.
    fn assign(&self, child: &Child) -> bool {
        use std::os::windows::io::AsRawHandle;
        unsafe { AssignProcessToJobObject(self.raw(), HANDLE(child.as_raw_handle())).is_ok() }
    }

    fn terminate(&self) {
        unsafe {
            let _ = TerminateJobObject(self.raw(), 1);
        }
    }
}

impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.raw());
        }
    }
}

/// A spawn or wait failure of a helper subprocess. Callers with their own error contract can
/// read [`CommandError::io_error`]; [`Display`](std::fmt::Display) keeps the action-handler text.
#[derive(Debug)]
pub(crate) enum CommandError {
    Spawn {
        program: String,
        error: std::io::Error,
    },
    Wait {
        program: String,
        error: std::io::Error,
    },
}

impl CommandError {
    /// The underlying I/O error, for callers that format it themselves (the Lua runner).
    pub(crate) fn io_error(&self) -> &std::io::Error {
        match self {
            Self::Spawn { error, .. } | Self::Wait { error, .. } => error,
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn { program, error } => {
                write!(f, "Failed to start '{program}': {error}")
            }
            Self::Wait { program, error } => {
                write!(f, "Failed to wait for '{program}': {error}")
            }
        }
    }
}

impl std::error::Error for CommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.io_error())
    }
}

impl From<CommandError> for String {
    fn from(error: CommandError) -> Self {
        error.to_string()
    }
}

/// Result of a bounded helper subprocess: exit status, capped streams and timeout/truncation
/// flags. `stdout`/`stderr` are the trimmed lossy strings the action handlers consume;
/// [`CommandOutcome::stdout_bytes`]/[`CommandOutcome::stderr_bytes`] expose the untrimmed capture
/// for callers whose output contract is byte-exact (the Lua `syn.run_cmd` bridge).
pub(crate) struct CommandOutcome {
    pub(crate) status: Option<ExitStatus>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) timed_out: bool,
    /// The run was stopped through its [`CommandCancellation`] rather than finishing on its own.
    pub(crate) cancelled: bool,
    pub(crate) stdout_truncated: bool,
    stdout_bytes: Vec<u8>,
    stderr_bytes: Vec<u8>,
}

impl CommandOutcome {
    /// An outcome for a run stopped before it could spawn.
    fn cancelled() -> Self {
        Self {
            status: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            cancelled: true,
            stdout_truncated: false,
            stdout_bytes: Vec::new(),
            stderr_bytes: Vec::new(),
        }
    }

    pub(crate) fn succeeded(&self) -> bool {
        !self.timed_out && !self.cancelled && self.status.is_some_and(|status| status.success())
    }

    pub(crate) fn exit_code(&self) -> Option<i32> {
        self.status.and_then(|status| status.code())
    }

    /// Untrimmed stdout as captured by the reader, capped at [`MAX_CAPTURED_OUTPUT`].
    pub(crate) fn stdout_bytes(&self) -> &[u8] {
        &self.stdout_bytes
    }

    /// Untrimmed stderr as captured by the reader, capped at [`MAX_CAPTURED_OUTPUT`].
    pub(crate) fn stderr_bytes(&self) -> &[u8] {
        &self.stderr_bytes
    }

    /// Maps a non-zero exit or timeout to `"{tool} timed out after {}s"` /
    /// `"{tool} failed with exit code {:?}"`.
    pub(crate) fn ensure_success(&self, tool: &str, timeout: Duration) -> Result<(), String> {
        if self.timed_out {
            return Err(format!("{tool} timed out after {}s", timeout.as_secs()));
        }
        if !self.succeeded() {
            return Err(format!(
                "{tool} failed with exit code {:?}",
                self.exit_code()
            ));
        }
        Ok(())
    }
}

/// Reads a bounded capture. The runner uses [`read_capped_tracked`] internally because it also
/// needs the truncation flag; this helper stays public for tests and other capture-only users.
#[allow(dead_code)]
pub(crate) fn read_capped(mut reader: impl std::io::Read, limit: usize) -> Vec<u8> {
    read_capped_tracked(&mut reader, limit).0
}

fn read_capped_tracked(mut reader: impl std::io::Read, limit: usize) -> (Vec<u8>, bool) {
    let mut collected = Vec::new();
    let mut truncated = false;
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                if collected.len() < limit {
                    let remaining = limit - collected.len();
                    let keep = n.min(remaining);
                    collected.extend_from_slice(&buffer[..keep]);
                    if keep < n {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (collected, truncated)
}

/// Runs one helper subprocess with a wall-clock deadline and bounded capture.
///
/// This is deliberately synchronous: callers in async action handlers use
/// [`run_command_bounded_async`], which moves the wait onto a blocking thread.
pub(crate) fn run_command_bounded(
    cmd: Command,
    timeout: Duration,
) -> Result<CommandOutcome, CommandError> {
    run_command_bounded_with(cmd, timeout, false, false, None)
}

/// Runs a user script with its process tree tracked by a Windows Job Object.
///
/// The job covers every descendant whatever its name or executable. With a [`CommandCancellation`]
/// a stop terminates the whole tree immediately; without one the job still makes the action's own
/// deadline kill the tree instead of only the shell that started it.
pub(crate) fn run_command_bounded_tracked(
    cmd: Command,
    timeout: Duration,
    cancel: Option<&CommandCancellation>,
) -> Result<CommandOutcome, CommandError> {
    // A failed job creation falls back to `taskkill /T`, so the tree is still the unit that dies.
    run_command_bounded_with(cmd, timeout, true, true, cancel)
}

fn run_command_bounded_with(
    mut cmd: Command,
    timeout: Duration,
    kill_tree: bool,
    track_tree: bool,
    cancel: Option<&CommandCancellation>,
) -> Result<CommandOutcome, CommandError> {
    let program = cmd.get_program().to_string_lossy().to_string();
    if cancel.is_some_and(CommandCancellation::is_cancelled) {
        return Ok(CommandOutcome::cancelled());
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut job = if track_tree {
        match ProcessJob::create() {
            Some(job) => Some(Arc::new(job)),
            None => {
                crate::logging::warn(
                    "SCRIPT",
                    "Could not create a job object; a stopped script will not kill its process tree",
                );
                None
            }
        }
    } else {
        None
    };

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => return Err(CommandError::Spawn { program, error }),
    };

    // A tiny window exists between the spawn and the assignment; a script shell has started
    // executing by then only after process creation, so descendants normally inherit membership.
    let mut cancelled = false;
    if let Some(job_ref) = job.as_ref() {
        if job_ref.assign(&child) {
            if let Some(cancel) = cancel {
                if cancel.attach(Arc::clone(job_ref)).is_err() {
                    // The stop landed before the job was published; terminate it in the loop.
                    cancelled = true;
                }
            }
        } else {
            crate::logging::warn(
                "SCRIPT",
                "Could not assign the script to its job object; a stopped script will not kill its process tree",
            );
            job = None;
        }
    }

    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    if let Some(pipe) = child.stdout.take() {
        std::thread::spawn(move || {
            let _ = stdout_tx.send(read_capped_tracked(pipe, MAX_CAPTURED_OUTPUT));
        });
    }
    if let Some(pipe) = child.stderr.take() {
        std::thread::spawn(move || {
            let _ = stderr_tx.send(read_capped_tracked(pipe, MAX_CAPTURED_OUTPUT));
        });
    }

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        if !cancelled && cancel.is_some_and(CommandCancellation::is_cancelled) {
            cancelled = true;
        }
        if cancelled {
            terminate_run(&mut child, job.as_deref(), kill_tree);
            break child.wait().ok();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    terminate_run(&mut child, job.as_deref(), kill_tree);
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                terminate_run(&mut child, job.as_deref(), kill_tree);
                return Err(CommandError::Wait { program, error });
            }
        }
    };

    // Readers normally finish as soon as the child's handles close; the timeout keeps a
    // grandchild that inherited a pipe from wedging the daemon action instead.
    let collect = |rx: mpsc::Receiver<(Vec<u8>, bool)>| {
        rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default()
    };
    let (stdout_bytes, stdout_truncated) = collect(stdout_rx);
    let stderr_bytes = collect(stderr_rx).0;
    let stdout = String::from_utf8_lossy(&stdout_bytes).trim().to_string();
    let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();

    if let Some(cancel) = cancel {
        cancel.detach();
    }

    Ok(CommandOutcome {
        status,
        stdout,
        stderr,
        timed_out,
        cancelled,
        stdout_truncated,
        stdout_bytes,
        stderr_bytes,
    })
}

/// Ends a run: the job kills every descendant, the taskkill fallback covers a failed job, and a
/// plain kill keeps the historical single-process behaviour.
fn terminate_run(child: &mut Child, job: Option<&ProcessJob>, kill_tree: bool) {
    if let Some(job) = job {
        job.terminate();
    } else if kill_tree {
        let _ = kill_process_tree(child);
    } else {
        let _ = child.kill();
    }
}

pub(crate) async fn run_command_bounded_async(
    cmd: Command,
    timeout: Duration,
) -> Result<CommandOutcome, String> {
    tokio::task::spawn_blocking(move || run_command_bounded(cmd, timeout))
        .await
        .map_err(|e| format!("Command task failed: {e}"))?
        .map_err(Into::into)
}

/// Async form of [`run_command_bounded_tracked`] for script action handlers.
pub(crate) async fn run_command_bounded_tracked_async(
    cmd: Command,
    timeout: Duration,
    cancel: Option<Arc<CommandCancellation>>,
) -> Result<CommandOutcome, String> {
    tokio::task::spawn_blocking(move || {
        run_command_bounded_tracked(cmd, timeout, cancel.as_deref())
    })
    .await
    .map_err(|e| format!("Command task failed: {e}"))?
    .map_err(Into::into)
}

/// Runs a short system utility and maps a non-zero exit or timeout to an action error.
pub(crate) async fn run_system_tool(program: &str, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    apply_no_window(&mut cmd);
    let out = run_command_bounded_async(cmd, UTILITY_TIMEOUT).await?;
    out.ensure_success(program, UTILITY_TIMEOUT)
}

/// Kills `child` and any process it started. Killing only the shell would leave a
/// long-running grandchild holding the stdout/stderr pipes open.
fn kill_process_tree(child: &mut Child) -> Option<ExitStatus> {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/F", "/T", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::win_args::apply_no_window(&mut cmd);
        let _ = cmd.status();
    }
    let _ = child.kill();
    child.wait().ok()
}

pub(crate) fn apply_no_window(cmd: &mut Command) {
    crate::win_args::apply_no_window(cmd);
}

pub(crate) fn powershell_command(script: &str) -> Command {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    apply_no_window(&mut cmd);
    cmd
}

pub(crate) fn powershell_helper_command(script: &str) -> Command {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    apply_no_window(&mut cmd);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn test_spawn_error_keeps_the_action_error_text() {
        let program = "synector_missing_command_for_the_runner_test";
        let error = match run_command_bounded(Command::new(program), Duration::from_secs(1)) {
            Ok(_) => panic!("a missing program must not spawn"),
            Err(error) => error,
        };
        let text = error.to_string();
        assert!(
            text.starts_with(&format!("Failed to start '{program}': ")),
            "{text}"
        );
        assert_eq!(String::from(error), text);
    }

    #[test]
    fn test_a_pre_cancelled_tracked_run_never_spawns() {
        let cancel = CommandCancellation::new();
        cancel.cancel();
        let outcome = run_command_bounded_tracked(
            Command::new("synector_never_spawned_marker"),
            Duration::from_secs(1),
            Some(&cancel),
        )
        .expect("a pre-cancelled run returns an outcome");
        assert!(outcome.cancelled);
        assert!(outcome.status.is_none(), "nothing could have been spawned");
    }

    /// A script that starts a grandchild with a different executable must lose the whole tree when
    /// its step test is stopped: the Job Object tracks process membership, not names or binaries.
    #[test]
    fn test_cancelling_a_tracked_script_kills_its_grandchild() {
        static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "synector_job_tree_{}_{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create the test directory");
        let script_path = dir.join("tree_test.ps1");
        let pid_path = dir.join("pids.txt");
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let child_exe = format!("{system_root}\\System32\\cmd.exe");
        let script = format!(
            "$child = Start-Process -FilePath '{}' -ArgumentList '/C','ping -n 60 127.0.0.1 > nul' -PassThru -WindowStyle Hidden\r\n\
             \"$PID $($child.Id)\" | Set-Content -Path '{}'\r\n\
             Start-Sleep -Seconds 60\r\n",
            child_exe.replace('\'', "''"),
            pid_path.display().to_string().replace('\'', "''"),
        );
        std::fs::write(&script_path, script).expect("write the test script");

        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            script_path.to_str().expect("utf-8 script path"),
        ]);
        apply_no_window(&mut cmd);

        let cancel = Arc::new(CommandCancellation::new());
        let runner_cancel = Arc::clone(&cancel);
        let runner = std::thread::spawn(move || {
            run_command_bounded_tracked(
                cmd,
                Duration::from_secs(120),
                Some(runner_cancel.as_ref()),
            )
        });

        let (shell_pid, grandchild_pid) = wait_for_reported_pids(&pid_path);
        assert!(
            is_process_alive(shell_pid),
            "the script shell must be running before the stop"
        );
        assert!(
            is_process_alive(grandchild_pid),
            "the grandchild must be running before the stop"
        );
        cancel.cancel();
        let outcome = runner.join().expect("runner thread").expect("tracked run");
        assert!(outcome.cancelled, "the stopped run must report cancellation");
        wait_until_dead(shell_pid, "the script shell");
        wait_until_dead(grandchild_pid, "the script's grandchild");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn wait_for_reported_pids(path: &std::path::Path) -> (u32, u32) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(text) = std::fs::read_to_string(path) {
                let mut parts = text.split_whitespace();
                if let (Some(shell), Some(grandchild)) = (parts.next(), parts.next()) {
                    if let (Ok(shell), Ok(grandchild)) = (shell.parse(), grandchild.parse()) {
                        return (shell, grandchild);
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "the test script never reported its process tree"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_until_dead(pid: u32, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while is_process_alive(pid) {
            assert!(
                Instant::now() < deadline,
                "{label} {pid} survived the job termination"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn is_process_alive(pid: u32) -> bool {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // STILL_ACTIVE; the process object also reports it for a live process.
        const STILL_ACTIVE: u32 = 259;
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut code = 0u32;
            let alive = GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE;
            let _ = CloseHandle(handle);
            alive
        }
    }
}
