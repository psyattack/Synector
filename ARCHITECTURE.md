# Synector Architecture

Synector is a lightweight Windows automation suite and process supervisor written in pure Rust.
It separates background execution from user interaction: a tray daemon owns rules, triggers and
system monitoring, while the egui desktop GUI is a controlling client that renders state and
edits drafts.

Per-file layout lives in the Structure section of [README.md](README.md); this document describes
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
| `src/config.rs` | `AppConfig` persistence and validation (JSON) |
| `src/i18n.rs`, `src/i18n/` | Locale registry and the English/Russian string catalogs |
| `src/autorun.rs` | Task Scheduler autostart registration |
| `src/updater.rs` | GitHub release check, streaming download, self-replacement |
| `src/sharing.rs` | Base64 + Deflate rule export/import codes |
| `src/shortcut.rs` | Windows `.lnk` generation for direct rule launch |
| `src/single_instance.rs` | Named mutex guards for daemon and GUI |

## Data flow

1. The GUI reads state over IPC and holds an editable draft with undo/redo history.
2. Mutations are sent as `IpcRequest` values; the daemon validates them, applies them to
   `AppConfig` and persists to `%LOCALAPPDATA%\Synector\`.
3. The daemon executes rules and streams status, telemetry and execution snapshots back, so
   the GUI reflects engine state without owning it.
4. Overlay windows are separate GPU-accelerated processes supervised by the daemon; the GUI
   only edits their definitions.

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

### Presets and dynamic operations

Overlay and macro templates are modular presets stored in `AppConfig` and synchronized between
daemon and GUI over IPC. Rules either reference a preset or detach it into an inline copy.
Macros can be driven by `Run`, `Stop` and `Toggle` operations with cancellation tracking, and
`Action::Overlay` supports `SetText` to update HUD strings at runtime.

### Updates, tray and notifications

The daemon owns update work: it checks GitHub releases, streams and verifies the download, and
can install silently in place with automatic restarts. Progress and status surface through the
tray menu and overlay notifications. Optional auto-install is a setting, not a default.

## Design conventions

- Localization: every user-visible string lives in the three catalog files, never inline in a
  view.
- No legacy shims: obsolete variants, fields and paths are removed instead of kept behind
  compatibility wrappers.
- Rust crate versions and commands come from `Cargo.toml` and [README.md](README.md); do not
  restate them here, they drift.
