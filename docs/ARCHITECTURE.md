# Synector Architecture

Synector is a Windows-only automation suite and process supervisor written in Rust. It separates
background execution from user interaction: a tray daemon owns rules, triggers, runtime state and
system monitoring, while the egui desktop GUI is a controlling client that renders daemon state and
edits drafts.

The repository layout (top level only) is in the Project structure section of
[README.md](../README.md); module-level pointers are in [CONTRIBUTING.md](../CONTRIBUTING.md)
("Where changes belong"). This document explains how the pieces behave and why.

Related documents: [documentation map](README.md) · [rule authoring](RULE_AUTHORING.md) ·
[rule examples](EXAMPLES.md) · [FAQ and troubleshooting](FAQ.md) · [contributing](../CONTRIBUTING.md).

## Two tiers

```
+-------------------------------------------------------------+
|            Interactive Desktop GUI (eframe / egui)           |
|  - Hardware accelerated, high-DPI                            |
|  - Themes: Dark, Light, Chocolate                            |
|  - English and Russian locales (src/i18n)                    |
|  - Rule Builder, Macro Studio, Overlay Studio, Inspector,    |
|    Analytics                                                 |
+--------------------------+----------------------------------+
                           | Windows named pipe
                           | \\.\pipe\Synector_Daemon
+--------------------------v----------------------------------+
|            System tray daemon (Tokio / Win32)                |
|  - Rule engine with quotas, cooldowns and active executions  |
|  - Hybrid ETW / Toolhelp32 process detection                 |
|  - Global hotkeys, file watchers, webhook and system events  |
|  - Audio, display, process, window and memory actions        |
|  - Embedded Lua 5.4.9 runtime                                |
|  - Overlay process supervisor                                |
|  - Tray icon, notifications and background self-update       |
+-------------------------------------------------------------+
```

Both tiers are the same executable, selected by command-line flags. They share the domain types in
`models` and the protocol in `ipc::protocol`, so a rule is identical on both sides. The daemon is the
only owner of runtime state; the GUI keeps a draft copy for editing and reconciliation.

## Runtime modes

