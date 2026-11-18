# Synector Architecture

Synector is a lightweight Windows automation suite and process supervisor written in pure Rust.
It separates background execution from user interaction: a tray daemon owns rules, triggers and
system monitoring, while the egui desktop GUI is a controlling client that renders state and
edits drafts.

Per-file layout lives in the Structure section of [README.md](../README.md); this document describes
how the pieces behave and why.

## Two tiers

```
+-------------------------------------------------------------+
|            Interactive Desktop GUI (eframe / egui)          |
|  - Hardware accelerated, high-DPI                          |
|  - Themes: Dark, Light, Chocolate                          |
|  - English and Russian locales (src/i18n)                   |
|  - Macro Studio, Rule Builder, Overlay Studio, Inspector,   |
|    Analytics                                               |
+--------------------------+----------------------------------+
                           | Windows named pipe
                           | \\.\pipe\Synector_Daemon
+--------------------------v----------------------------------+
|            System tray daemon (tokio / win32)                |
|  - Background rule engine with quotas and cooldowns          |
|  - Hybrid ETW (elevated) and snapshot process watcher        |
|  - Global hotkeys, file watchers, webhook and system events  |
|  - Audio, display, process and memory actions                |
|  - Embedded Lua 5.4 runtime                                  |
|  - Overlay process supervisor                                |
|  - Tray icon, notifications, background self-update          |
+-------------------------------------------------------------+
```

Both tiers are the same executable, selected by CLI flags. They share domain types through
`models` and `ipc::protocol`, so a rule is identical on both sides; the daemon is the only
owner of runtime state, the GUI keeps a draft copy for editing and reconciliation.

## Runtime modes

| Invocation | Behavior |
|---|---|
| no flags | Start `--daemon` in the background if absent, then open the GUI |
| `--gui` | Open the GUI and connect to an already running daemon |
| `--daemon` | Background service only (tray mode) |
| `--status` | Print daemon status over IPC and exit |
| `--overlay-window <id>` | Standalone borderless overlay window for one overlay |
| `--run-rule <id_or_title>` | Invoke a rule directly (IPC when the daemon runs, otherwise an autonomous runner fallback) |
| `--rules-validate <file>` | Parse and validate a JSON rule file without changing anything |
| `--rules-apply <file>` | Upsert rules from JSON (IPC when the daemon runs, direct `rules.json` edit otherwise) |
| `--rules-list`, `--rules-export [<file>]`, `--rules-delete <id>` | Inspect, export or delete rules from the command line |

Synector runs elevated by default (`--no-elevate` opts out). Elevation is what enables kernel
ETW process tracing, UIPI-safe input simulation into elevated applications, and silent
highest-privilege logon autostart through Task Scheduler.

## Layers

| Area | Responsibility |
|---|---|
| `src/gui/` | egui views, overlay renderer, macro recorder, high-level state and IPC sync |
| `src/daemon/` | Rule engine, actions, triggers and watchers, Lua runner, tray, overlay supervisor, webhook server, updater |
| `src/ipc/` | Named pipe client/server and the shared request/response protocol |
| `src/models.rs` | Domain model: `Rule`, `Trigger`, `Action`, tags, overlays |
| `src/cli.rs` | Command-line rule authoring: JSON parsing, validation, upsert and IPC dispatch |
| `src/config.rs` | Split configuration persistence: settings, rules, presets and runtime counters (JSON) |
| `src/i18n.rs`, `src/i18n/` | Locale registry and the English/Russian string catalogs |
| `src/autorun.rs` | Task Scheduler autostart registration |
| `src/updater.rs` | GitHub release check, asset selection, SHA-256 verification, portable self-replacement and silent installer updates |
| `src/sharing.rs` | Base64 + Deflate rule export/import codes |
| `src/shortcut.rs` | Windows `.lnk` generation for direct rule launch |
| `src/single_instance.rs` | Named mutex guards for daemon and GUI |

## Data flow

