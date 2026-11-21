# Contributing to Synector

Thanks for taking the time to improve Synector. This document covers the build, the code
conventions and the documentation rules that keep the project consistent.

## Before you start

- **Platform:** Windows 10 or 11, x64. Synector has no non-Windows fallback and does not need one.
- **Toolchain:** a recent stable Rust toolchain. The project does not pin an MSRV; the current
  `eframe`/`egui` 0.37 dependencies need Rust 1.95 or newer.
- **Installer work:** Inno Setup is only needed if you change `installer/synector.iss`. CI compiles
  it; local builds of the application do not require it.

## Build and test

```powershell
git clone https://github.com/psyattack/Synector.git
cd Synector

cargo check
cargo test
cargo clippy -- -D warnings
cargo fmt                      # optional local formatting; CI does not check it
```

`cargo clippy -- -D warnings` is the CI gate: warnings are errors. Unit tests live next to the code
in `#[cfg(test)] mod tests`; there is no `tests/` directory. Run a single module with
`cargo test <substring>` (for example `cargo test models::tests`).

Run modes:

```powershell
cargo run                    # spawn the daemon if absent, then open the GUI
cargo run -- --gui           # GUI only (starts the daemon if it is not running)
cargo run -- --daemon        # daemon/tray only
cargo run -- --status        # print daemon status over IPC and exit
```

## Where changes belong

| Area | Files |
|---|---|
| Domain types, serde tags, JSON schema | `src/models.rs` |
| Scheduling, quotas, cooldowns, active executions | `src/daemon/engine.rs` |
| Rule-to-rule calls and stops | `src/daemon/rule_call.rs` |
| Action execution handlers | `src/daemon/actions.rs` |
| Process detection | `src/daemon/watcher.rs` |
| File and directory watching | `src/daemon/file_watcher.rs` |
| Hotkeys | `src/daemon/hotkeys.rs` |
| System events and state | `src/daemon/system_events.rs` |
| System telemetry, media and key visualizer | `src/daemon/telemetry.rs` |
| Tray icon, menu and notifications | `src/daemon/tray.rs` |
| Overlay process lifecycle and placement | `src/daemon/overlay_manager.rs`, `src/gui/overlay_window.rs` |
| Webhook trigger server | `src/daemon/webhook_server.rs` |
| Lua runtime and the `ph` API | `src/daemon/lua_runner.rs` |
| Discord Rich Presence | `src/daemon/discord_rpc.rs` |
| Named-pipe server | `src/daemon/ipc_server.rs` |
| GUI state, drafts, IPC calls | `src/gui/state.rs` |
| Views and inspectors | `src/gui/views/` |
| Shared widgets and renderers | `src/gui/components/` |
| Localization | `src/i18n.rs`, `src/i18n/en.rs`, `src/i18n/ru.rs` |
| In-app guide | `src/gui/views/guide_modal.rs` |
| CLI rule authoring | `src/cli.rs` |
| IPC protocol | `src/ipc/protocol.rs` |
| Configuration persistence | `src/config.rs` |
| Share codes | `src/sharing.rs` |
| Autostart registration | `src/autorun.rs` |
| Updates and self-replacement | `src/updater.rs` |
| Logging | `src/logging.rs` |
| Single-instance guards | `src/single_instance.rs` |

Adding or changing a trigger, action or condition touches `models.rs`, the engine and action
handlers, the catalog/inspector GUI views, and both i18n catalogs. Treat `models.rs` as the single
source of truth: the JSON schema printed by `--rules-schema` is generated from it with `schemars`,
so the CLI format cannot drift from the real types.

## Code conventions

- Windows-only code: use `windows-rs`; no portability layer or non-Windows fallback.
- No legacy shims: remove obsolete variants, fields and paths instead of keeping compatibility
  wrappers.
- LF line endings are enforced by `.gitattributes`; keep them.
- Log identifiers and outcomes only. Never log script bodies, webhook payloads, rule JSON or other
  user secrets. Action errors that may embed script output go through the engine's `SensitiveLog`
  event.
- Input simulation must keep hardware scan codes (`MapVirtualKeyW` + `KEYEVENTF_SCANCODE`, plus
  `KEYEVENTF_EXTENDEDKEY` for extended keys) and hold times of at least 30–35 ms so DirectInput
  games register presses.
- Macro recording uses `WH_KEYBOARD_LL`/`WH_MOUSE_LL` only while recording.

## Tests

Add a unit test next to the code you change. Cover the behavior, not the implementation detail.
For rule-model changes, test serialization/deserialization and the quota/counter semantics as well
as the happy path. For GUI-only changes, at minimum verify `cargo check` and describe the manual
steps in the pull request.

## Documentation and localization

Documentation has one owner per topic. The document set and its owners are listed once, in
[docs/README.md](docs/README.md); update the owning file and link to it instead of copying its content
elsewhere.

User-visible GUI strings belong to `Strings` in `src/i18n.rs`, with values in `src/i18n/en.rs` and
`src/i18n/ru.rs`; views must not embed literals. The in-app guide is the documented exception: its
long prose uses Russian/English `if lang == Language::Russian { ... } else { ... }` branches in
`src/gui/views/guide_modal.rs`, and both branches must be updated together. Chrome labels in the
guide still come from `Strings`.

When dependencies change, update the dependency table and license texts in `NOTICES.md`. When CLI
commands or quota behavior change, update `docs/RULE_AUTHORING.md`. When subsystems change, update
`docs/ARCHITECTURE.md`. When behavior users ask about changes, update `docs/FAQ.md`.

Division of ownership: this file owns the contribution process and code conventions,
`docs/ARCHITECTURE.md` owns subsystem behavior, `docs/RULE_AUTHORING.md` owns the CLI and JSON format,
and `docs/RELEASING.md` owns the release flow. Where two documents appear to disagree, the owner wins.

## Commits and pull requests

Use Conventional Commits with a short scope, matching the existing history:

```
feat(ui): add unit-aware duration inputs
fix(engine): preserve counters when only the quota limit changes
docs(readme): document SHA-256 checksums
chore(release): bump version to 2.9.0
```

Keep a pull request focused on one concern. Fill in `.github/PULL_REQUEST_TEMPLATE.md`, list the
commands you ran, and add screenshots for UI changes. Describe user-facing changes in the pull
request: maintainers cut the release and turn that description into the `CHANGELOG.md` entry, as
described in [docs/RELEASING.md](docs/RELEASING.md).

By submitting a contribution you agree that it is licensed under the [MIT License](LICENSE).

## Reporting bugs and security issues

- Reproducible bugs and feature requests: [GitHub Issues](https://github.com/psyattack/Synector/issues/new/choose).
- Questions and troubleshooting: [docs/FAQ.md](docs/FAQ.md).
- Vulnerabilities: follow [SECURITY.md](SECURITY.md) and report privately; do not open a public
  issue.
- Community behavior: see [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