| Invocation | Behavior |
|---|---|
| no flags | Start `--daemon` in the background if absent, then open the GUI |
| `--gui` | Open the GUI; the daemon is started first if it is not running |
| `--daemon` | Background service only (tray mode) |
| `--status` | Print daemon status over IPC and exit |
| `--overlay-window <id>` | Standalone borderless overlay window for one overlay (`--calibration`, `--duration`, `--override-text`, `--overlay-data`) |
| `--run-rule <rule id>` | Invoke a rule directly (IPC when the daemon runs, otherwise an autonomous runner fallback) |
| `--rules-*` | Rule authoring CLI: validate, apply, sync, list, export, enable/disable, reset, delete, share, import, schema and templates. See [RULE_AUTHORING.md#commands](RULE_AUTHORING.md#commands). |

Synector runs elevated by default (`--no-elevate` opts out). Elevation enables kernel ETW process
tracing, UIPI-safe input simulation into elevated applications, and silent highest-privilege logon
autostart through Task Scheduler.

`main.rs` dispatches in a fixed order: standalone overlay windows → CLI rule commands → update-file
cleanup → `--status`/`--run-rule` → self-elevation → daemon or GUI. Overlay windows are separate
processes and must stay on the fast path; they never touch update files or single-instance mutexes.
CLI rule commands run before elevation because they are console tools.

## Layers

| Area | Responsibility |
|---|---|
| `src/gui/` | egui views, overlay renderer, macro recorder, GUI state and IPC synchronization |
| `src/daemon/` | Rule engine, action handlers, triggers and watchers, Lua runner, tray, overlay supervisor, webhook server, named-pipe server, Discord RPC |
| `src/ipc/` | Named pipe client and the shared request/response protocol |
| `src/models.rs` | Domain model: `Rule`, `Trigger`, `Action`, conditions, overlays, macro presets, quotas |
| `src/cli.rs` | Command-line rule authoring: parsing, validation, upsert/sync, share codes and IPC dispatch |
| `src/config.rs` | Split configuration persistence: settings, rules, presets and runtime counters |
| `src/i18n.rs`, `src/i18n/` | Locale registry and the English/Russian string catalogs |
| `src/autorun.rs` | Task Scheduler autostart registration |
| `src/updater.rs` | GitHub release checks, asset selection, SHA-256 verification, portable self-replacement and silent installer updates |
| `src/sharing.rs` | `SYN1:` Brotli + Base64 share codes: SHA-256 prefix, library dependency bundling, bounded decode |
| `src/shortcut.rs` | Windows `.lnk` generation for direct rule launch |
| `src/single_instance.rs` | Named mutex guards for daemon and GUI |

## Data flow

1. The GUI reads state over IPC and holds an editable draft with undo/redo history.
2. Mutations are sent as `IpcRequest` values; the daemon validates them, applies them to its
   in-memory `AppConfig` aggregate and persists only the affected config section under
   `%LOCALAPPDATA%\Synector\`.
3. The daemon executes rules and streams status, telemetry and execution snapshots back, so the GUI
   reflects engine state without owning it.
4. Overlay windows are separate GPU-accelerated processes supervised by the daemon; the GUI only
   edits their definitions.

## Configuration storage

The configuration is split into four independent, versioned JSON files in
`%LOCALAPPDATA%\Synector\` (relocatable through `SYNECTOR_CONFIG_DIR` for portable or isolated runs).
The sections and their user-facing contents are listed in
[README.md#configuration](../README.md#configuration); this section covers how they are written.
`AppConfig` exists only in memory: it is assembled from the sections on load and exchanged over IPC,
never written as one document. This keeps write volume proportional to what changed.

Every write goes through a temporary file that is flushed and renamed over the target, so a crash
mid-write cannot truncate user data. Every section carries a schema version that is re-stamped on
each save, and a section that fails to parse is renamed to `<name>.corrupt-<timestamp>` before
defaults replace it.

The daemon is the only writer while it is reachable. The GUI sends section-scoped requests
(`SaveSettings`, `SaveRule`, `ReorderRules`, `ResetRule`, `SaveOverlay`, ...) and only falls back to
a direct file write when IPC is unavailable, so the two processes cannot race on a
read-modify-write of the same file. `--rules-sync` is a single `SyncRules` request that replaces the
rule set, persists once, reloads hotkeys once and prunes counters of removed rules in one step.

Hot sections are read through a process-local cache (`settings.json`, `library.json`) that validates
the cached value against the file's size and modification time: action steps, Lua `ph.overlay_*`
calls and overlay windows avoid re-reading and re-parsing the file, while an externally written file
is still picked up immediately. Runtime counters, overlay drag positions and main-window geometry are
written coalesced (bounded intervals, plus a forced flush on quota pause, reset, window exit and
shutdown), so a fast-firing rule, a dragged HUD or a resized window does not turn every event into a
disk write. Under `cfg(test)` the config directory is an isolated per-process temporary directory, so
tests never touch real user data.

## Event sources and polling

Synector prefers push notifications, but Windows does not expose every event that way. The watcher
subscribes to kernel ETW process lifecycle events when elevated; if tracing cannot start or the
process is not elevated, it degrades to adaptive Toolhelp32 snapshot polling. Both paths feed the
same trigger surface, so a rule never depends on which one is active.

Other sources are mixed by nature:

- **Push/event sources:** kernel ETW process lifecycle records, Win32 named-pipe IPC, the local
  webhook server (`127.0.0.1:48123`), low-level keyboard/mouse hooks during macro recording, Raw
  Input camera deltas, `WM_POWERBROADCAST` for sleep/resume and console display state, and
  `WM_DEVICECHANGE` for plug-and-play devices.
- **Polling sources:** Toolhelp32 process snapshots when ETW is unavailable, foreground-window
  detection, the file watcher, resource thresholds, session lock, AC/battery power source,
  user idle/active, network state and system telemetry (CPU, RAM, network, battery, microphone).
  Polling loops back off when no rule uses the corresponding trigger.
- **Global hotkeys:** `hotkeys.rs` checks `GetAsyncKeyState` for the registered combinations on a
  short interval (8 ms while hotkeys exist) and sleeps on a condition variable when there are none.
  This is a polling design, not a `RegisterHotKey` hook; it is what allows mouse-button combinations
  and hold/repeat modes to share one code path.

### System notification window

`system_events.rs` starts a dedicated hidden top-level window on the
`system-event-notifications` thread. It registers `RegisterPowerSettingNotification` with
`GUID_CONSOLE_DISPLAY_STATE` and `RegisterDeviceNotificationW` for all device interface classes.
`WM_POWERBROADCAST` maps suspend/resume and display on/off/dimmed transitions;
`WM_DEVICECHANGE` maps device-interface arrival/removal and captures the interface path for the
optional `device_id_filter`.

Notifications are pushed to a bounded queue and wake the daemon loop immediately through a Tokio
`Notify`, so a sleep trigger can start its pipeline before Windows suspends the machine. The same
module exposes the current state used by pre-conditions: sleeping/awake, display on/off and whether
a tracked device is connected. Conditions for device state stay false until Windows has delivered at
least one plug-and-play notification, which avoids firing a "device disconnected" rule at startup.

## Rule engine

A rule pairs one optional trigger with preconditions, an ordered action pipeline, `on_exit_actions`,
and cooldown/quota limits. `src/daemon/engine.rs` schedules triggered runs, evaluates conditions,
enforces quotas and cooldowns, tracks active executions and reports analytics.

- **Cooldown:** minimum time between two triggered runs.
- **Lifetime quota:** `max_executions` without `quota_period`. When it is reached the rule is paused
  automatically and can be re-enabled with `--rules-reset` or the GUI reset action.
- **Periodic quota:** `quota_period` keeps the rule enabled and skips executions until the counter
  rolls over. The exact reset rules and custom windows are documented in
  [RULE_AUTHORING.md#quotas](RULE_AUTHORING.md#quotas).
- **Runtime counter ownership:** `execution_count`, `last_executed_at` and `quota_window_start` live
  in `runtime.json`, are zeroed in `rules.json`, and are preserved across ordinary rule updates. A
  changed `quota_period`, or enabling a limit where there was none, starts a fresh counter; changing
  only the numeric limit keeps the counter.

Pipelines support waits, inline checks and branching:

- `Action::Wait` delays by timer or by a system condition (window opened/closed, process
  started/exited, hotkey pressed, file created/deleted) with a timeout and an abort-or-continue
  timeout policy.
- `Action::Check` evaluates environmental criteria mid-pipeline and halts the remaining steps
  cleanly when unsatisfied, without raising an error.
- `Action::IfElse` evaluates conditions at runtime and runs the Then, Else If or Else action list.
- Toggle actions: firewall `Toggle`, window `ToggleMinimizeRestore` and `ToggleShowHide`.

`Trigger::GlobalHotkey` supports plain toggle mode, `hold_mode: true` and `repeat_while_held: true`.
In hold mode the entire pipeline repeats until the hotkey is released; `repeat_while_held` without
hold mode loops until the hotkey is pressed again. Every iteration goes through the normal quota
check, so the quota is decremented per iteration.

Infinite macros (`Action::Macro { infinite: true }`) have no natural end. The CLI validator rejects
them unless the trigger is a global hotkey, because another trigger offers no manual cancellation
path; the Rule Builder shows a warning and dims every step after the infinite one, since those steps
will never run.

## Companion mode (bound satellites)

`Action::StartProcess { close_on_exit: true }` attaches a companion process to a primary
application: the satellite starts with it and terminates when it exits. `Action::Overlay` has the
same `close_on_exit` flag for HUDs. While the primary application runs, the binding is tracked in
the daemon's active execution state so the rule list and analytics show it.

Process-triggered rules can enable `single_instance`. It does not prevent more copies of the
application from starting; it makes the rule fire only for the first matching instance and defers
exit actions and bound-satellite cleanup until every tracked instance has exited.

## Input simulation and Macro Studio

Keyboard and mouse events go through Win32 `SendInput`. Games and DirectInput consumers read
low-level scan codes and ignore virtual keys, so every simulated key carries a hardware scan code
obtained from `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)` together with `KEYEVENTF_SCANCODE`; extended keys
(arrows, Delete, Home, End, ...) also set `KEYEVENTF_EXTENDEDKEY`. Polled game engines only sample
input once per frame, so presses stay down for at least 30–35 ms.

Macro Studio records low-level keyboard and mouse hooks (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`) while a
recording is active, groups events into a timeline, and replays them with humanized jitter. Relative
mouse movement (`MouseCoordMode::Relative`) is captured through Raw Input and replayed as short
delta pulses for in-game camera turns.

