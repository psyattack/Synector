# FAQ and troubleshooting

Answers to the questions that come up most often. For the JSON rule format see
[RULE_AUTHORING.md](RULE_AUTHORING.md), for complete rules see [EXAMPLES.md](EXAMPLES.md), and for
internals see [ARCHITECTURE.md](ARCHITECTURE.md).

<details>
<summary>Contents</summary>

- [Installation and updates](#installation-and-updates)
  - [Which build should I use: Setup or portable?](#which-build-should-i-use-setup-or-portable)
  - [Windows SmartScreen or antivirus warns about the download](#windows-smartscreen-or-antivirus-warns-about-the-download)
  - [Do I need to run as administrator?](#do-i-need-to-run-as-administrator)
  - [Does Synector start with Windows?](#does-synector-start-with-windows)
  - [How do updates work?](#how-do-updates-work)
  - [Can I downgrade?](#can-i-downgrade)
  - [Upgrading from 2.8 or older](#upgrading-from-28-or-older)
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
  - [How do I change the appearance?](#how-do-i-change-the-appearance)
- [Rules and triggers](#rules-and-triggers)
  - [My rule does not fire](#my-rule-does-not-fire)
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
  Menu shortcut, registers `App Paths` and an uninstaller. Updates are applied by downloading and
  running the new setup silently.
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
than a sign of tampering. Do not disable antivirus protection or add exclusions for files whose
hash you have not verified.

### Do I need to run as administrator?

Synector starts elevated by default; `--no-elevate` opts out. Elevation enables:

- kernel ETW process tracing (otherwise Synector falls back to adaptive Toolhelp32 polling);
- input simulation into elevated applications (UIPI bypass);
- memory/priority/affinity tuning of elevated processes;
- silent highest-privilege logon autostart through Task Scheduler.

Without elevation most rules still work, but a rule that targets an elevated game or application,
or that relies on ETW, may not fire.

### Does Synector start with Windows?

The daemon registers a Task Scheduler autostart entry on first launch. It can be disabled in
Settings. The entry is separate from the installer: uninstalling removes it. First launch always
creates `%LOCALAPPDATA%\Synector\` with the configuration files and logs.

### How do updates work?

The daemon checks GitHub Releases, selects the right asset, downloads it and verifies the SHA-256
digest published by the GitHub API. An asset without a published digest is refused and its download
deleted, so an update is never applied unverified. An installed copy downloads the release's setup
and starts it silently; a portable copy replaces its own executable. With process masking enabled the
portable daemon runs from the masked copy, so the update targets the real executable: the GUI reports
its path to the daemon when it starts it, which keeps the target known after that window is closed,
and a running GUI window is the fallback. Only when neither source is available does the update fail
with an error. If the executable swap fails, the previous executable is kept or restored and the
update reports an error instead of claiming success. Progress is shown in the tray and overlay
notifications. Auto-install is optional and can be turned off in Settings.

If an update fails, check the log in `%LOCALAPPDATA%\Synector\logs\`, make sure the app is not
running from a read-only or protected folder, and install the latest release manually. The
configuration directory is preserved either way.

### Can I downgrade?

Install the older release over the current one. Configuration is versioned per section and
downgrades are not a supported migration path: a 2.8 build cannot read the 2.9 files. Before
changing versions, keep a copy of `%LOCALAPPDATA%\Synector\` and an export of your rules
(`--rules-export rules.json`), so the rules can be restored or re-entered.

### Upgrading from 2.8 or older

The 2.9 storage format is split into independent section files and older configuration is not
migrated automatically. A 2.8 installation has no `--rules-export` flag and its `PH1:` share codes
are rejected by 2.9, so rules have to be re-created in the new version: keep the old app installed
alongside while you copy rule settings across. `SYN1:` share codes exist only in 2.9 and later.

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
`info`). Restart Synector after changing it.

### How do I back up or move my setup?

Stop Synector first, then copy the whole `%LOCALAPPDATA%\Synector\` directory. For rules alone,
`Synector.exe --rules-export rules.json` or a `SYN1:` share code is enough. While the daemon is
running it is the only writer; editing the files underneath it is not supported.

### A configuration file is corrupt or I want a clean start

A section that fails to parse is renamed to `<name>.corrupt-<timestamp>` before defaults replace
it, so the original is not destroyed; a taken name gets a `-1`, `-2`, … suffix, so an earlier
quarantine copy is never overwritten. If the file cannot be moved aside, Synector refuses to
overwrite that section until you move it by hand, and the log names the blocking file. Rules are
checked one by one: a rule that contains an unknown key — including a field removed from the format
— or whose values fail validation is skipped, and the log names the rule id with the keys or
errors, while the rest of `rules.json` still loads. To start clean, stop Synector and remove or
rename the section files; to recover, restore a backup over them while the daemon is stopped.

### Why are there two Synector processes?

The tray daemon and the GUI are separate tiers of the same executable. Overlay windows are
additional borderless processes supervised by the daemon. Seeing several Synector processes is
normal when overlays are open. Each tier has a single-instance mutex, so a second daemon or GUI does
not start.

### A save failed — is my edit lost?

No. Saving a rule validates the draft first: structural errors are listed in the inspector and
nothing is sent, so an invalid rule never replaces the stored one. A rule or overlay edit also stays
open with its draft intact when the daemon rejects or the request fails, and a toast names the
error. When the daemon is confirmed unreachable, the GUI writes the affected config section directly
after a failed liveness probe, so the edit can still be saved offline. A sustained outage is logged
and shown as an offline toast. Settings edits keep the same guarantee: if neither the daemon nor the
direct write accepts the change, the edited form stays and the failure is reported instead of a
success toast.

### How do I create or change an overlay?

Overlays are edited in the Overlay Studio: select an Overlay action in the rule inspector and press
**OPEN →**. The studio arranges elements on a multi-monitor artboard; **POSITION ON SCREEN** switches
the overlay into calibration, which makes it draggable so it can be placed on the intended display.
The canvas is a preview, not a live overlay: clicking an element in the editor never runs its click
action. ToggleSwitch and ClockTimer state can still be exercised from the inspector, every property
edit is undoable, and saving a preset under an existing name creates a new `_2`/`_3` copy instead of
replacing the built-in one. The daemon then renders the overlay in a separate process, so it stays on
screen when the GUI is closed.

The action decides what happens at runtime: toggle visibility or flash on screen, optional override
text (tokens such as `{ProcessName}` are allowed), duration and fade, and "close when process exits"
to bind the overlay to the triggering application. Saved definitions live in `library.json`.

### How do I record and replay a macro?

Record it in Macro Studio: select a Macro action in the inspector and press **OPEN →**, then **F9**
starts and stops recording — the low-level hooks are active only while recording. **F10** test-runs
the sequence on a background thread, **Esc** stops recording, cancels an active drag-and-drop and
closes the studio. The action then decides playback: operation Run, Stop or Toggle, repeat count, and
an optional preset stored in `library.json`. Playback uses hardware scan codes with short holds and
humanized timing, so it also registers in DirectInput games.

## Interface

### How do I change the appearance?

**Settings → Appearance** holds three independent controls. The palette swatches switch between the
**Dark** and **Light** themes — only colours change. The **skin tiles** choose the interface
generation: the modern look (rounded, layered surfaces, one sliding pill for the active tab) or the
original studio one (hard edges, hairline rules, square chrome); the palette stays whatever you
picked. **Accent colour** offers a few presets and a custom colour picker for selection, focus rings
and active controls; **Reset** clears the custom accent back to the palette's own. All three choices
are stored in `settings.json` as `theme`, `ui_skin` and `accent` (omitted while no custom accent is
set) and apply immediately, without a restart.

## Rules and triggers

### My rule does not fire

Check, in order:

1. The rule is enabled and has not hit a lifetime quota.
2. The trigger target actually matches (process name, window, folder, hotkey, webhook path).
3. Preconditions are satisfied. A false condition prevents the pipeline from starting.
4. `cooldown_seconds` or a periodic quota is not skipping the run. `--rules-list` shows the counter
   and period.
5. The rule is not stuck in an active execution, and `stop_on_error` did not halt it after a failed
   step.
6. The daemon logs in `%LOCALAPPDATA%\Synector\logs\` do not show a permission or target-not-found
   error.

Process detection uses ETW when elevated and adaptive Toolhelp32 polling otherwise; both feed the
same trigger surface, so a missing ETW session alone does not disable process rules.

### What is the difference between a cooldown and a quota?

- **Cooldown** is a minimum delay between two triggered runs.
- **Lifetime quota** (`max_executions` without `quota_period`) pauses the rule after the limit.
  `--rules-reset` clears the counter and re-enables it.
- **Periodic quota** (`quota_period`) keeps the rule enabled and skips executions until the period
  rolls over.

See [RULE_AUTHORING.md#quotas](RULE_AUTHORING.md#quotas) for reset semantics and JSON fields.

### Why is my infinite macro rejected?

An infinite macro has no natural end. The CLI validator only accepts it with a global-hotkey
trigger, because every other trigger offers no manual cancellation path. The Rule Builder warns and
dims the steps after the infinite action.

### How do I trigger Synector from another program?

Use a webhook trigger and send a request to
`http://127.0.0.1:48123/api/v1/trigger/<endpoint_path>` (a rule id also works). The server is
local-only. See [RULE_AUTHORING.md#webhook-trigger](RULE_AUTHORING.md#webhook-trigger) for the
request and response contract, [RULE_AUTHORING.md](RULE_AUTHORING.md) for the JSON format and
[EXAMPLES.md](EXAMPLES.md) for a complete rule.

### Why does a hotkey not respond?

Global hotkeys are detected by polling `GetAsyncKeyState`, not by `RegisterHotKey`, which is what
allows mouse-button combinations and hold/repeat modes. Another application listening for the same
combination will also react, and a target that ignores synthetic input will still not respond. If a
combination does nothing, try another one and check the logs. Hold mode repeats the pipeline while
the key is held; repeat-while-held without hold mode loops until the hotkey is pressed again.

### Are share codes safe?

`SYN1:` codes carry a checksum and are decoded with bounds on code length and decompressed size,
and at most 500 rules, 500 overlays and 500 macro presets per code. Imported rules are created
disabled by default; review them before enabling. A share code can contain scripts, webhook actions
and other powerful steps, so only import codes from people you trust. See
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

Open the Overlay Studio and start calibration: the overlay becomes draggable and can be moved to
the intended monitor. Check multi-monitor placement and DPI scaling in the overlay definition. The
daemon must be running for overlays to exist. `WDA_EXCLUDEFROMCAPTURE` keeps an overlay out of
screen capture; it does not hide it on screen.

### A macro does not register in a game

Synector replays hardware scan codes and holds presses for at least 30–35 ms, which covers most
DirectInput games, but some titles and anti-cheat systems ignore or flag synthetic input. Run
Synector elevated if the game is elevated (UIPI otherwise blocks input). Relative camera turns use
Raw Input-compatible deltas and should be recorded in the same coordinate mode the game expects.
Use automation only where the application permits it.

### Audio actions fail

Check that the requested default device exists and is enabled, and that the target process is
playing audio. `default_device` uses the AudioDeviceCmdlets PowerShell module; if it is not
installed the action reports an error instead of doing nothing. `audio_format.device_name` must
match an active endpoint's display name — a name that matches nothing fails the action.
Per-application volume and channel balance depend on the Windows audio session for that process; a
stopped or suspended app may not expose one. Audio format changes can temporarily interrupt playback
by design.

### High CPU or constant disk writes

Polling is adaptive and backs off when no rule uses a trigger, but many independent polling
triggers, a fast interval schedule or `trace` logging increase load. Process triggers use ETW when
elevated; without elevation they fall back to snapshot polling. Runtime counters and overlay
positions are written coalesced, not on every event.

### The webhook port is busy

The local webhook server binds `127.0.0.1:48123`, and the port is fixed. It only runs while at
least one enabled rule uses the webhook trigger, so a busy port cannot affect a daemon whose rules
never use it. While a webhook rule is enabled and another application already owns the port, the
bind error is logged and the server retries with a capped backoff (1 s doubling to 30 s), so
webhook-triggered rules resume once the port is freed. Close the conflicting application, or stop
the rule source until the port is available.

### Where do I report a bug or a security issue?

- Bugs and feature requests: [GitHub Issues](https://github.com/psyattack/Synector/issues/new/choose).
  Include the Synector version, `winver`, whether the app was elevated, reproduction steps and the
  relevant log lines with secrets removed.
- Security problems: follow [SECURITY.md](../SECURITY.md) and report privately. Do not open a
  public issue.