1. The GUI reads state over IPC and holds an editable draft with undo/redo history.
2. Mutations are sent as `IpcRequest` values; the daemon validates them, applies them to its
   in-memory `AppConfig` aggregate and persists only the affected config section under
   `%LOCALAPPDATA%\Synector\`.
3. The daemon executes rules and streams status, telemetry and execution snapshots back, so
   the GUI reflects engine state without owning it.
4. Overlay windows are separate GPU-accelerated processes supervised by the daemon; the GUI
   only edits their definitions.

## Configuration storage

The configuration is split into four independent, versioned JSON files in
`%LOCALAPPDATA%\Synector\` (relocatable through `SYNECTOR_CONFIG_DIR` for portable or isolated
runs). `AppConfig` exists only in memory: it is assembled from the sections on load and exchanged
over IPC, never written as one document. This keeps write volume proportional to what changed.

| File | Contents | Written by |
|---|---|---|
| `settings.json` | Window geometry, tray icon, language, theme, update preferences | Settings changes |
| `rules.json` | Rule definitions; counter fields are written zeroed | Rule create/update/delete/reorder |
| `library.json` | Overlay and macro presets | Preset save/delete, overlay calibration |
| `runtime.json` | Per-rule `execution_count` and `last_executed_at` | Coalesced daemon flush |

Every write goes through a temporary file that is flushed and renamed over the target, so a crash
mid-write cannot truncate user data, and every file carries a schema version that is re-stamped on
each save. A section that fails to parse is renamed to `<name>.corrupt-<timestamp>` before defaults
replace it, so a hand-edited file is never silently destroyed by the next save.

The daemon is the only writer while it is reachable; the GUI sends section-scoped requests
(`SaveSettings`, `SaveRule`, `ReorderRules`, `ResetRule`, `SaveOverlay`, ...) and only falls back to
a direct file write when IPC is unavailable, so the two processes cannot race on a read-modify-write
of the same file. `--rules-sync` is a single `SyncRules` request that replaces the rule set, persists
once, reloads hotkeys once and prunes counters of removed rules in one step.

Hot sections are read through a process-local cache (`settings.json`, `library.json`) that validates
the cached value against the file's size and modification time: action steps, Lua `ph.overlay_*`
calls and overlay windows avoid re-reading and re-parsing the file, while an externally written file
is still picked up immediately. Runtime counters, overlay drag positions and main-window geometry
are written coalesced (bounded intervals, plus a forced flush on quota pause, reset, window exit and
shutdown), so a fast-firing rule, a dragged HUD or a resized window does not turn every event into
a disk write.

## Subsystems

### Input simulation and Macro Studio

Keyboard and mouse events go through Win32 `SendInput`. Games and DirectInput consumers read
low-level scan codes and ignore virtual keys, so every simulated key carries a hardware scan
code obtained from `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)` together with `KEYEVENTF_SCANCODE`;
extended keys (arrows, Delete, Home, End, ...) also set `KEYEVENTF_EXTENDEDKEY`. Polled game
engines only sample input once per frame, so presses stay down for at least 30-35 ms.

Macro Studio records low-level hooks, groups events into a timeline, and replays them with
humanized jitter and optional camera-rotation support.

### Overlay Studio

Overlays are borderless, GPU-accelerated windows rendered by a dedicated process
(`--overlay-window <id>`), positioned by the daemon's supervisor across monitors with DPI and
orientation awareness. Optional `WDA_EXCLUDEFROMCAPTURE` keeps overlays out of screen capture.
The studio itself is a WYSIWYG editor: multi-monitor artboard, snap-to-grid, pan/zoom,
element catalog, undo/redo, and interactive controls (buttons, toggles, sliders) bound to
actions, volume, opacity and live telemetry.

### Rule engine

A rule pairs one trigger with an ordered action pipeline. The engine schedules triggered runs,
enforces per-rule quotas (`max_executions`) and cooldowns, tracks active executions for the UI,
and reports analytics telemetry.

Pipelines support waits, inline checks and branching:

- `Action::Wait` delays by timer or by a system condition (window opened/closed, process
  started/exited, hotkey pressed, file created/deleted) with a timeout and an
  abort-or-continue timeout policy.
- `Action::Check` evaluates environmental criteria mid-pipeline and halts the remaining steps
  cleanly when unsatisfied, without raising an error.
- Toggle actions: firewall `Toggle`, window `ToggleMinimizeRestore` and `ToggleShowHide`.

### Triggers, hold mode and loops

`Trigger::GlobalHotkey` supports plain toggle mode, `hold_mode: true`, and
`repeat_while_held: true`. In hold mode the entire pipeline repeats until the hotkey is
released, and every iteration must stay synchronized with the rule's execution quota
(`max_executions`): the quota is checked and decremented per iteration, and the rule pauses
automatically when it is exhausted.

Infinite macros (`Action::Macro { infinite: true }`) are only allowed on triggers that offer a
manual cancellation path, meaning global hotkeys. With an infinite step, later pipeline steps
never run, so the editor has to warn about it and dim the following steps.

### Bound satellites (companion mode)

`Action::StartProcess { close_on_exit: true }` attaches a companion process to a primary
application: the satellite starts with it and terminates when it exits. For as long as the
primary application runs, the step is tracked as supervising in `active_rule_executions` so
the rule list and analytics show it.

### Process monitoring and elevation

When running elevated, the watcher subscribes to kernel process lifecycle events
(`EVENT_TRACE_FLAG_PROCESS`) for instant, zero-polling detection. If elevation or tracing is
unavailable it degrades to adaptive Toolhelp32 snapshot polling. Both paths feed the same
trigger surface, so rules never depend on which one is active.

### Rules: organization and editing

Rules carry custom colored tags, tag filters, and multi-mode sorting (manual/default,
alphabetical, trigger type, action count, status). In default sort order with no tag filter,
cards are reordered by drag handle and the new order is committed immediately and persisted
through IPC; filtered or sorted modes disable reordering with an explanatory tooltip.

### Rule authoring without the GUI

`src/cli.rs` validates and applies JSON rule files: upserts go through `IpcRequest::ImportRules`
while the daemon runs, or directly into `rules.json` when it does not, with daemon-owned runtime
counters kept in `runtime.json` and preserved across updates. Declarative `--rules-sync` is one
`SyncRules` request that replaces the whole set atomically instead of an upsert followed by a delete
per rule. The format is self-describing (`--rules-schema` is a `schemars`-derived draft-07 schema,
plus `--rules-reference` and `--rules-template`), and the CLI supports single-rule
enable/disable/reset, `--json` reports and `--dry-run`. The full command reference lives in
[RULE_AUTHORING.md](RULE_AUTHORING.md).

### Presets and dynamic operations

Overlay and macro templates are modular presets stored in `library.json` and synchronized between
daemon and GUI over IPC. Rules either reference a preset or detach it into an inline copy.
Macros can be driven by `Run`, `Stop` and `Toggle` operations with cancellation tracking, and
`Action::Overlay` supports `SetText` to update HUD strings at runtime.

### Installation forms and updates

Synector ships in two forms. The portable zip is a single `Synector.exe`; the installer
(`installer/synector.iss`, built only in CI) installs the same executable into `Program Files`,
adds Start Menu/`App Paths` integration and registers an uninstaller. A console bootstrap
(`installer/install.cmd`, or the same script fetched with `irm <url> | iex`) covers machine-driven
installs: it resolves the latest release, verifies the chosen artifact's SHA-256 and runs Setup,
extracts the portable archive or calls the uninstaller. Both the installer and the bootstrap are
build-time/distribution tooling, not runtime dependencies; user data always stays in
`%LOCALAPPDATA%\Synector\` and survives updates and uninstalls.

The daemon owns update work: it checks GitHub releases, streams and verifies the download against
the SHA-256 digest published by the GitHub API, and applies it according to the install mode. A
portable copy replaces its own executable in place; an installed copy downloads the release's
setup and starts it silently with `/VERYSILENT`, letting the installer replace files, refresh the
uninstaller metadata and restart Synector. The restart follows the update origin: a background
update keeps the daemon-only or GUI state that was active before. Progress and status surface
through the tray menu and overlay notifications, and optional auto-install is a setting.

The installer closes running instances and, when it runs as an update, removes a stale masked
daemon copy so it is rebuilt from the new binary. On uninstall it removes the autostart task, the
masked copy and the application files while keeping the user configuration directory.

## Design conventions

- Localization: every user-visible string lives in the three catalog files, never inline in a
  view.
- No legacy shims: obsolete variants, fields and paths are removed instead of kept behind
  compatibility wrappers.
- Rust crate versions and commands come from `Cargo.toml` and [README.md](../README.md); do not
  restate them here, they drift.
