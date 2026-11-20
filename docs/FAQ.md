# FAQ and troubleshooting

Answers to the questions that come up most often. For the JSON rule format see
[RULE_AUTHORING.md](RULE_AUTHORING.md), for complete rules see [EXAMPLES.md](EXAMPLES.md), and for
internals see [ARCHITECTURE.md](ARCHITECTURE.md).

## Installation and updates

### Which build should I use: Setup or portable?

- **Setup** (`Synector-Setup-<version>-Windows-x64.exe`) installs into `Program Files`, adds a Start
  Menu shortcut, registers `App Paths` and an uninstaller. Updates are applied by downloading and
  running the new setup silently.
- **Portable** (`Synector-v<version>-Windows-x64.zip`) is a single `Synector.exe`. Extract it
  anywhere and run it; updates replace the executable in place.

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
- input simulation into elevated applications through UIPI;
- memory/priority/affinity tuning of elevated processes;
- silent highest-privilege logon autostart through Task Scheduler.

Without elevation most rules still work, but a rule that targets an elevated game or application,
or that relies on ETW, may not fire.

### Does Synector start with Windows?

The daemon registers a Task Scheduler autostart entry on first launch. It can be disabled in
Settings. The entry is separate from the installer: uninstalling removes it, and running portable
without autostart leaves the machine untouched.

### How do updates work?

The daemon checks GitHub Releases, selects the right asset, downloads it and verifies the SHA-256
digest published by the GitHub API. An installed copy downloads the release's setup and starts it
silently; a portable copy replaces its own executable. Progress is shown in the tray and overlay
notifications. Auto-install is optional and can be turned off in Settings.

If an update fails, check the log in `%LOCALAPPDATA%\Synector\logs\`, make sure the app is not
running from a read-only or protected folder, and install the latest release manually. The
configuration directory is preserved either way.

### Can I downgrade?

Install the older release over the current one. Configuration is versioned per section, but
downgrades are not a supported migration path: keep an export of your rules
(`--rules-export rules.json`) or a `SYN1:` share code before changing versions, and restore from it
if the older build cannot read the current data.

### Upgrading from 2.8 or older

The 2.9 storage format is split into independent section files and older configuration is not
migrated automatically. Export the rules from the old version (`--rules-export` or a share code)
and import them into the new one. Old `PH1:` share codes are not accepted by 2.9 and later;
re-export them from the updated app.

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
it, so the original is not destroyed. To start clean, stop Synector and remove or rename the
section files; to recover, restore a backup over them while the daemon is stopped.

### Why are there two Synector processes?

The tray daemon and the GUI are separate tiers of the same executable. Overlay windows are
additional borderless processes supervised by the daemon. Seeing several Synector processes is
normal when overlays are open. Each tier has a single-instance mutex, so a second daemon or GUI does
not start.

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
local-only. See [RULE_AUTHORING.md](RULE_AUTHORING.md) for the JSON format and
[EXAMPLES.md](EXAMPLES.md) for a complete rule.

### Why does a hotkey not respond?

Global hotkeys are detected by polling `GetAsyncKeyState`, not by `RegisterHotKey`, which is what
allows mouse-button combinations and hold/repeat modes. A combination already consumed by another
application or by Windows may still not reach Synector. Try a different combination and check the
logs. Hold mode repeats the pipeline while the key is held; repeat-while-held without hold mode
loops until the hotkey is pressed again.

### Are share codes safe?

`SYN1:` codes carry a checksum and are decoded with bounds on code length, decompressed size and
rule count. Imported rules are created disabled by default; review them before enabling. A share
code can contain scripts, webhook actions and other powerful steps, so only import codes from people
you trust. See [ARCHITECTURE.md#sharing-and-backup](ARCHITECTURE.md#sharing-and-backup).

### PowerShell does not show CLI output

Release builds use the Windows GUI subsystem, so PowerShell does not wait for them and cannot
capture their output or exit code. Prefix commands with `cmd /c`:

```powershell
cmd /c "Synector.exe --rules-validate rule.json"
cmd /c "type rule.json | Synector.exe --rules-apply -"
```

Tools that spawn the process with piped stdio and wait for it, such as Python `subprocess.run` or
Node `child_process`, work directly.

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
playing audio. Per-application volume and channel balance depend on the Windows audio session for
that process; a stopped or suspended app may not expose one. Audio format changes can temporarily
interrupt playback by design.

### High CPU or constant disk writes

Polling is adaptive and backs off when no rule uses a trigger, but many independent polling
triggers, a fast interval schedule or `trace` logging increase load. Process triggers use ETW when
elevated; without elevation they fall back to snapshot polling. Runtime counters and overlay
positions are written coalesced, not on every event.

### The webhook port is busy

The local webhook server binds `127.0.0.1:48123`, and the port is fixed. If another application
already owns it, the server cannot start and webhook-triggered rules will not fire; the daemon log
records the bind error. Close the conflicting application.

### Where do I report a bug or a security issue?

- Bugs and feature requests: [GitHub Issues](https://github.com/psyattack/Synector/issues/new/choose).
  Include the Synector version, `winver`, whether the app was elevated, reproduction steps and the
  relevant log lines with secrets removed.
- Security problems: follow [SECURITY.md](../SECURITY.md) and report privately. Do not open a
  public issue.
