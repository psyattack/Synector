# FAQ and troubleshooting

Answers to the questions that come up most often. The JSON rule format is documented in
[RULE_AUTHORING.md](RULE_AUTHORING.md), complete rules are in [EXAMPLES.md](EXAMPLES.md), and
internals are in [ARCHITECTURE.md](ARCHITECTURE.md).

<details>
<summary>Contents</summary>

- [Installation and updates](#installation-and-updates)
  - [Which build should I use: Setup or portable?](#which-build-should-i-use-setup-or-portable)
  - [Windows SmartScreen or antivirus warns about the download](#windows-smartscreen-or-antivirus-warns-about-the-download)
  - [Do I need to run as administrator?](#do-i-need-to-run-as-administrator)
  - [Does Synector start with Windows?](#does-synector-start-with-windows)
  - [How do updates work?](#how-do-updates-work)
  - [Can I downgrade?](#can-i-downgrade)
- [Daemon and GUI](#daemon-and-gui)
  - [Does closing the window stop my rules?](#does-closing-the-window-stop-my-rules)
  - [Where are configuration and logs?](#where-are-configuration-and-logs)
  - [How do I back up or move my setup?](#how-do-i-back-up-or-move-my-setup)
  - [A configuration file is corrupt or I want a clean start](#a-configuration-file-is-corrupt-or-i-want-a-clean-start)
  - [Why are there two Synector processes?](#why-are-there-two-synector-processes)
  - [A save failed — is my edit lost?](#a-save-failed--is-my-edit-lost)
  - [How do I create or change an overlay?](#how-do-i-create-or-change-an-overlay)
  - [How do I record and replay a macro?](#how-do-i-record-and-replay-a-macro)
- [Interface](#interface)
  - [Where do I find a setting?](#where-do-i-find-a-setting)
  - [How do I search?](#how-do-i-search)
  - [What does the analytics dialog show?](#what-does-the-analytics-dialog-show)
  - [How do I change the appearance?](#how-do-i-change-the-appearance)
- [Rules and triggers](#rules-and-triggers)
  - [My rule does not fire](#my-rule-does-not-fire)
  - [Why does a manual step test run for a long time?](#why-does-a-manual-step-test-run-for-a-long-time)
  - [What is the difference between a cooldown and a quota?](#what-is-the-difference-between-a-cooldown-and-a-quota)
  - [Why is my infinite macro rejected?](#why-is-my-infinite-macro-rejected)
  - [How do I trigger Synector from another program?](#how-do-i-trigger-synector-from-another-program)
  - [Why does a hotkey not respond?](#why-does-a-hotkey-not-respond)
  - [Are share codes safe?](#are-share-codes-safe)
  - [PowerShell does not show CLI output](#powershell-does-not-show-cli-output)
- [Troubleshooting](#troubleshooting)
  - [The daemon does not start or exits immediately](#the-daemon-does-not-start-or-exits-immediately)
  - [An overlay is not visible](#an-overlay-is-not-visible)
  - [A macro does not register in a game](#a-macro-does-not-register-in-a-game)
  - [Audio actions fail](#audio-actions-fail)
  - [High CPU or constant disk writes](#high-cpu-or-constant-disk-writes)
  - [The webhook port is busy](#the-webhook-port-is-busy)
  - [Where do I report a bug or a security issue?](#where-do-i-report-a-bug-or-a-security-issue)

</details>

## Installation and updates

### Which build should I use: Setup or portable?

- **Setup** (`Synector-Setup-<version>-Windows-x64.exe`) installs into `Program Files`, adds a Start
  Menu shortcut, registers `App Paths` and an uninstaller; updates run the new setup silently.
- **Portable** (`Synector-v<version>-Windows-x64.zip`) contains `Synector.exe` plus the license and
  attribution files. Extract it anywhere and run the executable; updates replace it in place.

Both forms share the same configuration in `%LOCALAPPDATA%\Synector\` and the same daemon.

### Windows SmartScreen or antivirus warns about the download

Synector is distributed through GitHub Releases. Verify the file before running it:

```powershell
Get-FileHash .\Synector-Setup-<version>-Windows-x64.exe -Algorithm SHA256
```

Compare the result with the matching line in `SHA256SUMS.txt` from the same release. If the hashes
match and you downloaded from the official release page, the warning is a reputation prompt rather
than a sign of tampering. Do not disable antivirus protection or add exclusions for files whose hash
you have not verified.

### Do I need to run as administrator?

Synector starts elevated by default; `--no-elevate` opts out. Elevation is needed for:

- kernel-level process tracing (without it, process detection falls back to snapshot polling);
- input simulation into elevated applications;
- memory, priority and affinity tuning of elevated processes;
- silent autostart through Task Scheduler.

Rules that do not touch an elevated target work without elevation.

### Does Synector start with Windows?

The daemon registers a Task Scheduler autostart entry on first launch; it can be disabled in
**Settings** and the uninstaller removes it. The first launch also creates the configuration
directory and picks the interface language from Windows: a Russian Windows UI opens Synector in
Russian, every other UI language in English. The choice is stored in `settings.json` and is never
re-detected — change it in **Settings → Appearance**.

### How do updates work?

The daemon checks GitHub Releases, downloads the right asset and verifies the SHA-256 digest published
by the GitHub API; an asset without a published digest is refused and its download deleted. An
installed copy runs the new setup silently, a portable copy replaces its own executable, and progress
shows in the tray. Auto-install is optional and can be turned off in Settings.

If an update fails, check the log in `%LOCALAPPDATA%\Synector\logs\`, make sure the app is not
running from a read-only or protected folder, and install the latest release manually. The
configuration directory is preserved either way.

### Can I downgrade?

Install the older release over the current one. Configuration is versioned per section and
downgrading is not a supported migration path, so keep a copy of `%LOCALAPPDATA%\Synector\` and an
export of your rules (`--rules-export rules.json`) before changing versions.

## Daemon and GUI

### Does closing the window stop my rules?

No. The daemon is the tier that owns rules and triggers; the GUI is only a client. Closing the GUI,
or never opening it, leaves automation running. Use the tray menu to pause, open the window or exit
the daemon.

### Where are configuration and logs?

- Configuration: `%LOCALAPPDATA%\Synector\` (`settings.json`, `rules.json`, `library.json`,
  `runtime.json`). Set `SYNECTOR_CONFIG_DIR` to relocate the whole directory.
- Logs: `%LOCALAPPDATA%\Synector\logs\`, one file per day, 7-day retention and an 8 MiB cap per
  file.

The log level comes from `SYNECTOR_LOG` (`off`, `error`, `warn`, `info`, `debug`, `trace`; default
`info`) and is read at start-up. **Settings → General** (*Write Logs to Disk*) stops new events from
being written without deleting the files already on disk; the environment variable stays an
independent override.

### How do I back up or move my setup?

Stop Synector first, then copy the whole `%LOCALAPPDATA%\Synector\` directory. For rules alone,
`Synector.exe --rules-export rules.json` or a `SYN1:` share code is enough. While the daemon is
running it is the only writer, so editing the files underneath it is not supported.

### A configuration file is corrupt or I want a clean start

A section that fails to parse is renamed to `<name>.corrupt-<timestamp>` before defaults replace it,
so the original is not destroyed. Rules are checked one by one: a rule with an unknown key or invalid
values is skipped and the log names its id and the problem, while the rest of `rules.json` still
loads. To start clean, stop Synector and remove or rename the section files; to recover, restore a
backup over them while the daemon is stopped.

### Why are there two Synector processes?

The tray daemon and the GUI are separate tiers of the same executable, and each overlay window is
another borderless process supervised by the daemon. Seeing several Synector processes is normal when
overlays are open.

### A save failed — is my edit lost?

No. Saving a rule validates the draft first: structural errors are listed in the inspector and
nothing is sent, so an invalid rule never replaces the stored one. A rejected or failed save keeps
the edit open with its draft intact and names the error in a toast. While the daemon is confirmed
unreachable the GUI writes the affected config section directly, so the edit can still be saved
offline.

### How do I create or change an overlay?

Overlays are edited in the Overlay Studio: select an Overlay action in the rule inspector and press
**OPEN →**. The studio arranges elements on a multi-monitor artboard, and **POSITION ON SCREEN**
switches the overlay into calibration, where it can be dragged to the intended display. The canvas is
a preview, not a live overlay, so clicking an element never runs its click action; ToggleSwitch and
ClockTimer state can still be exercised from the inspector. The daemon renders the overlay in a
separate process, so it stays on screen when the GUI is closed.

The action decides what happens at runtime: toggle visibility or flash the overlay, optional override
text (tokens such as `{ProcessName}` are allowed), duration and fade, and a lifetime that keeps the
overlay until the triggering application exits (`until_trigger_process_exit`) or hides it when the
rule run ends (`until_run_end`). Definitions live in `library.json`.

### How do I record and replay a macro?

Record it in Macro Studio: select a Macro action in the inspector and press **OPEN →**, then **F9**
starts and stops recording. **F10** test-runs the sequence, **Esc** stops recording and closes the
studio. The action then decides playback: operation Run, Stop or Toggle, repeat count and an optional
preset stored in `library.json`. Playback uses hardware scan codes with short holds and humanized
timing, so it also registers in DirectInput games.

## Interface

### Where do I find a setting?

Settings is a dialog opened from the gear glyph in the window header: one scrollable column of
untitled blocks — general, appearance and update settings — with its own search field across the top.
The field keeps only the rows whose title or description matches the query, and a query that names a
block keeps that whole block. Every switch and picker saves as you change it: there is no Save button
and no restart unless a row says otherwise.

### How do I search?

The rules list is the only page, and the header holds one search field. The chart and gear glyphs to
its right open the analytics and settings dialogs. Typing opens a dimmed results panel grouped by
surface — rules, settings, the activity log, actions, triggers, conditions, overlay presets and macro
presets. Choosing the rules group keeps the filtered list and the query in the field; choosing any
other group opens that surface with the query already typed into its own search field. Escape closes
the panel first and clears the field on the next press.

### What does the analytics dialog show?

It is one scrolling page: the activity log with its level/category and text filters, then the
metrics — the sum of the per-rule execution counters, session executions, what is running now and the
24-hour success rate; a live 60-second CPU/RAM chart with the daemon's own CPU, memory and disk use;
executions grouped by source; a 24-hour activity chart with failures marked; the most active rules;
and enabled rules that need attention. History is kept in the daemon's memory for the last 24 hours
and restarts with it.

### How do I change the appearance?

**Settings → Appearance** holds three independent controls — palette, skin generation and accent —
plus a switch that lets Windows pick the palette. The palette swatches switch between the **Dark** and
**Light** themes; *Follow Windows Theme* makes the palette track the Windows light/dark setting
instead, without a restart. Picking a swatch by hand turns the switch off and freezes that palette as
your choice. The **skin tiles** choose the interface generation — the modern look or the original
studio one — while the palette stays whatever was picked. **Accent colour** offers presets and a
custom colour picker for selection, focus rings and active controls; **Reset** clears the custom
accent back to the palette's own. All choices apply immediately.

## Rules and triggers

### My rule does not fire

Check, in order:

1. The rule is enabled and has not hit a lifetime quota.
2. The trigger target actually matches (process name and its selected start/exit event, window,
   folder, hotkey, webhook path).
3. Preconditions are satisfied. A false condition prevents the pipeline from starting.
4. `cooldown_seconds` or a periodic quota is not skipping the run. `--rules-list` shows the counter
   and period.
5. The rule is not stuck in an active execution, and `stop_on_error` did not halt it after a failed
   step.
6. The daemon logs in `%LOCALAPPDATA%\Synector\logs\` do not show a permission or target-not-found
   error.

Process detection uses kernel-level tracing when elevated and adaptive snapshot polling otherwise;
both feed the same trigger surface, so a missing tracing session alone does not disable process
rules. A process trigger fires the rule's one pipeline on the event selected in the rule builder — On
Process Start (the default) or On Process Exit — and both go through the same preconditions, cooldown
and quota.

### Why does a manual step test run for a long time?

Testing a step executes only that action, but it executes it for real: a script that copies files runs
until it finishes, a wait step waits for its configured time. The test is bounded by the action's own
ceiling plus a short slack, so a long script is never reported as a lost daemon. While it runs, the
step card keeps the executing focus, the rest of the window stays connected, and the Play button
becomes Stop; stopping aborts the action like a Test Run and terminates the whole process tree the
script started. The editor paints no result badge — the outcome is in the daemon log.

### What is the difference between a cooldown and a quota?

- **Cooldown** is a minimum delay between two triggered runs.
- **Lifetime quota** (`max_executions` without `quota_period`) pauses the rule after the limit.
  `--rules-reset` clears the counter and re-enables it.
- **Periodic quota** (`quota_period`) keeps the rule enabled and skips executions until the period
  rolls over.

See [RULE_AUTHORING.md#quotas](RULE_AUTHORING.md#quotas) for reset semantics and JSON fields.

### Why is my infinite macro rejected?

An infinite macro has no natural end. The CLI validator only accepts it with a global-hotkey trigger,
because every other trigger offers no manual cancellation path. The Rule Builder warns and dims the
steps after the infinite action.

### How do I trigger Synector from another program?

Use a webhook trigger and send a request to
`http://127.0.0.1:48123/api/v1/trigger/<endpoint_path>` (a rule id also works). The server is
local-only. See [RULE_AUTHORING.md#webhook-trigger](RULE_AUTHORING.md#webhook-trigger) for the
request and response contract, [RULE_AUTHORING.md](RULE_AUTHORING.md) for the JSON format and
[EXAMPLES.md](EXAMPLES.md) for a complete rule.

### Why does a hotkey not respond?

Synector watches the keyboard directly instead of registering a system-wide hotkey, which is what
allows mouse-button combinations and hold/repeat modes. Another application listening for the same
combination will also react, and a target that ignores synthetic input will still not respond. If a
combination does nothing, try another one and check the logs. Hold mode repeats the pipeline while
the key is held; repeat-while-held without hold mode loops until the hotkey is pressed again.

### Are share codes safe?

`SYN1:` codes carry a checksum and are decoded with bounds on code length and payload size. Imported
rules are created disabled by default; review them before enabling. A share code can contain scripts,
webhook actions and other powerful steps, so only import codes from people you trust. See
[ARCHITECTURE.md#sharing-and-backup](ARCHITECTURE.md#sharing-and-backup).

### PowerShell does not show CLI output

Release builds use the Windows GUI subsystem, so PowerShell neither waits for them nor captures their
output or exit code. Prefix commands with `cmd /c`:

```powershell
cmd /c "Synector.exe --rules-validate rule.json"
```

The full explanation, including how other tools that spawn and wait for the process behave, is in
[RULE_AUTHORING.md#console-notes](RULE_AUTHORING.md#console-notes).

## Troubleshooting

### The daemon does not start or exits immediately

Another Synector instance may hold the single-instance mutex; check Task Manager for an existing
daemon or GUI. Look for an update-file cleanup failure or a permission error in the log. Starting
from a folder that requires higher privileges than the process has can also prevent self-update or
overlay processes from starting.

### An overlay is not visible

Open the Overlay Studio and start calibration: the overlay becomes draggable and can be moved to the
intended monitor. Check multi-monitor placement and DPI scaling in the overlay definition. The daemon
must be running for overlays to exist. Anti-capture protection keeps an overlay out of screen
capture; it does not hide it on screen.

### A macro does not register in a game

Synector replays hardware scan codes and holds presses for at least 30–35 ms, which covers most
DirectInput games, but some titles and anti-cheat systems ignore or flag synthetic input. Run
Synector elevated if the game is elevated, because Windows otherwise blocks input into a
higher-privilege process. Camera turns should be recorded in the same coordinate mode the game
expects. Use automation only where the application permits it.

### Audio actions fail

Check that the requested default device exists and is enabled, and that the target process is playing
audio. `default_device` uses the AudioDeviceCmdlets PowerShell module; if it is not installed the
action reports an error instead of doing nothing. `audio_format.device_name` must match an active
endpoint's display name. Per-application volume and channel balance depend on the Windows audio
session for that process; a stopped or suspended app may not expose one. Audio format changes can
temporarily interrupt playback by design.

### High CPU or constant disk writes

Polling is adaptive and backs off when no rule uses a trigger, but many independent polling triggers,
a fast interval schedule or `trace` logging increase load. Runtime counters and overlay positions are
written coalesced, not on every event.

### The webhook port is busy

The local webhook server binds `127.0.0.1:48123`, and the port is fixed. It only runs while at least
one enabled rule uses the webhook trigger, so a busy port cannot affect a daemon whose rules never
use it. While a webhook rule is enabled and another application owns the port, the bind error is
logged and the server retries with a capped backoff, so webhook-triggered rules resume once the port
is freed. Close the conflicting application, or stop the rule source until the port is available.

### Where do I report a bug or a security issue?

- Bugs and feature requests: [GitHub Issues](https://github.com/psyattack/Synector/issues/new/choose).
  Include the Synector version, `winver`, whether the app was elevated, reproduction steps and the
  relevant log lines with secrets removed.
- Security problems: follow [SECURITY.md](../SECURITY.md) and report privately. Do not open a public
  issue.
