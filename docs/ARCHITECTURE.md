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
|  - Dark and light palettes                                   |
|  - English and Russian locales (src/i18n)                    |
|  - Rule Builder, Macro Studio, Overlay Studio, Inspector     |
|  - Global search, Analytics and Settings dialogs             |
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
| `--daemon` | Background service only (tray mode; headless if the tray cannot be created) |
| `--status` | Print daemon status over IPC and exit |
| `--overlay-window <id>` | Standalone borderless overlay window for one overlay (`--calibration`, `--duration`, `--override-text`, `--overlay-data`) |
| `--run-rule <rule id>` | Invoke a rule directly: `TestRunRule` over IPC when the daemon runs, otherwise an in-process run through the engine's shared step loop; a missing or empty id is a usage error that exits with code 2 |
| `--rules-*` | Rule authoring CLI: validate, apply, sync, list, export, enable/disable, reset, delete, share, import, schema and templates. See [RULE_AUTHORING.md#commands](RULE_AUTHORING.md#commands). |

After the overlay fast path and the CLI rule commands, the remaining arguments are validated as
runtime flags. An unknown flag, or a value flag (`--run-rule`, `--overlay-window`) with a missing
or empty value, prints `error:` plus the usage block on stderr and exits with code 2 instead of
starting the GUI behind an elevation prompt. The CLI rule commands keep their own strict parser.

When no daemon answers, `--run-rule` runs the rule through the engine's shared step loop, so
`PrevOutput`, `StepN.Output`, `AbortPipeline` and `stop_on_error` behave exactly as in a daemon run.
It applies the same gates as the GUI Run button: the quota and the rule's preconditions are
enforced, while the cooldown and `is_enabled` are not, so a disabled rule can be tested. The
differences are deliberate consequences of there being no daemon: nothing is persisted (analytics,
runtime counters and a lifetime-quota auto-pause), rule-to-rule control has no dispatcher, and the
context carries `ProcessId` 0 with no live process path — `ProcessName` is the rule's
process/window/resource target when it has one and `Manual Run` otherwise.

Synector runs elevated by default (`--no-elevate` opts out). Elevation enables kernel ETW process
tracing, UIPI-safe input simulation into elevated applications, and silent highest-privilege logon
autostart through Task Scheduler. The GUI forwards `--no-elevate` to the daemon it spawns, so the
opt-out covers the whole process tree rather than just the GUI, and reports its own executable path
to the daemon as `--gui-exe <path>` so a portable update with process masking targets the real
executable after the window is closed (see
[Installation forms and updates](#installation-forms-and-updates)).

`src/app/` (entered from `src/main.rs`) dispatches in a fixed order: standalone overlay windows →
CLI rule commands → update-file cleanup → `--status`/`--run-rule` → self-elevation → daemon or GUI.
Runtime flags, overlay companions and CLI command spellings come from one vocabulary table in
`src/app/args.rs`; the CLI keeps its own parser but a gate test pins that the two spellings sets
agree. Overlay windows are separate
processes and must stay on the fast path; they never touch update files or single-instance mutexes.
CLI rule commands run before elevation because they are console tools.

Daemon liveness is a named-pipe ping (`IpcClient::is_daemon_running()`), not a single-instance mutex
probe: a mutex only shows that some process holds the name, while the pipe shows that the daemon
answers. Raw mutex probing stays private to `SingleInstanceGuard`, which uses it for GUI liveness.
A daemon whose tray cannot be created runs headless, but it still exits through the coordinated
shutdown when asked: On App Close rules stop the engine, running overlays are closed synchronously
so their fade-out plays instead of the kill-on-close Job Object ending them, and only then are the
runtime counters and overlay positions flushed. That wiring lives in `src/daemon/runtime/`:
`bootstrap` creates the Tokio runtime and installs the rule-control dispatcher, hotkey listener, IPC
server and background loops in their fixed order, `loops` holds each long-lived loop body, `launch`
records the `--gui-exe` path and `shutdown` owns the signal plus the ordered sequence above.

## Layers

| Area | Responsibility |
|---|---|
| `src/gui/` | egui views and inspectors (`src/gui/views/inspector/`), the rules view (`src/gui/views/rules_view/` behind `src/gui/views/rules_view.rs`), the overlay studio (`src/gui/views/overlay_modal/` behind `src/gui/views/overlay_modal.rs`) and macro studio (`src/gui/views/macro_modal/` behind `src/gui/views/macro_modal.rs`) module trees, the catalog modals (`src/gui/views/catalogs/`), the preset-picker body shared by both studios (`src/gui/views/presets.rs`), the header bar and its global search overlay (`src/gui/views/header_bar.rs`, `src/gui/views/global_search.rs`), the analytics and settings dialogs (`src/gui/views/analytics_modal.rs`, `src/gui/views/settings_modal.rs`), the shared catalog shell and GUI primitives (`src/gui/components/`, including the colour editor in `src/gui/components/color_picker.rs`, tag pills in `src/gui/components/chips.rs` and the hand-painted charts in `src/gui/components/charts.rs`), overlay renderer, macro recorder, and the GUI state hub (`src/gui/state.rs` facade over `src/gui/state/`) with its IPC synchronization |
| `src/daemon.rs`, `src/daemon/runtime/` | Daemon tier entry: the module list and `run_daemon`, with the start-up/loop/shutdown wiring split into `bootstrap` (runtime creation and fixed install/spawn order), `loops` (process, window, system-event, schedule, hotkey, file, webhook, coalesced-flush and update-check loops), `shutdown` (the signal and the coordinated sequence) and `launch` (the `--gui-exe` path) |
| `src/daemon/engine/` | Rule runtime split by owner: `scheduler`, `quota`, `executions`, `pipeline` (the one pipeline launcher), `projector`, `triggers`, `state`, `ipc`, `offline`, `update` (the updater display state), `rule_call` (the rule-control dispatcher) and `activity` (the rule-activity registry), plus the engine suite; `src/daemon/engine.rs` is the `DaemonEngine` facade |
| `src/daemon/actions/` | One module per action family (`process`, `window`, `input`, `macro`, `audio`, `script`, `filesystem`, `webhook`, `system`, `overlay`, `tokens`) over the shared bounded runner in `src/command.rs` and the MTA COM guard in `src/win_com.rs`; `src/daemon/actions.rs` is the facade, the control-flow dispatcher and the exhaustive standard-action family dispatcher; command-line/PowerShell quoting lives in `src/win_args.rs` |
| `src/daemon/conditions.rs` | Condition evaluation and process/window/device matching shared by the engine ticks and the action handlers (`evaluate_condition`, `matches_process`, `device_filter_matches`, `strip_exe_suffix`, foreground-process probe); action code no longer reaches the engine for these |
| `src/daemon/overlay_manager/` | Overlay child supervision split into `supervisor`, `job`, `text`, `monitors` and `update`; `src/daemon/overlay_manager.rs` re-exports the unchanged surface |
| `src/daemon/` services | Process watcher, hotkeys, system events, telemetry (`src/daemon/telemetry/`), the rolling analytics collector (`src/daemon/analytics.rs`), tray (`src/daemon/tray/`), file watcher, Lua runner, webhook server, named-pipe server, Discord RPC |
| `src/app/` | Process entry and dispatch: fixed stage order, the shared argv vocabulary (`src/app/args.rs`; spellings in `src/arg_spellings.rs`), daemon spawn, elevation, console reattachment, update-file cleanup, `--status` and `--run-rule` |
| `src/win_args.rs` | One Windows command-line helper set: UTF-16-equivalent argument splitting/quoting/joining, PowerShell single-quoted literal escaping and the no-window spawn flag shared by the daemon, the app tier, updater, autorun and shortcuts |
| `src/command.rs` | One bounded subprocess runner and one `CommandOutcome`: spawn/wait errors keep the action wording, output is capped at 64 KiB per stream with truncation flags, a timeout kills the child (optionally its process tree, used by the Lua `syn.run_cmd` bridge), and `ensure_success` centralises the utility timeout/exit-code wording |
| `src/win_com.rs` | One COM apartment guard (MTA or STA, infallible or fallible) shared by action audio, telemetry audio and shortcut creation; it uninitialises only when `CoInitializeEx` returned `S_OK`, so a thread another component already initialised is never decremented |
| `src/daemon/engine/rule_call.rs` | Rule-to-rule control channel between `Action::RuleControl` and the engine dispatcher; the historical `crate::daemon::rule_call` module path is kept as a re-export |
| `src/daemon/engine/activity.rs` | Process-wide running-pipeline snapshot behind rule-activity conditions; the historical `crate::daemon::rule_activity` module path is kept as a re-export |
| `src/ipc/` | Named-pipe client, the shared request/response protocol, the one framing implementation both sides use, and the overlay wire encoding |
| `src/models/` | Domain model: `Rule`, `Trigger`, `Action`, conditions, overlays, macro presets, quotas, raw-key/schedule validation, the shared action-tree visitor (`Rule::visit_actions`), the analytics snapshot wire types (`src/models/analytics.rs`) and the generated rule schema (`src/models.rs` is the facade) |
| `src/validation.rs` | Structural rule validation and non-blocking warnings shared by the CLI, config loading, engine mutations and the GUI save path |
| `src/cli/` | Command-line rule authoring: argument parser, command bodies, upsert/sync planning, share codes, JSON envelopes and the daemon/offline target selection in `src/cli/target.rs`, which also owns the one IPC round-trip adapter behind the command-specific `*_via_daemon` functions and the one post-snapshot offline→daemon transition helper (`src/cli.rs` is the facade) |
| `src/config/` | Split configuration persistence: storage policy, caches and the four section modules (`src/config.rs` is the facade) |
| `src/i18n/` | Locale registry and the Windows UI-language detection (`src/i18n.rs`), the flat `Strings` declaration (`src/i18n/strings.rs`) and the English/Russian string catalogs |
| `src/autorun.rs` | Task Scheduler autostart registration |
| `src/updater/` | GitHub release checks, asset selection, SHA-256 verification, portable self-replacement, silent installer updates and stale update-file/`.old` executable cleanup (`src/updater.rs` is the facade over `release`, `install_mode`, `portable`, `install`, `apply`, `temp` and `util`) |
| `src/sharing/` | `SYN1:` Brotli + Base64 share codes split by stage (`format`, `payload`, `encode`, `decode`, `import`, `assets`; `src/sharing.rs` is the facade): SHA-256 prefix, library dependency bundling, bounded decode |
| `src/clipboard.rs` | One system-clipboard helper set (`copy_text_to_clipboard`, `read_text`) shared by the share flow, the inspector and token expansion |
| `src/shortcut.rs` | Windows `.lnk` generation for direct rule launch, written through `IShellLink`; a same-named foreign shortcut is never overwritten (a rule-id-suffixed name is used instead) |
| `src/single_instance.rs` | Named mutex guards for daemon and GUI |

### Design tokens and skins

The GUI reads every colour and every geometry value from one token layer, `src/gui/theme.rs`
(facade over `src/gui/theme/`). `tokens` owns the dark and light palettes and the resolved
`Tokens` value a view reads; `skin` owns the geometry, density and stroke metrics; `style` derives
egui's `Visuals`, spacing and text styles from both; `motion` owns the platform motion permission;
`paint` holds the shared painting helpers and `fonts` registers the bundled type stacks.

The palette and the skin are independent axes: switching one never changes the other. A view
asks for a role (`t.sunken`), a helper (`t.hairline()`) or a metric (`t.metrics().card_radius`)
rather than branching on either, which is what lets a second visual generation ship as a new
metrics value instead of a second view tree. The few surfaces that genuinely compose differently
per generation — the filter cells, the appearance previews — read `Metrics::skin` for that one
branch, never to pick a colour.

A skin owns geometry (radii, control and bar heights), strokes, elevation (`Shadow`) and motion
(`Motion`). The type and colour voice is shared: both generations keep the mono-caps labels and the
Archivo display face, and both palettes are orthogonal to the skin, so both skins must stay
readable in both. The generations differ in how they compose chrome: the classic one keeps flat,
opaque panels with square cards, while the modern one lifts tinted translucent surfaces
(`Tokens::raised`, `Tokens::surface`) and draws the selected filter cell as a lifted pill instead
of an inverted block. The modern generation is the default; the classic one stays selectable from
Settings (`ui_skin`).

A user accent (`settings.accent`) re-tints `Palette::accent`, derives a readable `accent_ink` from
it and tints the selection background; an accent change is applied at once instead of fading,
because a fade would chase the colour picker's drag. Omitting the value keeps the palette's own
accent, and the Settings reset button clears the override.

Windows can decide two things here: the interface language once, and the palette continuously if
asked to. The first launch adopts the Windows UI language (`Language::system` from
`GetUserDefaultUILanguage`, mapped onto the shipped catalogs: Russian for `LANG_RUSSIAN`, English for
everything else) and persists it with the section it materializes; every later launch keeps the
persisted choice. The palette stays the user's unless `follow_system_theme` is on (it is off by
default): the GUI then renders the palette Windows applies to its own apps and re-reads it at most
once a second, so a switch made in Windows is picked up without a restart. That palette comes from
`AppTheme::system` (`AppsUseLightTheme` under `HKCU\…\Themes\Personalize`), the same lookup the
daemon uses to shade the tray menu, and it is the palette `Settings::effective_theme` reports; a
missing or unreadable value keeps the configured `theme`. Picking a palette by hand turns the switch
off, and the switch itself stores the palette currently on screen as `theme` — that field is both the
explicit fallback and the effective palette. Neither the adopted palette nor the switch raises the
theme toast: automatic switching is not a user edit and writes nothing to disk.

Motion has two owners: the skin sets the durations, and `theme::motion` asks Windows whether client
area animations are enabled (`SPI_GETCLIENTAREAANIMATION`, cached for the process). When the system
says no, every duration collapses to zero, transitions return their target immediately and no
transition schedules a repaint. Components animate user-triggered state only — hover, press, a
drawer, a dialog, a drop indicator — never decoration. A palette change additionally cross-fades
the colour roles (`Palette::lerp`, 0.18 s): views read the blended palette while the fade runs, and
each frame re-derives the native widget visuals until it settles, when the target visuals are
applied once more. A skin change never fades colours, because the palette is unchanged.

`src/gui/theme.rs` also owns one convention test, `views_never_pick_a_literal_frame_radius`: it
walks `src/gui/views/**` and fails if a `Frame` carries a literal corner radius instead of a skin
helper, so the sweep cannot erode unnoticed.

The window backdrop (`theme::paint_backdrop`) is painted once behind every panel. The classic skin
paints the flat page colour; the modern skin paints a quiet two-stop wash derived from the palette,
which its translucent panels (`surface_alpha`) let through. Nothing decorative is painted on top of
it.

## Window shell, dialogs and global search

The main window has no navigation tabs: the rules list is the only page, and the header owns the
one rules search field, centered between the wordmark and the window caption buttons. The field is
deliberately background-free with only an outline, and sized to the glyph buttons beside it, so it
reads as title-bar chrome rather than as a form; every other search field in the app (settings,
catalogs, process picker, analytics logs) reuses the same outlined chrome, while widths and heights
stay per call site. The analytics and settings surfaces are base-class dialogs
(`src/gui/components/modal.rs`) opened from two glyph buttons right of the field. A dialog joins
`GuiState::is_modal_open()`, opens with the shared scrim and animation, and opening one surrenders
the search focus so typing cannot land in the field behind the scrim.

Typing a query opens the global search overlay instead of a plain dropdown: a scrim `Area` covers
everything below the header while a results panel groups matches by the surface that can consume
them — rules, settings, the activity log, actions, triggers, conditions, overlay presets and macro
presets — with a count and up to three previews each. Applying a group keeps the query in the
header for rules (which are already filtered behind the panel) and transfers it into the target's
own search field for every other surface: the settings dialog, the analytics log filter, a
rule-builder catalog over a fresh or current draft, the overlay studio's preset picker, or the
Macro Studio opened on a draft rule with the matched preset attached and its preset picker filtered.
The overlay is not a `Modal`: the header field stays visible and editable, and Escape closes the
results first, then clears the query.

The analytics dashboard reads daemon-owned data. `src/daemon/analytics.rs` keeps a rolling
24-hour window bucketed by local wall-clock hour — the current hour plus the 24 before it — with
runs, outcomes, per-source and per-rule counts, and the last two minutes of CPU/RAM samples. The
one-second runtime flush loop takes those two numbers with a CPU/RAM-only probe (no battery read,
no COM audio endpoint) outside the engine lock, and the dashboard draws the last minute of them.
The same tick samples the daemon's own CPU share, private working set and disk throughput from the
process I/O counters (`DaemonUsage`), shown as one line in the SYSTEM LOAD card; network and GPU
are deliberately absent because Windows exposes no cheap per-process counter for either and a
daemon's share of them is effectively zero.
The shared step loop reports every finished pipeline once (`PipelineOutcome`), so an error that
does not halt a pipeline still counts; a run that never reports an outcome is recorded as aborted
by its execution guard, so a user stop, a replaced run and a stopped recurring cycle are counted in
one place. The GUI polls one narrow
`GetAnalytics` request per second only while its dialog is open; the ordinary state snapshot does
not carry the analytics payload. The dashboard paints charts with
`src/gui/components/charts.rs` on `Tokens` roles and `Tokens::series`, so there is no plotting
dependency and both palettes and a custom accent stay coherent.

## Data flow

1. The GUI reads state over IPC and holds an editable draft with undo/redo history.
2. Mutations are queued to one IPC worker thread and sent as `IpcRequest` values; the daemon
   validates them, applies them to its in-memory `AppConfig` aggregate and persists only the
   affected config section under `%LOCALAPPDATA%\Synector\`. The editable draft is discarded only
   after the daemon confirms the mutation; a rejected or failed mutation keeps the draft open and
   reports the error. Every other GUI round-trip goes through the same worker too — terminating a
   process, saving settings, preset-library writes, overlay-preview show/hide, the calibration
   position poll and the analytics dashboard poll — so the render loop never blocks on the named
   pipe; the frame thread only enqueues and applies results, and the latest result wins. Testing a
   step is the one exception: it executes an action inline on the daemon and may legitimately run
   for minutes, so the worker hands it to its own thread instead of serializing the state poll
   behind it. The inspector paints the tested step with the same focus an executing step gets and
   clears it when the daemon reports completion; the editor paints no result badge (the outcome is
   in the daemon log). While a test runs, its Play button becomes Stop: the GUI sends
   `StopTestStep` with the test's correlation id, the daemon terminates the script's Job Object and
   aborts the action task, exactly like `StopRuleExecution` aborts a Test Run. A stop that overtakes
   its start is remembered and refuses the request when it arrives.
3. The daemon executes rules and streams status, telemetry and execution snapshots back, so the GUI
   reflects engine state without owning it. A sustained daemon outage (three failed polls plus a
   one-second quiet period) is logged and shown as an offline toast instead of being collapsed into
   a retry counter, and the window stays open with the offline state so unsaved drafts survive.
4. Overlay windows are separate GPU-accelerated processes supervised by the daemon; the GUI only
   edits their definitions.

IPC round-trips are bounded. A client call gets 5 s (750 ms for the liveness ping) and an inline
`TestStep` gets the action's own worst-case wall clock plus a 60 s slack
(`daemon::actions::max_execution_duration`, whose exhaustive match forces every action to state
its ceiling), so a long script test is not mistaken for a hung daemon; a busy pipe
(`ERROR_PIPE_BUSY`) is retried briefly because another process may be connecting to the single
listening instance, and a missing pipe is re-checked a few times before
it counts as absent. A call that exceeds its bound cancels its blocked worker
(`CancelSynchronousIo`) and joins it, so the pipe handle is released and repeated timeouts cannot
accumulate threads; the next call opens a fresh instance. During GUI exit the round-trips share a
1.5 s shutdown budget, window geometry is persisted first, and a spent budget returns immediately
instead of waiting out another timeout. The typed transport error separates `Unavailable` (the pipe
does not exist) from `Busy`/`Timeout` (the daemon is alive but not answering); only `Unavailable`
is proof that the daemon is offline, so a slow or busy daemon never triggers an offline fallback.
Frames are capped at 16 MiB in both directions: the client rejects an over-cap response length
before reading the body, and the daemon replaces an oversized response (an implausibly large state
snapshot) with a small `Error` response so the caller reports an error instead of losing the
connection.

## Configuration storage

The configuration is split into four independent, versioned JSON files in
`%LOCALAPPDATA%\Synector\` (relocatable through `SYNECTOR_CONFIG_DIR` for portable or isolated runs).
The sections and their user-facing contents are listed in
[README.md#configuration](../README.md#configuration); this section covers how they are written.
`AppConfig` exists only in memory: it is assembled from the sections on load and exchanged over IPC,
never written as one document. This keeps write volume proportional to what changed.

Every write goes through a temporary file that is flushed and renamed over the target, so a crash
mid-write cannot truncate user data; `<section>.<pid>.tmp` files left behind by a writer that died
before the rename are swept once they are five minutes old, because a younger one may still belong
to a live writer. Every section carries a schema version: an older file is upgraded in memory and
stamped with `CONFIG_VERSION` on its next save, while a file written by a newer build is read as-is
and never overwritten — the refusal is logged and returned as a save error, so this build cannot
downgrade it. A section that fails to parse is moved aside as `<name>.corrupt-<timestamp>` (with a
`-1`, `-2`, … suffix when that name is taken, so an earlier quarantine copy is never replaced)
before defaults take over; if the file cannot be moved aside, writes to that section are refused
until it is moved or repaired by hand. First-run materialization creates only the section files
that are missing and writes each one from the state already loaded, so an existing `runtime.json`
keeps its counters even when another section has to be created or quarantined. A first run — no
`settings.json` yet — also adopts the Windows UI language before the section is written; from then
on the persisted `language` is the user's choice and is never re-detected.

Rule-level strictness is separate from section parsing. `rules.json` entries are checked with
`validate_rule_keys` before deserialization: a rule carrying an unknown key is logged with its rule
id and keys and skipped, while the rest of the file loads. After deserialization each rule also runs
through the same per-rule validator as the GUI Save and CLI apply paths: a rule whose values are
invalid (an out-of-range custom quota window, a wait outside 0..=86400 s, a malformed schedule) is
skipped on its own, with its id and errors logged. Neither check drops a rule the GUI or CLI would
accept. The unknown-key check is how the fields removed from the model (`schedule.delay_sec`, the
`system` trigger's `idle_duration_sec`, the `resource` condition's `duration_sec`) are handled — no
migration shim rewrites old files, so such a rule must be updated. `--rules-apply` and share import
reject the input instead of skipping. Overlay actions are normalized on the same load, so the
embedded definition and `overlay_id` share one id and a `close_on_exit` binding refers to the
overlay that actually runs.

The daemon is the only writer while it is reachable. The GUI sends section-scoped requests
(`SaveSettings`, `SaveRule`, `ReorderRules`, `ResetRule`, `SaveOverlay`, ...), so the two processes
cannot race on a read-modify-write of the same file. Rule, overlay-library and runtime-counter
fallbacks run only after a failed liveness probe: `rules.json` (`SaveRule`, `DeleteRule`,
`ToggleRule`, `ReorderRules`), `library.json` (`SaveOverlay`, `SaveMacroPreset`) and the `ResetRule`
counters in `runtime.json`; a reachable daemon that rejects one of those is reported instead of being
bypassed. `--rules-sync` is a single `SyncRules` request that replaces the rule set, persists once,
reloads hotkeys once and prunes counters of removed rules in one step. An offline CLI write re-checks
daemon liveness immediately before persisting and switches to IPC if the daemon became reachable in
between; offline delete and sync also prune the removed rules' counters from `runtime.json`. An
offline share import commits `library.json` before `rules.json`, so imported rules never reference a
missing preset or overlay.

Hot sections are read through a process-local cache (`settings.json`, `library.json`) that validates
the cached value against the file's size and modification time: action steps, Lua `syn.overlay_*`
calls and overlay windows avoid re-reading and re-parsing the file, while an externally written file
is still picked up immediately. A cached stamp is trusted only after the file's timestamp is at
least two seconds old, so a coarse filesystem timestamp cannot hide an equal-length rewrite, and a
failed read is never cached, so a transient sharing or permission error does not pin the default
section. Runtime counters, overlay drag positions and main-window geometry are written coalesced
(bounded intervals, plus a forced flush on quota pause, reset, window exit and shutdown), so a
fast-firing rule, a dragged HUD or a resized window does not turn every event into a disk write.
Under `cfg(test)` the config directory is an isolated per-process temporary directory, so tests
never touch real user data.

## Event sources and polling

Synector prefers push notifications, but Windows does not expose every event that way. The watcher
subscribes to kernel ETW process lifecycle events when elevated; if tracing cannot start, the
process is not elevated, or the realtime `ProcessTrace` thread exits, it degrades to adaptive
Toolhelp32 snapshot polling. The ETW callback only queues `(pid, event)`; names are resolved on the
daemon thread from the image path first and from a Toolhelp snapshot second, so protected/PPL
processes produce the same events as polling. A name that cannot be resolved when the start arrives
is filled by the periodic reconcile from a fresh snapshot, which then emits the deferred start
event once; a pid that already exited is dropped with one debug line. The trace thread records the
`ProcessTrace` result, so an exited trace is logged and the same tick falls through to polling. The ETW session name carries
the Windows session id (`SynectorKernelProcessTrace-<session-id>`), because the session is
machine-wide while daemons are per user. Both paths feed the same trigger surface, so a rule never
depends on which one is active.

Other sources are mixed by nature:

- **Push/event sources:** kernel ETW process lifecycle records, Win32 named-pipe IPC, the local
  webhook server (`127.0.0.1:48123`), low-level keyboard/mouse hooks during macro recording, Raw
  Input camera deltas, `WM_POWERBROADCAST` for sleep/resume and console display state, and
  `WM_DEVICECHANGE` for plug-and-play devices. The webhook server only listens while at least one
  enabled rule uses the `webhook` trigger; the daemon re-reads the rule set and starts or stops the
  listener, so a daemon whose rules do not use the trigger holds no socket and logs no listener line.
- **Polling sources:** Toolhelp32 process snapshots when ETW is unavailable, foreground-window
  detection, the file watcher, resource thresholds, session lock, AC/battery power source,
  user idle/active, network state and system telemetry (CPU, RAM, network, battery, volume).
  Polling loops back off when no rule uses the corresponding trigger: the process snapshots drop to
  a 3 s dormancy tick when no process or resource rule and no bound companion needs them, foreground
  focus drops to 3 s without a `window_focus` rule, and the system-event sampler drops to 10 s when
  no enabled rule has a system trigger or an `event` condition (its notification window still wakes
  it for suspend, display and device broadcasts). The system trigger's idle
  event uses a fixed five-minute threshold; only the `event` condition carries a rule-authored
  `idle_duration_sec`.
- **Global hotkeys:** `hotkeys.rs` checks `GetAsyncKeyState` for the registered combinations on a
  short interval (8 ms while hotkeys exist) and blocks on a condition variable with no periodic
  wake-up when there are none. This is a polling design, not a `RegisterHotKey` hook; it is what
  allows mouse-button combinations and hold/repeat modes to share one code path.

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

A rule pairs one optional trigger with preconditions, one ordered action pipeline, and
cooldown/quota limits. `src/daemon/engine.rs` is the `DaemonEngine` facade: it schedules
triggered runs, evaluates conditions, enforces quotas and cooldowns, tracks active executions and
reports analytics. The implementation is split by owner under `src/daemon/engine/`: `scheduler`
owns admission and tick throttling, `quota` the documented counter semantics, `executions` active
pipelines, abort handles, recurring test loops and bound satellites, `pipeline` the step loop, the
single launch machinery behind the triggered/recurring paths and the call/context entry points,
`triggers` the process/window/schedule/resource/system intake, `projector` the read-only
`DaemonState` snapshot the IPC layer serializes, `state` the log buffer and section persistence,
`ipc` the request dispatcher and rule writers, `offline` the `--run-rule` fallback, `update` the
updater display state (`UpdateStatus`), `rule_call` the dispatcher installed by the daemon,
`activity` the process-wide activity registry, and `tests` the engine suite. The bound-satellite,
bound-overlay and rule-overlay registries live in the tracker, so one teardown owner is responsible
for them; `DaemonEngine::trigger(rule, origin, context_name, pid, extra_ctx)` is the only pipeline
entry point and every caller states the `ExecutionOrigin` explicitly (there is no string-to-origin
fallback). Condition evaluation and process/window/device matching live in `src/daemon/conditions.rs`,
which the action handlers call directly; `Rule::visit_actions` in the model is the one read-only
action-tree traversal used by the engine, sharing, validation and the GUI, including the
active-overlay branch of `is_rule_executing`, which scans nested `IfElse` branches as well as the
top-level pipeline.

- **Cooldown:** minimum time between two triggered runs. It is seeded from `last_executed_at`
  when the daemon starts, so a restart does not clear a cooldown that is still running; a run the
  user starts directly (Run/Test Run, an overlay click target or `--run-rule`) bypasses it.
- **Lifetime quota:** `max_executions` without `quota_period`. When it is reached the engine logs
  `completed quota (x/y) and has been auto-paused` at WARN, pauses the rule, and the rule can be
  re-enabled with `--rules-reset` or the GUI reset action.
- **Periodic quota:** `quota_period` keeps the rule enabled and skips executions until the counter
  rolls over; the rollover logs `reached its quota (x/y); the counter resets …` at INFO. One helper
  classifies both cases and owns their messages. The exact reset rules and custom windows are
  documented in [RULE_AUTHORING.md#quotas](RULE_AUTHORING.md#quotas).
- **Runtime counter ownership:** `execution_count`, `last_executed_at` and `quota_window_start` live
  in `runtime.json`, are zeroed in `rules.json`, and are preserved across ordinary rule updates. A
  changed `quota_period`, or enabling a limit where there was none, starts a fresh counter;
  `last_executed_at` is always kept because schedule triggers use it for deduplication. Changing
  only the numeric limit keeps the counter.

Pipelines support waits, inline checks and branching:

- `Action::Wait` delays by timer or by a system condition (window opened/closed, process
  started/exited, hotkey pressed, file created/deleted, another rule starting or stopping) with a
  timeout and an abort-or-continue timeout policy.
- `Action::Check` evaluates environmental criteria mid-pipeline and halts the remaining steps
  cleanly when unsatisfied, without raising an error. `RuleCondition::RuleRunning` observes another
  rule's pipeline activity here, in `IfElse` and in rule pre-conditions.
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

Schedule triggers are admitted by the scheduler rather than by scanning every rule on each tick. A
one-shot schedule is consumed only when its run actually starts, so a one-shot whose preconditions
or quota rejected the attempt stays armed.

Action handlers are asynchronous. Each action family lives in its own module under
`src/daemon/actions/` (`process`, `window`, `input`, `macro`, `audio`, `script`, `filesystem`,
`webhook`, `system`, `overlay`, `tokens`); the standard-action dispatcher matches every family
exhaustively, so a new `Action` variant cannot silently no-op. Helper subprocesses — scripts, archive
tools, audio/device helpers and elevated launches — run through the one bounded runner in
`src/command.rs` on a blocking thread that enforces a timeout and caps captured output at 64 KiB
per stream. PowerShell/Batch scripts and the Lua `syn.run_cmd` bridge run through the runner's
tracked variant: each execution owns a Windows Job Object, so the action's deadline and a stopped
manual step test (`StopTestStep`) terminate every descendant whatever its name or executable. A
process that leaves the tree by design (UAC elevation through AppInfo, Task Scheduler) is outside
the job, a failed job assignment falls back to `taskkill /T`, and the job is not kill-on-close, so a
script that starts a background program and exits leaves that program running as before. The Windows
argument quoter, PowerShell literal escaper and no-window spawn flag live in `src/win_args.rs`; the
COM apartment guard lives in `src/win_com.rs`. The budgets are 300 s for PowerShell/Batch scripts,
600 s for archive helpers and 120 s for an elevation prompt; a spawn failure, non-zero exit or
timeout is reported as an action error instead of being ignored. Outbound `http_webhook` actions use
the process-wide `reqwest` client and send configured headers as request headers, never through a
shell; a non-2xx response fails the step. Discord Rich Presence actions validate the reply frame —
opcode, nonce and an `evt: ERROR` rejection — so a rejected presence update is an action error
rather than a silent success.

### Rule-to-rule control

`Action::RuleControl` runs inside the calling pipeline's task and cannot reach the engine, which
lives behind the daemon's mutex. The daemon installs a dispatcher (`src/daemon/engine/rule_call.rs`,
the historical `crate::daemon::rule_call` path): the
action submits a `call` or `stop` request and awaits the reply, and a waiting call is resolved
outside the engine lock so a long pipeline never freezes the daemon.

Every execution records the call chain that created it (`ActiveExecutionItem::call_chain`). The
chain is how a direct self-call and a call to a rule that is already running in the same chain are
rejected before the target starts, and how nesting stops at eight rules. A `wait: true` call is
signalled by the called pipeline's execution guard, so a normal finish, an abort and a `stop` all
release the caller.

A `call` goes through the same gates as a triggered run: quota, cooldown, conditions and
`is_enabled`; a skipped call is logged and is not an error for the caller. `stop` aborts the target's
active executions and releases their resources without touching `is_enabled`, which separates it
from the user-facing Stop button: that one also disables the rule and persists the pause. The GUI
Run/Test Run emulation, releasing a hold-mode hotkey and a `rule_control` stop abort only the
running executions, so the rule stays enabled for its next trigger. Executions are identified by a
process-wide monotonic id shared by triggered, exit, recurring and called pipelines; active
executions, abort handles and call chains are keyed by it.

### Rule activity conditions

`RuleCondition::RuleRunning`, `WaitCondition::RuleStarted` and `WaitCondition::RuleStopped` observe
another rule instead of the machine. The engine publishes every pipeline start and finish into
`src/daemon/engine/activity.rs` (the historical `crate::daemon::rule_activity` path),
a process-wide map guarded by a short lock: a start is recorded where
the pipeline task is created, and the finish sits in the same execution guard that reports the
completion event, so a normal finish, an abort, a user stop and a `rule_control stop` all decrement
the counter through one path. Readers (wait polling, check, if/else and pre-conditions) take that
lock directly and never touch the engine mutex, so a condition cannot freeze the daemon or deadlock
against the pipeline it observes.

`active` is the number of pipelines of the rule that run right now. That is the same state
`rule_control stop` acts on: a rule whose pipeline ended but whose companion process or overlay is
still alive reports stopped. Waits poll the snapshot every 100 ms like the other wait conditions;
`RuleStarted` additionally remembers the rule's monotonic `starts` counter when the wait begins, so
a start that already finished between two polls still satisfies it. An empty target never satisfies
a rule-activity condition; a target that does not exist is simply never running, and the CLI
validator warns about both.

## Companion mode (bound satellites)

`Action::StartProcess { close_on_exit: true }` attaches a companion process to a primary
application: the satellite starts with it and terminates when it exits. `Action::Overlay` has the
same `close_on_exit` flag for HUDs. While the primary application runs, the binding is tracked in
the daemon's active execution state so the rule list and analytics show it. Only a run the user
starts directly (Run/Test Run, an overlay click target or `--run-rule`) force-terminates the rule's
bound satellites at start and after the pipeline finishes; scheduled, webhook, hotkey and
system-event runs leave them tracked while the primary process keeps running.

Process-triggered rules choose which lifecycle event fires their pipeline with `event`: `started`
(the default) or `exited`. Both choices run the same ordered pipeline through the ordinary trigger
path, so pre-conditions, cooldown, quota and analytics apply to either one; a start-triggered run
can bind `close_on_exit` satellites, an exit-triggered run cannot because its primary process is
already gone. `single_instance` does not prevent more copies of the application from starting; it
makes the start pipeline fire only for the first matching instance and the exit pipeline fire only
when the last tracked instance has exited, and it defers bound-satellite cleanup until then.

## Input simulation and Macro Studio

Keyboard and mouse events go through Win32 `SendInput`. Games and DirectInput consumers read
low-level scan codes and ignore virtual keys, so every simulated key carries a hardware scan code
obtained from `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)` together with `KEYEVENTF_SCANCODE`; extended keys
(arrows, Delete, Home, End, ...) also set `KEYEVENTF_EXTENDEDKEY`. Polled game engines only sample
input once per frame, so presses stay down for at least 30–35 ms. Volume and media keys (the overlay
mute, play/pause and next/previous controls) use the same path: `MapVirtualKeyW` scan code,
`KEYEVENTF_EXTENDEDKEY` for the E0-prefixed media range and a 35 ms hold.

Macro Studio records low-level keyboard and mouse hooks (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`) while a
recording is active, groups events into a timeline, and replays them with humanized jitter. The
hook, the polling fallback and the GUI capture path report the same physical transition, so a
lock-free per-key gate admits it once; a recorded tap is stretched to a 35 ms minimum replay hold so
DirectInput games register it, and stopping closes every still-held key and mouse button with a
synthetic release so the timeline never ends with an unmatched press. Relative mouse movement
(`MouseCoordMode::Relative`) is captured through Raw Input and replayed as short delta pulses for
in-game camera turns. Named presets are saved and deleted through the daemon; the modal writes
`library.json` directly only after a failed liveness probe and reports a daemon rejection instead
of retrying silently. The Macro Studio and the Overlay Studio share that one library-write path
(`src/gui/state/library_write.rs`).

