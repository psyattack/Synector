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
| Domain types, serde tags, JSON schema | `src/models/` (facade `src/models.rs`) |
| Structural rule validation and warnings shared by CLI/config/engine/GUI | `src/validation.rs` |
| Scheduling, quotas, cooldowns, active executions | `src/daemon/engine/` (facade `src/daemon/engine.rs`; modules `scheduler`, `quota`, `executions`, `pipeline`, `projector`, `triggers`, `state`, `ipc`, `offline`, `update`, `rule_call`, `activity`) |
| Daemon start-up, background loops and shutdown order | `src/daemon/runtime/` (facade `src/daemon/runtime.rs`; `src/daemon.rs` keeps the module list and `run_daemon`) |
| Rule-to-rule calls and stops | `src/daemon/engine/rule_call.rs` (`crate::daemon::rule_call` is a re-export) |
| Rule-activity snapshot behind wait/check conditions | `src/daemon/engine/activity.rs` (`crate::daemon::rule_activity` is a re-export) |
| Condition evaluation and process/window/device matching | `src/daemon/conditions.rs` |
| Action execution handlers | `src/daemon/actions/` (facade `src/daemon/actions.rs`; one module per family) |
| Bounded subprocess runner and `CommandOutcome` (action handlers, Lua bridge, updater tools) | `src/command.rs` |
| Windows command-line quoting, PowerShell literals and the no-window spawn flag | `src/win_args.rs` |
| COM apartment guard (MTA/STA, fallible/infallible) | `src/win_com.rs` |
| Action timeout budgets and module-side re-exports | `src/daemon/actions/support/` |
| Read-only action-tree traversal | `src/models/rule.rs` (`Rule::visit_actions`) |
| Process detection | `src/daemon/watcher.rs` |
| File and directory watching | `src/daemon/file_watcher.rs` |
| Hotkeys | `src/daemon/hotkeys.rs` |
| System events and state | `src/daemon/system_events.rs` |
| System telemetry, media and key visualizer | `src/daemon/telemetry/` (facade `src/daemon/telemetry.rs`) |
| Tray icon, menu and notifications | `src/daemon/tray/` (facade `src/daemon/tray.rs`) |
| Overlay process lifecycle and placement | `src/daemon/overlay_manager/` (facade `src/daemon/overlay_manager.rs`), `src/gui/overlay_window.rs`; placement math in `src/models/overlay/geometry.rs` |
| Webhook trigger server | `src/daemon/webhook_server.rs` |
| Lua runtime and the `ph` API | `src/daemon/lua_runner.rs` |
| Discord Rich Presence | `src/daemon/discord_rpc.rs` |
| Named-pipe server | `src/daemon/ipc_server.rs` |
| GUI state, drafts, undo, IPC calls | `src/gui/state/` (facade `src/gui/state.rs`) |
| Preset library writes shared by the overlay and macro studios | `src/gui/state/library_write.rs` |
| Views and inspectors | `src/gui/views/`; the inspector, rules view, overlay studio and macro studio are module trees: `src/gui/views/inspector/` (facade `src/gui/views/inspector.rs`), `src/gui/views/rules_view/` (facade `src/gui/views/rules_view.rs`), `src/gui/views/overlay_modal/` (facade `src/gui/views/overlay_modal.rs`), `src/gui/views/macro_modal/` (facade `src/gui/views/macro_modal.rs`) |
| Shared widgets and renderers | `src/gui/components/`; the rule-card and condition glyph tables shared by the rules list and the inspector live in `src/gui/views/rule_summary.rs` |
| Catalog modal shell (entry definition, chrome, filtering, lifecycle) | `src/gui/components/catalog_modal.rs`; per-kind tables and commit adapters stay in `src/gui/views/catalogs/` |
| Shared GUI primitives (positional `{}` fill, drag-and-drop math/painters, modal header, HSVA colour editor, tag pills) | `src/gui/components/text.rs`, `src/gui/components/dnd.rs`, `src/gui/components/modal.rs`, `src/gui/components/color_picker.rs`, `src/gui/components/chips.rs` |
| Studio preset picker shared by the overlay and macro studios | `src/gui/views/presets.rs` (each studio keeps its own modal shell, data source and load/delete/save effects) |
| Localization | `src/i18n/strings.rs`, `src/i18n/en.rs`, `src/i18n/ru.rs` |
| In-app guide | `src/gui/views/guide_modal.rs` |
| CLI rule authoring | `src/cli/` (facade `src/cli.rs`); one daemon IPC round-trip adapter and the post-snapshot offline→daemon transition helper in `src/cli/target.rs` |
| Process entry, runtime dispatch and argv vocabulary | `src/app/` (facade `src/app.rs`); cross-process flag spellings in `src/arg_spellings.rs` |
| IPC protocol | `src/ipc/protocol.rs` |
| IPC framing shared by client and server | `src/ipc/framing.rs` |
| Overlay wire encoding | `src/ipc/overlay_wire.rs` |
| Configuration persistence | `src/config/` (facade `src/config.rs`) |
| Share codes | `src/sharing/` (facade `src/sharing.rs`; stages `format`, `payload`, `encode`, `decode`, `import`, `assets`) |
| Clipboard access | `src/clipboard.rs` (`copy_text_to_clipboard`, `read_text`) |
| Autostart registration | `src/autorun.rs` |
| Updates and self-replacement | `src/updater/` (facade `src/updater.rs`) |
| Logging | `src/logging.rs` |
| Single-instance guards | `src/single_instance.rs` |

Adding or changing a trigger, action or condition touches `src/models/`, the engine
(`src/daemon/engine/`) and the action handlers (`src/daemon/actions/`), the catalog/inspector GUI
views, and both i18n catalogs. Treat `src/models/` (facade `src/models.rs`) as the single source of
truth: the JSON schema printed by `--rules-schema` is generated from it with `schemars`, so the CLI
format cannot drift from the real types.

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

User-visible GUI strings belong to `Strings` in `src/i18n/strings.rs`, with values in `src/i18n/en.rs` and
`src/i18n/ru.rs`; views must not embed literals. The same `{...}` placeholders must appear in the
English and Russian value of a field — `test_catalog_placeholder_parity` fails the test suite when
they differ. The in-app guide is the documented exception: its long prose uses Russian/English
`if lang == Language::Russian { ... } else { ... }` branches in `src/gui/views/guide_modal.rs`, and
both branches must be updated together. Chrome labels in the guide still come from `Strings`.

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
