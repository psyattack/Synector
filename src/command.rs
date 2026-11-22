//! One bounded subprocess runner and one outcome type for short-lived helper processes.
//!
//! Action helpers, the Lua `ph.run_cmd` bridge and the updater's external tools all spawn a
//! helper program with the same requirements: a wall-clock deadline, captured output that cannot
//! grow the daemon's heap without bound, and a result the caller can turn into its own message.
//! The runner keeps draining a pipe after the capture cap so the child never blocks on a full
//! buffer, kills the child (and optionally its process tree) at the deadline, and reports
//! spawn/wait failures separately so each caller keeps its exact error text.

use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

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
/// for callers whose output contract is byte-exact (the Lua `ph.run_cmd` bridge).
pub(crate) struct CommandOutcome {
    pub(crate) status: Option<ExitStatus>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) timed_out: bool,
    pub(crate) stdout_truncated: bool,
    /// Parity with `stdout_truncated` for callers that surface stderr bytes; the Lua bridge
    /// only marks truncated stdout, so nothing reads this field yet.
    #[allow(dead_code)]
    pub(crate) stderr_truncated: bool,
    stdout_bytes: Vec<u8>,
    stderr_bytes: Vec<u8>,
}

impl CommandOutcome {
    pub(crate) fn succeeded(&self) -> bool {
        !self.timed_out && self.status.is_some_and(|status| status.success())
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
    run_command_bounded_with(cmd, timeout, false)
}

/// Like [`run_command_bounded`], and additionally kills the child's process tree (`taskkill
/// /F /T /PID`) when `kill_tree` is set. A shell command that starts a long-running grandchild
/// would otherwise keep the captured pipes open after the child itself is killed.
pub(crate) fn run_command_bounded_ex(
    cmd: Command,
    timeout: Duration,
    kill_tree: bool,
) -> Result<CommandOutcome, CommandError> {
    run_command_bounded_with(cmd, timeout, kill_tree)
}

fn run_command_bounded_with(
    mut cmd: Command,
    timeout: Duration,
    kill_tree: bool,
) -> Result<CommandOutcome, CommandError> {
    let program = cmd.get_program().to_string_lossy().to_string();
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => return Err(CommandError::Spawn { program, error }),
    };

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
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    break if kill_tree {
                        kill_process_tree(&mut child)
                    } else {
                        let _ = child.kill();
                        child.wait().ok()
                    };
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                if kill_tree {
                    let _ = kill_process_tree(&mut child);
                }
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
    let (stderr_bytes, stderr_truncated) = collect(stderr_rx);
    let stdout = String::from_utf8_lossy(&stdout_bytes).trim().to_string();
    let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();

    Ok(CommandOutcome {
        status,
        stdout,
        stderr,
        timed_out,
        stdout_truncated,
        stderr_truncated,
        stdout_bytes,
        stderr_bytes,
    })
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
}
