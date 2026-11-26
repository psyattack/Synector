# Synector Architecture

Synector is a Windows-only automation suite and process supervisor written in Rust. A tray daemon
owns rules, triggers, runtime state and system monitoring; the egui desktop GUI is a controlling
client that renders daemon state and edits drafts.

The repository layout is in the Project structure section of [README.md](../README.md), and the
module map is in [CONTRIBUTING.md](../CONTRIBUTING.md) ("Where changes belong"). This document
explains how the pieces behave and why.

Related documents: [documentation map](README.md) · [rule authoring](RULE_AUTHORING.md) ·
[rule examples](EXAMPLES.md) · [FAQ and troubleshooting](FAQ.md).

## Two tiers

```
+-------------------------------------------------------------+
|            Interactive desktop GUI (eframe / egui)          |
|  - Hardware accelerated, high-DPI, dark and light palettes   |
|  - English and Russian locales (src/i18n)                   |
|  - Rule Builder, Macro Studio, Overlay Studio, Inspector    |
|  - Global search, analytics and settings dialogs            |
+--------------------------+----------------------------------+
                           | Windows named pipe
                           | \\.\pipe\Synector_Daemon
+--------------------------v----------------------------------+
|            System tray daemon (Tokio / Win32)               |
|  - Rule engine with quotas, cooldowns and active executions |
|  - Process detection, hotkeys, file watching, webhooks      |
|  - System events, audio/display/process/window actions      |
|  - Embedded Lua runtime and overlay supervision             |
|  - Tray icon, notifications and background self-update      |
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
| `--daemon` | Background service only (tray mode; headless if the tray cannot be created) |
| `--status` | Print daemon status over IPC and exit |
| `--overlay-window <id>` | Standalone borderless overlay window for one overlay (`--calibration`, `--duration`, `--override-text`, `--overlay-data`) |
| `--run-rule <rule id>` | Invoke a rule directly: `TestRunRule` over IPC when the daemon runs, otherwise an in-process run through the engine's shared step loop; a missing or empty id is a usage error that exits with code 2 |
| `--rules-*` | Rule authoring CLI: validate, apply, sync, list, export, enable/disable, reset, delete, share, import, schema and templates. See [RULE_AUTHORING.md#commands](RULE_AUTHORING.md#commands). |

After the overlay fast path and the CLI rule commands, the remaining arguments are validated as
runtime flags. An unknown flag, or a value flag with a missing or empty value, prints `error:` plus
the usage block and exits with code 2 instead of starting the GUI behind an elevation prompt. The
rule-authoring CLI keeps its own strict parser.

When no daemon answers, `--run-rule` runs the rule through the engine's shared step loop, so
`PrevOutput`, `StepN.Output`, `AbortPipeline` and `stop_on_error` behave exactly as in a daemon run.
It applies the same gates as the GUI Run button: quota and preconditions are enforced, cooldown and
`is_enabled` are not, so a disabled rule can be tested. Nothing is persisted, rule-to-rule control
has no dispatcher, and the context carries `ProcessId` 0 with `ProcessName` set to the rule's target
or `Manual Run`.

Synector runs elevated by default (`--no-elevate` opts out). Elevation enables kernel ETW process
tracing, UIPI-safe input simulation into elevated applications, and silent highest-privilege logon
autostart through Task Scheduler. The GUI forwards `--no-elevate` to the daemon it spawns and reports
its own executable path as `--gui-exe <path>`, so a portable update can target the real executable
after the window is closed (see [Installation forms and updates](#installation-forms-and-updates)).

`src/app/` (entered from `src/main.rs`) dispatches in a fixed order: standalone overlay windows → CLI
rule commands → update-file cleanup → `--status`/`--run-rule` → self-elevation → daemon or GUI.
Overlay windows are separate processes and stay on the fast path: they never touch update files or
single-instance mutexes. CLI rule commands run before elevation because they are console tools.

Daemon liveness is the named-pipe ping (`IpcClient::is_daemon_running()`), not a single-instance
mutex probe: a mutex only shows that some process holds the name, while the pipe shows that the
daemon answers. A daemon whose tray cannot be created runs headless but still shuts down in order:
On App Close rules stop the engine, running overlays are closed so their fade-out plays, and only
then are runtime counters and overlay positions flushed.

## GUI design system

Every colour and geometry value comes from one token layer, `src/gui/theme.rs` (facade over
`src/gui/theme/`): `tokens` owns the dark and light palettes and the resolved value a view reads,
`skin` the geometry, density and stroke metrics, `style` the egui `Visuals`, `motion` the platform
motion permission, `paint` the shared painting helpers and `fonts` the bundled type stacks.

Palette and skin are independent axes: a view asks for a role (`t.sunken`), a helper
(`t.hairline()`) or a metric (`t.metrics().card_radius`) instead of branching on either. The two
skins differ in how they compose chrome — the classic one is flat and opaque, while the modern
default lifts tinted translucent surfaces — and both must stay readable in both palettes. A user
accent re-tints `Palette::accent` and derives a readable `accent_ink`; it applies at once instead of
fading, because a fade would chase the colour picker's drag.

Windows decides the interface language once, and the palette continuously if asked to: the first
launch adopts the Windows UI language and persists it, and *Follow Windows Theme* re-reads the
Windows light/dark setting at most once a second without a restart. A missing or unreadable value
keeps the configured theme, and automatic switching raises no toast and writes nothing to disk.

Motion has two owners: the skin sets the durations and `theme::motion` asks Windows whether
client-area animations are enabled (`SPI_GETCLIENTAREAANIMATION`). When the system says no, every
duration collapses to zero and no transition schedules a repaint. Components animate user-triggered
state only, never decoration. A palette change cross-fades the colour roles, and the window backdrop
is painted once behind every panel.

A convention test, `views_never_pick_a_literal_frame_radius`, walks `src/gui/views/**` and fails if
a `Frame` carries a literal corner radius instead of a skin helper.

## Window shell, dialogs and global search

The main window has no navigation tabs: the rules list is the only page, and the header owns the one
rules search field, centered between the wordmark and the window caption buttons. Every other search
field reuses the same outlined chrome, with widths and heights per call site. The analytics and
settings surfaces are dialogs opened from two glyph buttons right of the field; opening one joins
`GuiState::is_modal_open()` and surrenders the search focus.

Typing a query opens the global search overlay instead of a plain dropdown: a scrim `Area` covers
everything below the header while a results panel groups matches by the surface that can consume
them — rules, settings, the activity log, actions, triggers, conditions, overlay presets and macro
presets. Applying a group keeps the query in the header for rules and transfers it into the target's
own search field for every other surface. The overlay is not a `Modal`, so the header stays visible
and editable; Escape closes the results first, then clears the query.

The analytics dashboard reads daemon-owned data. `src/daemon/analytics.rs` keeps a rolling 24-hour
window bucketed by local wall-clock hour, per-source and per-rule counts, and the last two minutes of
CPU/RAM samples; the one-second flush loop also samples the daemon's own CPU share, working set and
disk throughput (`DaemonUsage`) outside the engine lock. Network and GPU are deliberately absent
because Windows exposes no cheap per-process counter for either. The GUI polls one narrow
`GetAnalytics` request only while its dialog is open, and the charts in
`src/gui/components/charts.rs` paint from token roles, so there is no plotting dependency.

## Data flow

1. The GUI reads state over IPC and holds an editable draft with undo/redo history.
2. Mutations are queued to one IPC worker thread and sent as `IpcRequest` values; the daemon
   validates them, applies them to its in-memory `AppConfig` aggregate and persists only the
   affected config section. The draft is discarded only after the daemon confirms the mutation; a
   rejected or failed mutation keeps it open and reports the error. Every other GUI round-trip goes
   through the same worker, so the render loop never blocks on the named pipe. A step test is the
   one exception: it executes an action inline on the daemon and may run for minutes, so it gets its
   own thread instead of serializing the state poll behind it. While it runs its Play button becomes
   Stop, and `StopTestStep` aborts it exactly like `StopRuleExecution` aborts a Test Run.
3. The daemon executes rules and streams status, telemetry and execution snapshots back, so the GUI
   reflects engine state without owning it. A sustained daemon outage is logged and shown as an
   offline toast, and the window stays open with unsaved drafts intact.
4. Overlay windows are separate GPU-accelerated processes supervised by the daemon; the GUI only
   edits their definitions.

IPC round-trips are bounded: a client call gets 5 s (750 ms for the liveness ping), a busy pipe is
retried, and a timed-out call cancels and joins its blocked worker so handles cannot accumulate.
During GUI exit the round-trips share a 1.5 s budget with window geometry persisted first. Only
`Unavailable` (the pipe does not exist) proves the daemon offline; `Busy` and `Timeout` do not, so a
slow daemon never triggers an offline fallback. Frames are capped at 16 MiB in both directions, and
the daemon replaces an oversized response with a small `Error` response.

## Configuration storage

The configuration is split into four independent, versioned JSON files in
`%LOCALAPPDATA%\Synector\` (relocatable through `SYNECTOR_CONFIG_DIR`). The sections and their
contents are listed in [README.md#configuration](../README.md#configuration); this section covers
how they are written. `AppConfig` exists only in memory: it is assembled from the sections on load
and exchanged over IPC, never written as one document.

Every write goes through a temporary file that is flushed and renamed over the target, so a crash
mid-write cannot truncate user data. Every section carries a schema version: an older file is
upgraded in memory and stamped with `CONFIG_VERSION` on its next save, while a file written by a
newer build is read as-is and never overwritten — the refusal is logged and returned as a save
error. A section that fails to parse is moved aside as `<name>.corrupt-<timestamp>` before defaults
take over; if it cannot be moved aside, writes to that section are refused until it is repaired by
hand. First-run materialization creates only the missing section files, so an existing
`runtime.json` keeps its counters, and the first run also adopts the Windows UI language before
`settings.json` is written.

Rule-level strictness is separate from section parsing. `rules.json` entries are checked with
`validate_rule_keys` before deserialization, so a rule carrying an unknown key is logged with its id
and keys and skipped while the rest of the file loads; each rule then runs through the same
per-rule validator as the GUI Save and CLI apply paths, so a rule with invalid values is skipped on
its own. `--rules-apply` and share import reject such input instead of skipping it. There is no
migration shim: a rule that still carries a removed field must be updated.

The daemon is the only writer while it is reachable. The GUI sends section-scoped requests
(`SaveSettings`, `SaveRule`, `ReorderRules`, `ResetRule`, `SaveOverlay`, ...), so the two processes
cannot race on a read-modify-write of the same file. The rule, overlay-library and runtime-counter
fallbacks run only after a failed liveness probe, and a reachable daemon that rejects a request is
reported instead of being bypassed. `--rules-sync` is a single `SyncRules` request that replaces the
rule set, persists once and reloads hotkeys once. An offline share import commits `library.json`
before `rules.json`, so imported rules never reference a missing preset or overlay.

Hot sections are read through a process-local cache (`settings.json`, `library.json`) validated
against the file's size and modification time, so an externally written file is picked up
immediately. Runtime counters, overlay drag positions and main-window geometry are written
coalesced, so a fast-firing rule, a dragged HUD or a resized window does not turn every event into a
disk write. Under `cfg(test)` the config directory is an isolated per-process temporary directory.

## Event sources and polling

Windows does not expose every event as a push notification. The watcher subscribes to kernel ETW
process lifecycle events when elevated; if tracing cannot start or the realtime trace thread exits,
it degrades to adaptive Toolhelp32 snapshot polling. The ETW callback only queues `(pid, event)`;
names are resolved on the daemon thread, so protected/PPL processes produce the same events as
polling. Both paths feed the same trigger surface, so a rule never depends on which one is active.

- **Push/event sources:** kernel ETW process records, Win32 named-pipe IPC, the local webhook server
  (`127.0.0.1:48123`), low-level keyboard/mouse hooks during macro recording, Raw Input camera
  deltas, `WM_POWERBROADCAST` for sleep/resume and display state, and `WM_DEVICECHANGE` for
  plug-and-play devices. The webhook server only listens while at least one enabled rule uses the
  `webhook` trigger.
- **Polling sources:** Toolhelp32 snapshots when ETW is unavailable, foreground-window detection,
  the file watcher, resource thresholds, session lock, AC/battery power, user idle/active, network
  state and system telemetry. Polling loops back off when no rule uses the corresponding trigger;
  the system trigger's idle event uses a fixed five-minute threshold, while the `event` condition
  keeps a rule-authored `idle_duration_sec`.
- **Global hotkeys:** `hotkeys.rs` polls `GetAsyncKeyState` for the registered combinations and
  blocks with no periodic wake-up when there are none. That is what allows mouse-button
  combinations and hold/repeat modes to share one code path.

### System notification window

`system_events.rs` starts a dedicated hidden top-level window that registers
`RegisterPowerSettingNotification` for console display state and `RegisterDeviceNotificationW` for
all device interface classes. Notifications are pushed to a bounded queue and wake the daemon loop
through a Tokio `Notify`, so a sleep trigger can start its pipeline before Windows suspends the
machine. The module also exposes the state used by preconditions: sleeping/awake, display on/off and
whether a tracked device is connected. Device conditions stay false until Windows has delivered at
least one plug-and-play notification, which avoids firing a "device disconnected" rule at startup.

## Rule engine

A rule pairs one optional trigger with preconditions, one ordered action pipeline, and cooldown and
quota limits. `src/daemon/engine.rs` is the `DaemonEngine` facade: it schedules triggered runs,
evaluates conditions, enforces quotas and cooldowns, tracks active executions and reports analytics.
The implementation is split by owner under `src/daemon/engine/`: `scheduler` owns admission and tick
throttling, `quota` the counter semantics, `executions` active pipelines and owned
processes/overlays, `pipeline` the step loop and the single launch machinery, `triggers` the
process/window/schedule/resource/system intake, `projector` the read-only `DaemonState` snapshot
the IPC layer serializes, `state` the log buffer and section persistence, `ipc` the request
dispatcher, `offline` the `--run-rule` fallback, `update` the updater display state, `rule_call` the
rule-control dispatcher and `activity` the rule-activity registry.

`DaemonEngine::trigger(rule, origin, context_name, pid, extra_ctx)` is the only pipeline entry point,
and every caller states the `ExecutionOrigin` explicitly. Condition evaluation and
process/window/device matching live in `src/daemon/conditions.rs`, which the action handlers call
directly. `Rule::visit_actions` is the one read-only action-tree traversal used by the engine,
sharing, validation and the GUI.

- **Cooldown:** minimum time between two triggered runs, seeded from `last_executed_at` so a restart
  does not clear a running cooldown. A run the user starts directly bypasses it.
- **Lifetime quota:** `max_executions` without `quota_period`. When it is reached the rule is paused
  and can be re-enabled with `--rules-reset` or the GUI reset action.
- **Periodic quota:** `quota_period` keeps the rule enabled and skips executions until the counter
  rolls over; the reset points are documented in
  [RULE_AUTHORING.md#quotas](RULE_AUTHORING.md#quotas).
- **Runtime counters:** `execution_count`, `last_executed_at` and the quota window anchor live in
  `runtime.json`, are zeroed in `rules.json`, and are preserved across ordinary rule updates; a
  changed `quota_period` or a newly enabled limit starts a fresh counter.

Pipelines support waits, inline checks and branching: `Action::Wait` delays by timer or by a system
condition with a timeout and an abort-or-continue policy; `Action::Check` halts the remaining steps
cleanly when its criteria are unmet; `Action::IfElse` runs the Then, Else If or Else list. Toggle
actions include the firewall toggle, window `ToggleMinimizeRestore` and `ToggleShowHide`.

`Trigger::GlobalHotkey` supports plain toggle mode, `hold_mode: true` and `repeat_while_held: true`,
and every iteration goes through the normal quota check. Infinite macros
(`Action::Macro { infinite: true }`) are rejected by the CLI validator unless the trigger is a
global hotkey, because another trigger offers no manual cancellation path. Schedule triggers are
admitted by the scheduler rather than by scanning every rule on each tick, and a one-shot schedule is
consumed only when its run actually starts.

Action handlers are asynchronous, one module per family under `src/daemon/actions/`, dispatched by an
exhaustive match so a new `Action` variant cannot silently no-op. Helper subprocesses run through the
one bounded runner in `src/command.rs` on a blocking thread that enforces a timeout and caps captured
output at 64 KiB per stream. PowerShell/Batch scripts and the Lua `syn.run_cmd` bridge run through
the runner's tracked variant: each execution owns a Windows Job Object, so the action's deadline and
a stopped manual test terminate every descendant whatever its name or executable. A process that
leaves the tree by design (UAC elevation through AppInfo, Task Scheduler) is outside the job, a
failed job assignment falls back to `taskkill /T`, and the job is not kill-on-close, so a script that
starts a background program and exits leaves that program running. The budgets are 300 s for scripts,
600 s for archive helpers and 120 s for an elevation prompt. Outbound `http_webhook` actions use the
process-wide `reqwest` client and send configured headers as request headers, never through a shell;
a non-2xx response fails the step.

### Rule-to-rule control

`Action::RuleControl` runs inside the calling pipeline's task and cannot reach the engine, which
lives behind the daemon's mutex. The daemon installs a dispatcher (`src/daemon/engine/rule_call.rs`):
the action submits a `call` or `stop` request and awaits the reply, and a waiting call is resolved
outside the engine lock so a long pipeline never freezes the daemon. Every execution records the
call chain that created it, which is how direct self-calls and cycles are rejected before the target
starts and how nesting stops at eight rules. A `wait: true` call is signalled by the called
pipeline's execution guard, so a normal finish, an abort and a `stop` all release the caller. `stop`
aborts the target's active executions and releases their resources without touching `is_enabled`,
which separates it from the user-facing Stop button that also disables the rule. Executions are
identified by one process-wide monotonic id.

### Rule activity conditions

`RuleCondition::RuleRunning`, `WaitCondition::RuleStarted` and `WaitCondition::RuleStopped` observe
another rule instead of the machine. The engine publishes every pipeline start and finish into
`src/daemon/engine/activity.rs`, a process-wide map guarded by a short lock: a start is recorded
where the pipeline task is created, and the finish sits in the same execution guard that reports the
completion event, so a normal finish, an abort, a user stop and a `rule_control stop` all decrement
the counter through one path. Readers take that lock directly and never touch the engine mutex, so a
condition cannot freeze the daemon or deadlock against the pipeline it observes.

`active` counts the pipelines of the rule running right now and `owned` the live processes and
overlay shows a run still owns; a rule whose pipeline ended but still owns a resource reports
running. Waits poll the snapshot every 100 ms, and `RuleStarted` remembers the rule's `starts`
counter when the wait begins, so a start that already finished between two polls still satisfies it.

## Resource lifetimes (owned processes and overlays)

`start_process` and `overlay` actions carry a `ResourceLifetime`: `detach` (the default) leaves the
process or overlay alone, `until_run_end` releases it when the run that created it ends, and
`until_trigger_process_exit` keeps it alive until the triggering process exits. Only a
process-start run with a live pid can use the host lifetime; a rule call or test run without a
primary process downgrades it to `until_run_end` rather than leaking the resource.

The engine captures the spawned PID and the generation of the overlay show it put on screen. Teardown
kills that exact PID tree and hides the overlay only while the recorded generation is still the
active show, so a later show by another run is never hidden by an earlier cleanup. Every overlay show
is recorded for rule-stop cleanup, including a detached one; only a non-detached resource counts as
a live resource. `until_run_end` teardown runs on every pipeline ending, a user run first releases
the rule's previous non-detached resources, and a stop, abort or removal releases all of them. The
process loop prunes resources that disappeared on their own and updates the activity counter, so a
stale companion cannot keep a rule looking running.

Process-triggered rules choose the lifecycle event that fires their pipeline with `event`: `started`
(the default) or `exited`. Both run the same ordered pipeline through the ordinary trigger path, so
preconditions, cooldown, quota and analytics apply to either one. `single_instance` makes the start
pipeline fire only for the first matching instance and the exit pipeline only when the last tracked
instance has exited, deferring host-lifetime cleanup until then. A rule call or a test run without a
primary process has no host to wait for, so its host-lifetime resources are released with the run.

## Input simulation and Macro Studio

Keyboard and mouse events go through Win32 `SendInput`. Games and DirectInput consumers read
low-level scan codes and ignore virtual keys, so every simulated key carries a hardware scan code
from `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)` with `KEYEVENTF_SCANCODE`, and extended keys also set
`KEYEVENTF_EXTENDEDKEY`. Polled game engines sample input once per frame, so presses stay down for at
least 30–35 ms. Volume and media keys use the same path.

Macro Studio records low-level keyboard and mouse hooks (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`) while a
recording is active, groups events into a timeline and replays them with humanized jitter. The hook,
the polling fallback and the GUI capture path report the same physical transition, so a lock-free
per-key gate admits it once; a recorded tap is stretched to a 35 ms minimum hold, and stopping closes
every still-held key and mouse button. Relative mouse movement (`MouseCoordMode::Relative`) is
captured through Raw Input and replayed as short delta pulses. Named presets are saved and deleted
through the daemon; the modal writes `library.json` directly only after a failed liveness probe.

## Overlay Studio

Overlays are borderless, GPU-accelerated windows rendered by a dedicated process
(`--overlay-window <id>`), positioned by the daemon's supervisor across monitors with DPI and
orientation awareness, and assigned to a kill-on-close Job Object so they cannot outlive the daemon.
Optional `WDA_EXCLUDEFROMCAPTURE` keeps them out of screen capture. `SetText` and
`syn.overlay_set_text` publish a label through an atomically replaced file that the overlay polls, so
text updates repaint without restarting the process. Positions reported while the overlay is dragged
are coalesced and sent to the daemon by a worker, so the window procedure never blocks on IPC.

The studio itself is a WYSIWYG editor: multi-monitor artboard, snap-to-grid, pan/zoom, element
catalog, undo/redo, and interactive controls (buttons, toggles, sliders) bound to actions, volume,
opacity and live telemetry. The canvas is a non-interactive preview — it paints the real element
widgets but never forwards live pointer input, so an editor click cannot execute an element's
`on_click` rule. Every inspector property edit is one undo step, and elements are clamped to the card
only when the card or element geometry changes. Calibration is started from the studio and makes the
overlay draggable; saving a preset always creates a new id, and built-in presets are never replaced.

Timers run in the overlay process: a stopwatch adds and a countdown subtracts the real frame delta,
so a paused or minimized process catches up instead of drifting; clock and uptime are display-only. A
countdown reaching zero runs its `on_finish` action once per completed run, and a reset re-arms it.
Element click actions that need the daemon are queued to a worker instead of running on the render
thread. A `WindowRelative` overlay caches its target window handle, re-resolves it when it stops
being trackable, and converts the target's physical rectangle with the target monitor's DPI.

Media widgets show the current title and artist, an equalizer and transport controls. A progress bar
and a seek control are not rendered: playback position needs a Windows media-session (GSMTC)
integration that does not exist yet, and an always-zero bar with a dead seek knob would be fake UI.

## Sharing and backup

`src/sharing/` turns rules into `SYN1:` text codes: the JSON payload is Brotli-compressed with a
versioned custom dictionary, prefixed with an 8-byte SHA-256 checksum and Base64URL-encoded. A code
bundles the macro presets and overlays its rules reference, so the recipient gets a working rule set:
presets and overlays whose id exists locally with different content are re-identified, a
content-equal entry is reused, duplicate rules are skipped, colliding rule ids get a fresh id, and
bundled overlay button and toggle targets are re-pointed at the final imported rule ids. A target
that neither the code nor the local rules provide is reported as a warning.

Sender-local data never travels: runtime counters are reset, and custom icon paths plus overlay image
and button icon paths are dropped with a notice. A truncated or altered code fails the checksum
instead of decoding to garbage, and the comparison accumulates every differing byte. Decoding is
bounded (4 MiB of code, 8 MiB of decompressed payload, 500 rules, 500 overlays and 500 presets), so a
hostile code cannot exhaust memory. Export compresses on a worker thread and copies the code to the
clipboard; import is one daemon-owned `ImportShare` request that applies rules plus library entries
and creates the rules disabled by default. JSON files remain the backup format: `--rules-export` and
the CLI authoring commands cover file-based migration.

## Installation forms and updates

Synector ships in two forms. The portable zip is `Synector.exe` plus `LICENSE` and `NOTICES.md`; the
installer (`installer/synector.iss`, built only in CI) installs the same executable into
`Program Files`, adds Start Menu and `App Paths` integration and registers an uninstaller. A console
bootstrap (`installer/install.cmd`, or `install.ps1` fetched with `irm <url> | iex`) covers
machine-driven installs.

The daemon owns update work: it checks GitHub releases, streams the download and verifies it against
the SHA-256 digest published by the GitHub API. An asset without a published digest is refused and
the download deleted, so an update is never applied unverified. Every release also publishes
`SHA256SUMS.txt` for manual verification. A portable copy replaces its own executable in place; an
installed copy downloads the release's setup and starts it silently with
`/VERYSILENT /UpdateMode=gui|daemon`, letting the installer replace files, refresh the uninstaller
metadata and restart Synector. With process masking enabled the portable daemon runs from the
`WindowsServiceHost.exe` cache, so the updater targets the real executable: the GUI reports its own
path as `--gui-exe <path>` and a running GUI window is the fallback. A failed in-place swap restores
or keeps the previous executable and reports the failure. The GUI prepares the masked copy before it
spawns the daemon, and every masked daemon start verifies or refreshes it; when preparation fails the
daemon runs unmasked with the fallback logged. The restart keeps the daemon-only or GUI state that
was active before. Progress and status surface through the tray menu and overlay notifications, and
optional auto-install is a setting.

The PowerShell bootstrap that extracts the portable archive rejects entries with rooted paths or `..`
segments before resolving the destination, so a crafted archive cannot write outside the chosen
folder. The installer closes running instances and, for updates, removes the stale masked daemon copy
so it is rebuilt from the new binary. On uninstall the installer removes the autostart tasks, the
masked copy and the application files while keeping the user configuration directory.

## Rules: organization and editing

Rules carry custom colored tags, tag filters, and multi-mode sorting (manual/default, name ascending
or descending, trigger type, action count, status). In default sort order with no tag filter and no
active search, cards are reordered by drag handle and the new order is persisted through IPC;
filtered, searched or sorted modes disable reordering with an explanatory tooltip.

The GUI keeps an undo/redo draft per edited rule and per overlay definition (up to 50 snapshots)
through one bounded-history implementation (`src/gui/state/undo.rs`). An edit gesture is one undo
step — a drag or a slider commits the pre-gesture state once instead of one snapshot per frame. The
expanded Script Modal is the exception: it is a text surface and keeps its own bounded history of the
script text, owning Ctrl+Z / Ctrl+Y while it is open, so one undo in the dialog does not revert
unrelated rule fields. Opening a rule renders its stored values and writes no defaults into the draft
outside real widget interaction. Save runs the same structural validator as the CLI before it sends
anything; the errors are listed in the inspector and the editor stays open, and the daemon validates
again when it applies the mutation.

## Logging and localization

Every user-visible string lives in the `Strings` catalog (`src/i18n/strings.rs`, `src/i18n/en.rs`,
`src/i18n/ru.rs`). The in-app guide is the deliberate exception: its long prose is written as
Russian/English branches inside `src/gui/views/guide_modal.rs`, while its chrome labels come from
`Strings`. Both language branches must be updated together.

Diagnostics go through `crate::logging` into daily files under `%LOCALAPPDATA%\Synector\logs\`; the
level comes from `SYNECTOR_LOG` (`off|error|warn|info|debug|trace`, default `info`). File logging can
also be turned off with the `log_to_disk` setting, which stops new events without deleting existing
files; `SYNECTOR_LOG=off` remains an independent override. Log identifiers and outcomes only — never
script bodies, webhook payloads, rule JSON or other user secrets; action errors that may embed script
output go through the engine's `SensitiveLog` event. Every append and compaction is serialized across
processes with a named mutex, so the daemon, the GUI and a CLI command cannot interleave a write.
