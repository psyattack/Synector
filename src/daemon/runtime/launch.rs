use std::path::PathBuf;
use std::sync::Mutex as StdMutex;

/// Executable of the GUI that spawned this daemon (`--gui-exe <path>`), when it reported one.
///
/// The updater replaces this executable for a portable installation with process masking: the
/// daemon then runs from the masked copy in `%LOCALAPPDATA%\Synector\bin`, which is only a cache,
/// and the GUI window may already be closed when the update starts.
static LAUNCHING_GUI_EXE: StdMutex<Option<PathBuf>> = StdMutex::new(None);

/// Parses the `--gui-exe <path>` argument. An absent flag or an empty value means "unknown" and
/// leaves the updater with its GUI-window lookup.
pub(crate) fn parse_gui_exe_argument(args: &[String]) -> Option<PathBuf> {
    let position = args
        .iter()
        .position(|arg| arg == crate::arg_spellings::GUI_EXE)?;
    let path = PathBuf::from(args.get(position + 1)?);
    (!path.as_os_str().is_empty()).then_some(path)
}

/// Records the launching GUI executable once, at daemon start-up.
pub(super) fn record_launching_gui_exe(args: &[String]) {
    let Some(path) = parse_gui_exe_argument(args) else {
        return;
    };
    let mut lock = LAUNCHING_GUI_EXE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *lock = Some(path);
}

/// The executable the updater must replace when this daemon runs as the masked copy.
///
/// The path is recorded as the GUI passed it; the consumer validates that it still exists before
/// replacing it. `None` when the daemon was not spawned by the GUI (`--daemon` directly or by
/// autostart).
pub fn launching_gui_exe() -> Option<PathBuf> {
    LAUNCHING_GUI_EXE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}