## Overlay Studio

Overlays are borderless, GPU-accelerated windows rendered by a dedicated process
(`--overlay-window <id>`), positioned by the daemon's supervisor across monitors with DPI and
orientation awareness. Optional `WDA_EXCLUDEFROMCAPTURE` keeps overlays out of screen capture.
The studio itself is a WYSIWYG editor: multi-monitor artboard, snap-to-grid, pan/zoom, element
catalog, undo/redo, and interactive controls (buttons, toggles, sliders) bound to actions, volume,
opacity and live telemetry. Calibration is started from the studio and makes the overlay draggable;
there is no global calibration hotkey.

## Sharing and backup

`src/sharing.rs` turns rules into `SYN1:` text codes that can be pasted into chats: the JSON payload
is Brotli-compressed (quality 11, with a versioned custom dictionary), prefixed with an 8-byte
SHA-256 checksum and Base64URL-encoded. A code also bundles the macro presets and overlays its rules
reference, so the recipient gets a working automation set. Presets or overlays whose id already
exists locally with different content are re-identified and every rule reference is rewritten; rules
whose content already exists are skipped as duplicates, and colliding rule ids get a fresh id.

Sender-local data never travels: runtime counters are reset, custom icon paths are dropped with a
notice, and truncated or altered codes fail the checksum instead of decoding to garbage. Decoding is
bounded (code length, decompressed size and rule count), so a hostile code cannot exhaust memory.
Exporting is one click: the code is compressed on a worker thread and copied straight to the
clipboard. Importing is one daemon-owned `IpcRequest::ImportShare` step that applies rules plus
library entries, skips duplicates and creates the rules disabled by default. The toast reports how
many rules, library entries and duplicates were handled, while advisory notices (scripts, elevated
processes, webhooks, dropped icons, missing presets/overlays) are written to the log. JSON files
remain the backup format: `--rules-export` and the CLI authoring commands cover file-based migration.

