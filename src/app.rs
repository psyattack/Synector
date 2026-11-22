//! Process entry: one argv becomes exactly one runtime mode.
//!
//! [`run`] is the only entry point `main` calls. The stage sequence itself lives in
//! [`dispatch`], so the process contract is readable in one place, while the other modules own
//! the argument vocabulary ([`args`]), daemon start-up ([`spawn`]), self-elevation
//! ([`elevation`]), console reattachment ([`console`]), update-file cleanup ([`cleanup`]),
//! `--status` ([`status`]) and `--run-rule` ([`run_rule`]).

// `args` and `console` are crate-visible because `shortcut.rs` quotes through
// `app::args::quote_windows_argument` and `cli.rs` reattaches the console through
// `app::console::ensure_console`; the rest are private to `app`.
pub(crate) mod args;
pub(crate) mod console;

mod cleanup;
mod dispatch;
mod elevation;
mod run_rule;
mod spawn;
mod status;

/// Runs the process through the frozen dispatch stages.
pub(crate) fn run() {
    dispatch::run();
}
