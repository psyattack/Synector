//! Command-line rule authoring: validate, apply, list, export and delete rules without the GUI.
//!
//! Rule files are plain JSON in the [`crate::models::Rule`] serde shape: one rule object, an array
//! of rules, or `{"rules": [...]}`. Applying upserts by `id`; when the daemon is running the change
//! goes through IPC so it takes effect immediately, otherwise `rules.json` is edited directly.
//! Runtime counters stay in the daemon-owned `runtime.json`, never in rule files.

mod args;
mod commands;
mod discovery;
mod json;
mod rules_io;
mod share;
mod target;

pub use self::args::USAGE;

#[cfg(test)]
pub(crate) use self::args::command_spellings;

use crate::app::console::ensure_console;

use self::args::{has_unresolved_rule_command, is_known_flag, log_invocation};
use self::commands::dispatch;

/// Entry point from `main`. Returns `false` when the arguments are not a CLI authoring command.
pub fn run(args: &[String]) -> bool {
    if !args.iter().any(|arg| is_known_flag(arg)) {
        // A mistyped `--rules-*` command must not fall through to update cleanup, UAC
        // self-elevation and a daemon/GUI start: fail closed with usage instead ([U13-001]).
        if has_unresolved_rule_command(args) {
            ensure_console();
            crate::logging::error("CLI", "Unknown rule command");
            eprintln!("error: unknown rule command");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
        return false;
    }
    ensure_console();
    let command = log_invocation(args);
    let exit_code = dispatch(args);
    if exit_code == 0 {
        crate::logging::info("CLI", &format!("{command} completed"));
    } else {
        crate::logging::error("CLI", &format!("{command} exited with code {exit_code}"));
    }
    std::process::exit(exit_code)
}