## Installation forms and updates

Synector ships in two forms. The portable zip is `Synector.exe` plus `LICENSE` and `NOTICES.md`; the
installer (`installer/synector.iss`, built only in CI) installs the same executable into `Program
Files`, adds Start Menu and `App Paths` integration and registers an uninstaller. A console bootstrap
(`installer/install.cmd`, or `install.ps1` fetched with `irm <url> | iex`) covers machine-driven
installs: it resolves the latest release, verifies the chosen artifact's SHA-256 and runs Setup,
extracts the portable archive or calls the uninstaller.

The daemon owns update work: it checks GitHub releases, streams and verifies the download against the
SHA-256 digest published by the GitHub API, and applies it according to the install mode. Every
release also publishes `SHA256SUMS.txt` alongside the artifacts for manual verification; the updater
itself uses the API digest. A portable
copy replaces its own executable in place; an installed copy downloads the release's setup and starts
it silently with `/VERYSILENT /UpdateMode=gui|daemon`, letting the installer replace files, refresh
the uninstaller metadata and restart Synector. The restart follows the update origin: a background
update keeps the daemon-only or GUI state that was active before. Progress and status surface through
the tray menu and overlay notifications, and optional auto-install is a setting.

The installer closes running instances and, for updates and interactive installs, removes the stale
masked daemon copy so it is rebuilt from the new binary. On uninstall it removes the autostart task,
the masked copy and the application files while keeping the user configuration directory.

## Rules: organization and editing

Rules carry custom colored tags, tag filters, and multi-mode sorting (manual/default, name ascending
or descending, trigger type, action count, status). In default sort order with no tag filter and no
active search, cards are reordered by drag handle and the new order is committed immediately and
persisted through IPC; filtered, searched or sorted modes disable reordering with an explanatory
tooltip.

The GUI keeps an undo/redo draft per edited rule and per overlay definition (up to 50 snapshots), so
editing, testing and saving a rule never mutates daemon-owned counters before the user commits.

## Logging and localization

Every user-visible string lives in the `Strings` catalog (`src/i18n.rs`, `src/i18n/en.rs`,
`src/i18n/ru.rs`). The in-app guide is the deliberate exception: its long prose is written as
Russian/English `if lang == Language::Russian { ... } else { ... }` branches inside
`src/gui/views/guide_modal.rs`, while its chrome labels come from `Strings`. Both language branches
must be updated together.

Diagnostics go through `crate::logging` into daily files under `%LOCALAPPDATA%\Synector\logs`; the
level comes from `SYNECTOR_LOG` (`off|error|warn|info|debug|trace`, default `info`). Log identifiers
and outcomes only — never script bodies, webhook payloads, rule JSON or other user secrets. Action
errors that may embed script output are routed through the engine's `SensitiveLog` event.

## Design conventions

- Localization is mandatory for regular views; the guide's inline bilingual prose is the documented
  exception.
- Documentation has one owner per topic; the document set and its owners are listed in one place,
  [docs/README.md](README.md). Link instead of copying text across files.
- `README.md` and `README.ru.md` are mirrors and must change together.
