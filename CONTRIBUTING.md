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
cargo fmt
```

`cargo clippy -- -D warnings` is the CI gate: warnings are errors. Unit tests live next to the code
in `#[cfg(test)] mod tests`; there is no `tests/` directory. Run a single module with
`cargo test <substring>` (for example `cargo test models::tests`).

Run modes:

```powershell
cargo run                    # spawn the daemon if absent, then open the GUI
cargo run -- --gui           # GUI only, connects to an existing daemon
cargo run -- --daemon        # daemon/tray only
cargo run -- --status        # print daemon status over IPC and exit
```

## Where changes belong

| Area | Files |
|---|---|
| Domain types, serde tags, JSON schema | `src/models.rs` |
| Scheduling, quotas, cooldowns, active executions | `src/daemon/engine.rs` |
| Action execution handlers | `src/daemon/actions.rs` |
| Process detection | `src/daemon/watcher.rs` |
| Hotkeys | `src/daemon/hotkeys.rs` |
| System events and state | `src/daemon/system_events.rs` |
| GUI state, drafts, IPC calls | `src/gui/state.rs` |
| Views and inspectors | `src/gui/views/` |
| Localization | `src/i18n.rs`, `src/i18n/en.rs`, `src/i18n/ru.rs` |
| In-app guide | `src/gui/views/guide_modal.rs` |
| CLI rule authoring | `src/cli.rs` |
| IPC protocol | `src/ipc/protocol.rs` |
| Configuration persistence | `src/config.rs` |

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

Documentation has one owner per topic. Update the owning file and link to it instead of copying its
content elsewhere.

| File | Owns |
|---|---|
| `README.md`, `README.ru.md` | User overview, features, installation, quick-start commands. Keep both in sync. |
| `docs/ARCHITECTURE.md` | Subsystem behavior and design rationale. |
| `docs/RULE_AUTHORING.md` | CLI reference, JSON rule format, quotas, share codes. |
| `docs/EXAMPLES.md` | Ready-to-apply rule recipes. |
| `docs/FAQ.md` | User questions and troubleshooting. |
| `docs/README.md` | Documentation map and entry point. |
| `CONTRIBUTING.md` | This file: build, style, PR process. |
| `SECURITY.md` | Vulnerability reporting policy. |

User-visible GUI strings belong to `Strings` in `src/i18n.rs`, with values in `src/i18n/en.rs` and
`src/i18n/ru.rs`; views must not embed literals. The in-app guide is the documented exception: its
long prose uses Russian/English `if lang == Language::Russian { ... } else { ... }` branches in
`src/gui/views/guide_modal.rs`, and both branches must be updated together. Chrome labels in the
guide still come from `Strings`.

When dependencies change, update the direct-dependency table (and attributions) in
`THIRD-PARTY-NOTICES.md`. When CLI commands or quota behavior change, update
`docs/RULE_AUTHORING.md`. When subsystems change, update `docs/ARCHITECTURE.md`. When behavior
users ask about changes, update `docs/FAQ.md`.

## Commits and pull requests

Use Conventional Commits with a short scope, matching the existing history:

```
feat(ui): add unit-aware duration inputs
fix(engine): preserve counters when only the quota limit changes
docs(readme): document SHA-256 checksums
chore(release): bump version to 2.9.0
```

Keep a pull request focused on one concern. Fill in `.github/PULL_REQUEST_TEMPLATE.md`, list the
commands you ran, and add screenshots for UI changes. Maintainers update `CHANGELOG.md` at release
time; describe user-facing changes in the PR so the note can be written accurately.

By submitting a contribution you agree that it is licensed under the [MIT License](LICENSE).

## Reporting bugs and security issues

- Reproducible bugs and feature requests: [GitHub Issues](https://github.com/psyattack/Synector/issues/new/choose).
- Questions and troubleshooting: [docs/FAQ.md](docs/FAQ.md).
- Vulnerabilities: follow [SECURITY.md](SECURITY.md) and report privately; do not open a public
  issue.
- Community behavior: see [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