## Overlay Studio

Overlays are borderless, GPU-accelerated windows rendered by a dedicated process
(`--overlay-window <id>`), positioned by the daemon's supervisor across monitors with DPI and
orientation awareness. The supervisor is split by owner under `src/daemon/overlay_manager/`
(`supervisor` for active children and calibrated positions, `job` for the kill-on-close Job Object,
`text` for the live-text channel, `monitors` for DPI/logical bounds and `update` for the updater's
notification overlays); `src/daemon/overlay_manager.rs` re-exports the same public surface.
Optional `WDA_EXCLUDEFROMCAPTURE` keeps overlays out of screen capture.
Running overlay children are assigned to a kill-on-close Job Object, so they cannot outlive the
daemon. `SetText` and `syn.overlay_set_text` publish a label to
`%LOCALAPPDATA%\Synector\overlay_text\<slug>-<hash>.txt` by atomic replace; the overlay window polls
that file every 150 ms, so text updates repaint without restarting the process. Positions reported
by `WM_MOVE`/`WM_EXITSIZEMOVE` while the overlay is being dragged are throttled, coalesced into one
pending slot and sent to the daemon by a worker, so the window procedure never blocks on IPC.

The studio itself is a WYSIWYG editor: multi-monitor artboard, snap-to-grid, pan/zoom, element
catalog, undo/redo, and interactive controls (buttons, toggles, sliders) bound to actions, volume,
opacity and live telemetry. The canvas is a non-interactive preview — it paints the real element
widgets but never forwards live pointer input, so an editor click cannot execute an element's
`on_click` rule. ToggleSwitch and ClockTimer state can be exercised locally from the inspector;
overlay click actions can trigger or toggle a rule. Smallest authorable sizes and positions are
enforced: a crosshair `x`/`y` is an absolute offset from the card corner and stays at 1 px or more
(0 or a negative value is the renderer's "centre" sentinel, not an authorable value). Every inspector
property edit — size, colour, label, slider, rotation — is one undo step, and elements are clamped to
the card only when the card or element geometry changes; the clamp never rewrites a Text font size
and overflow is clipped by the card painter. Graph background and InteractiveControl active colour
are element-scoped and edited in the element's own panel.

Calibration is started from the studio and makes the overlay draggable; there is no global
calibration hotkey. Saving a preset always creates a new id: a derived id or typed name that already
exists gets `_2`, `_3`, …, a name without letters or digits is refused, and built-in presets are
never replaced. Preset writes go through the daemon; the GUI writes `library.json` directly only
after a failed liveness probe and reports a daemon rejection instead of retrying silently, on the
same shared write path the Macro Studio uses.

Timers run in the overlay process: a Stopwatch adds, and a Countdown subtracts, the real frame
delta, so a paused or minimized process catches up instead of drifting; Clock and Uptime are
display-only. A countdown reaching zero stops and runs its `on_finish` action once per completed
run, and a reset re-arms it. `ResetTimer` and `ToggleTimer` click actions act on the overlay's timer
elements — a countdown returns to its configured duration and a stopwatch to zero, and toggling a
finished countdown restarts it. Element click actions that need the daemon (trigger/toggle rule,
kill process, trim memory) are queued to a worker instead of running on the render thread. The
update-progress overlay polls the narrow `IpcRequest::GetUpdateProgress` query, which answers with
only the progress fraction and status string, so the frame loop never deserializes the full state
snapshot. A `WindowRelative` overlay caches its target window
handle and re-resolves it when the handle stops being trackable or after two seconds, then converts
the target's physical rectangle with the target monitor's DPI because element geometry is logical
points.

Media widgets show the current title and artist, an equalizer and transport controls (previous,
play/pause, next). A progress bar and a seek control are not rendered: playback position needs a
Windows media-session (GSMTC) integration that does not exist yet, and an always-zero bar with a
dead seek knob would be fake UI.

## Sharing and backup

The `src/sharing/` tree (facade `src/sharing.rs` over the `format`, `payload`, `encode`, `decode`,
`import` and `assets` stages) turns rules into `SYN1:` text codes that can be pasted into chats: the
JSON payload is Brotli-compressed (quality 11, with a versioned custom dictionary), prefixed with an
8-byte SHA-256 checksum and Base64URL-encoded. A code also bundles the macro presets and overlays
its rules reference, so the recipient gets a working rule set. Presets or overlays whose id
already exists locally with different content are re-identified; a content-equal entry is reused
instead of duplicated, so importing the same code twice adds nothing. Rules whose content already
exists are skipped as duplicates, and colliding rule ids get a fresh id. Bundled overlay button and
toggle targets are re-pointed at the final imported rule ids, and a target that neither the code nor
the local rule set provides is reported as a warning.

Sender-local data never travels: runtime counters are reset, and custom icon paths plus overlay
image and button icon paths are dropped with a notice. Truncated or altered codes fail the checksum
instead of decoding to garbage, and the comparison accumulates every differing byte rather than
stopping at the first one. Decoding is bounded (4 MiB of code, 8 MiB of decompressed payload, 500
rules, 500 overlays and 500 macro presets), so a hostile code cannot exhaust memory. Exporting is
one click: the code is compressed on a worker thread and copied straight to the clipboard through
the shared `src/clipboard.rs` helper. Importing is one daemon-owned `IpcRequest::ImportShare` step
that applies rules plus library entries, skips duplicates and creates the rules disabled by default.
The toast reports how many rules, library entries and duplicates were handled, while advisory
notices (scripts, elevated processes, webhooks, dropped icons and overlay images, unresolved overlay
targets, missing presets/overlays) are written to the log. JSON files remain the backup format:
`--rules-export` and the CLI authoring commands cover file-based migration.

## Installation forms and updates

Synector ships in two forms. The portable zip is `Synector.exe` plus `LICENSE` and `NOTICES.md`; the
installer (`installer/synector.iss`, built only in CI) installs the same executable into `Program
Files`, adds Start Menu and `App Paths` integration and registers an uninstaller. A console bootstrap
(`installer/install.cmd`, or `install.ps1` fetched with `irm <url> | iex`) covers machine-driven
installs: it resolves the latest release, verifies the chosen artifact's SHA-256 and runs Setup,
extracts the portable archive or calls the uninstaller.

The daemon owns update work: it checks GitHub releases, streams the download and verifies it against
the SHA-256 digest published by the GitHub API. An asset without a published digest is refused and
the download deleted, so an update is never applied unverified. Every release also publishes
`SHA256SUMS.txt` alongside the artifacts for manual verification; the updater itself uses the API
digest. A portable copy replaces its own executable in place; an installed copy downloads the
release's setup and starts it silently with `/VERYSILENT /UpdateMode=gui|daemon`, letting the
installer replace files, refresh the uninstaller metadata and restart Synector. With process masking
enabled the portable daemon runs from the `WindowsServiceHost.exe` cache, so the portable updater
targets the real executable instead of that cache. The GUI reports its own executable path to the
daemon it spawns (`--gui-exe <path>`), so the target stays known after the GUI window is closed
(tray-only updates); the executable of a running GUI window is the fallback, and only when neither
is available does the update fail closed with an error. A failed in-place swap restores or keeps the
previous executable and reports the failure. The GUI prepares the masked copy before it spawns the
daemon, and every daemon start with masking enabled verifies or refreshes that copy; when
preparation fails the daemon runs unmasked with the fallback logged, so a degraded masked deployment
is never silent.
When masking has been disabled, a daemon start that is not itself the masked copy removes the stale
image; if the running daemon is that copy, the skip is logged and the next start from the real
executable cleans it up. The restart follows the update origin: a background update keeps the
daemon-only or GUI state that was active before. Progress and status surface through the tray menu
and overlay notifications, and optional auto-install is a setting.

The PowerShell bootstrap that extracts the portable archive rejects entries with rooted paths or
`..` segments before resolving the destination, so a crafted archive cannot write outside the chosen
folder. The installer closes running instances and, for updates and interactive installs, removes the
stale masked daemon copy so it is rebuilt from the new binary. The masked autostart entry is the
per-user task `Synector_DaemonMasked`; a legacy `WindowsServiceHost` task is migrated only after its
definition proves it belongs to Synector. On uninstall the installer removes the autostart tasks,
the masked copy and the application files while keeping the user configuration directory.

## Rules: organization and editing

Rules carry custom colored tags, tag filters, and multi-mode sorting (manual/default, name ascending
or descending, trigger type, action count, status). In default sort order with no tag filter and no
active search, cards are reordered by drag handle and the new order is committed immediately and
persisted through IPC; filtered, searched or sorted modes disable reordering with an explanatory
tooltip.

The GUI keeps an undo/redo draft per edited rule and per overlay definition (up to 50 snapshots),
through one bounded-history implementation shared by the two drafts (`src/gui/state/undo.rs`), so
editing, testing and saving a rule never mutates daemon-owned counters before the user commits. An
edit gesture is one undo step — a drag or a slider commits the pre-gesture state once instead of one
snapshot per frame. Opening a rule only renders its stored values: the inspector writes no defaults
into the draft outside real widget interaction, so an unset field stays unset and is reported by
validation on save instead of being silently patched. Save runs the same structural validator as the
CLI before it sends anything; the errors are listed in the inspector and the editor stays open, and
the daemon validates again when it applies the mutation.

## Logging and localization

Every user-visible string lives in the `Strings` catalog (`src/i18n/strings.rs`, `src/i18n/en.rs`,
`src/i18n/ru.rs`). The in-app guide is the deliberate exception: its long prose is written as
Russian/English `if lang == Language::Russian { ... } else { ... }` branches inside
`src/gui/views/guide_modal.rs`, while its chrome labels come from `Strings`. Both language branches
must be updated together.

Diagnostics go through `crate::logging` into daily files under `%LOCALAPPDATA%\Synector\logs`; the
level comes from `SYNECTOR_LOG` (`off|error|warn|info|debug|trace`, default `info`). File logging can
also be turned off with the `log_to_disk` setting: every process reads it at start-up, the daemon
applies a runtime change when it saves the section, and the GUI applies it on the toggle and then
follows the daemon's snapshot, so all tiers stop writing together. Disabling only stops new events —
the files already on disk, including the retention window, are left untouched and logging resumes on
the same day's file when it is turned back on; `SYNECTOR_LOG=off` remains an independent override.
Log identifiers and outcomes only — never script bodies, webhook payloads, rule JSON or other user
secrets. Action errors that may embed script output are routed through the engine's `SensitiveLog`
event. The logger primes today's file at start-up, so an unwritable logs directory is reported once
on stderr and returned from `init()`; the process entry prints that returned failure as an explicit
diagnostic because a release build has no console, and the open is not retried until the next day.
Every append and compaction is serialized across processes with a named mutex, so the daemon, the
GUI and a CLI command cannot interleave a write.

## Design conventions

- Localization is mandatory for regular views; the guide's inline bilingual prose is the documented
  exception.
- Documentation has one owner per topic; the document set and its owners are listed in one place,
  [docs/README.md](README.md). Link instead of copying text across files.
- `README.md` and `README.ru.md` are mirrors and must change together.
