<p align="center">
  <img src="assets/app.svg" alt="Synector Logo" width="120" height="120">
</p>

<h1 align="center">Synector</h1>

<p align="center">Lightweight, native Windows automation suite and process supervisor built in 100% pure Rust.</p>

<p align="center">
  <a href="https://github.com/psyattack/Synector/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/psyattack/Synector"></a>
  <a href="https://github.com/psyattack/Synector/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/psyattack/Synector/total"></a>
  <a href="LICENSE"><img alt="Licence: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
</p>

<p align="center">
  <a href="https://github.com/psyattack/Synector/releases/latest">Download</a>
  ·
  <a href="https://github.com/psyattack/Synector/issues/new">Issues</a>
</p>

> [!WARNING]
> Synector interacts directly with Windows system processes and administrative APIs. The developers are not liable for any errors, including those caused by the user, as also stated in the license.

<p align="center">
  <img src="https://github.com/user-attachments/assets/d81b93e2-cac7-42d2-a23a-596cdb111d51">
</p>

## Overview

Synector ships as a single Windows executable with two tiers. A silent tray **daemon** owns the rule
engine, triggers, runtime counters and system monitoring around the clock. A hardware-accelerated
**GUI** (eframe/egui) is a controlling client: it edits draft rules, displays daemon state and closes
without stopping automation.

Synector talks to Windows directly: Win32 APIs, Win32 named pipes, embedded Lua and no browser
engine, bundled web runtime or telemetry. Detection is event-driven where Windows exposes an event
source — kernel ETW for process lifecycles, low-level hooks during macro recording, HTTP webhooks,
Raw Input camera deltas, power broadcasts for sleep/resume and display state, and plug-and-play
device notifications. Foreground-window focus, file changes, resource thresholds, session lock,
AC/battery power and network state are sampled with small, adaptive polling loops instead of
pretending otherwise.

## Features

* **Dual-tier, pure Rust** — the tray daemon owns rules, triggers and monitoring; close the GUI and automation keeps running with near-zero CPU.
* **Hybrid event detection** — kernel ETW process tracing when elevated, adaptive Toolhelp32 snapshot polling otherwise; both feed the same trigger surface. Global hotkeys, window focus, file watcher, HTTP webhook, sleep/resume, display on/off, device connect/disconnect, session lock/unlock, AC/battery power, user idle/active and network state.
* **Visual rule pipelines** — drag-and-drop actions with undo/redo, wait steps with timeout policies, mid-pipeline checks, If/Else branching, toggle actions, per-rule cooldowns and lifetime or periodic execution quotas.
* **Companion process mode** — bind helper tools, HUDs or recorders to a primary game or app: they start with it, are supervised while it runs, and are terminated when it exits.
* **Overlay Studio (HUD)** — WYSIWYG GPU-accelerated overlays: crosshairs, telemetry badges, graphs, timers, media widgets, interactive controls, multi-monitor placement and `WDA_EXCLUDEFROMCAPTURE` anti-capture protection.
* **Macro Studio** — keyboard and mouse recording through low-level Windows hooks, DirectInput-compatible scan-code playback, humanized motion, Raw Input camera turns, Run/Stop/Toggle operations and reusable presets.
* **Scripting and CLI automation** — embedded Lua 5.4.9 with the `ph` API, PowerShell and Batch actions; JSON rule authoring via `--rules-apply`/`--rules-sync` with schema, template and reference discovery ([CLI reference](docs/RULE_AUTHORING.md)); `SYN1:` share codes, desktop `.lnk` shortcuts and `--run-rule`.
* **System and device control** — working-set trimming, CPU priority and affinity, firewall toggles, window actions, master/per-app volume, channel balance, audio format and default playback/recording device switching.
* **Observability** — analytics dashboard with KPI cards, real-time in-app log stream, and daily rotating log files with configurable verbosity.
* **Installer and portable builds** — a native Windows installer with an uninstaller, Start Menu shortcut and `App Paths` integration, or a single portable executable. Both update themselves through daemon-managed GitHub release checks with SHA-256 verification, tray progress and optional auto-install.

## Installation

Requires Windows 10 / 11 (x64).

- **Setup** — `Synector-Setup-<version>-Windows-x64.exe` from [**GitHub Releases**](https://github.com/psyattack/Synector/releases/latest).
- **Portable** — `Synector-v<version>-Windows-x64.zip`: extract anywhere and run `Synector.exe`.
- **Installer script** — one line in PowerShell: `irm https://raw.githubusercontent.com/psyattack/Synector/main/installer/install.ps1 | iex`.

> [!IMPORTANT]
> **Administrator Privileges**: Synector launches elevated by default (override with `--no-elevate`). Required for:
> * **Kernel ETW**: zero-polling, real-time process lifecycle events.
> * **UIPI Bypass**: hardware input simulation into elevated games and apps.
> * **Process Control & Autostart**: memory/affinity tuning and silent elevated logon via Task Scheduler.

> [!NOTE]
> The daemon registers a Task Scheduler autostart entry on first launch (configurable in Settings).

## Development

Requires Rust **1.95 or newer** on Windows. The floor comes from the `eframe`/`egui` 0.36
dependencies.

```powershell
# Clone the repository
git clone https://github.com/psyattack/Synector.git
cd Synector

# Compile and run all unit tests
cargo check
cargo test

# Lint gate used by CI (warnings are errors)
cargo clippy -- -D warnings

# Run in development mode (spawns the daemon and opens the GUI)
cargo run

# Run only the background daemon (tray mode)
cargo run -- --daemon

# Open the GUI against an existing daemon
cargo run -- --gui

# Print daemon status over the named pipe
cargo run -- --status

# Build the optimized release binary
cargo build --release
```

CI (`.github/workflows/ci.yml`) runs `cargo check`, `cargo test` and `cargo clippy -- -D warnings`
on `windows-latest`.

### Stack

- **UI:** egui / eframe, egui_extras, egui-phosphor
- **Runtime:** Tokio
- **Engine:** mlua (vendored Lua 5.4.9), windows-rs
- **Data:** Serde / Serde JSON, schemars
- **Networking:** reqwest + rustls
- **Compression and hashing:** brotli, flate2, zip, sha2

### Structure

```
Synector/
├── .github/
│   ├── ISSUE_TEMPLATE/                    # Bug report and feature request forms
│   └── workflows/
│       ├── ci.yml                         # Check, test and clippy on windows-latest
│       └── release.yml                    # Portable zip, Inno Setup installer and GitHub release
├── assets/
│   ├── fonts/                             # Bundled variable fonts (Archivo, GeistMono, Inter) + OFL texts
│   ├── app.ico                            # Win32 application icon
│   ├── app.png                            # High-res RGBA icon
│   └── app.svg                            # Vector logo
├── docs/
│   ├── ARCHITECTURE.md                    # Subsystem behavior and design conventions
│   └── RULE_AUTHORING.md                  # Command-line rule authoring reference
├── installer/
│   ├── install.cmd                        # Console bootstrap (menu: Setup / Portable / Uninstall)
│   ├── install.ps1                        # Release download, SHA-256 verification, Setup/Portable/Uninstall
│   └── synector.iss                       # Inno Setup script (compiled only in CI)
├── src/
│   ├── daemon/                            # Background daemon
│   │   ├── actions.rs                     # Execution handlers for every Action variant
│   │   ├── discord_rpc.rs                 # Discord Rich Presence client over \\.\pipe\discord-ipc-*
│   │   ├── engine.rs                      # Rule scheduler, quotas/cooldowns, active executions, analytics
│   │   ├── file_watcher.rs                # Adaptive directory/file polling watcher
│   │   ├── hotkeys.rs                     # Global hotkey monitor (plain, hold and repeat-while-held modes)
│   │   ├── ipc_server.rs                  # Named pipe listener and connection worker
│   │   ├── lua_runner.rs                  # Embedded Lua 5.4.9 runtime with the custom `ph` API
│   │   ├── overlay_manager.rs             # Overlay process lifecycle, calibration and monitor placement
│   │   ├── pe_mask.rs                     # WindowsServiceHost.exe masked daemon image
│   │   ├── system_events.rs               # Sleep/resume, display, device, session, power, idle and network events
│   │   ├── telemetry.rs                   # CPU/RAM/network/battery/mic samples, key visualizer, media state
│   │   ├── tray.rs                        # Win32 tray icon, menu, notifications and update progress
│   │   ├── watcher.rs                     # Hybrid ETW / Toolhelp32 process watcher
│   │   └── webhook_server.rs              # Local HTTP trigger server (127.0.0.1:48123)
│   ├── gui/                               # egui desktop client
│   │   ├── components/                    # Shared widgets and renderers
│   │   │   ├── duration.rs                # Unit-aware duration input (ms/s/min/h/d)
│   │   │   ├── icons.rs                   # Win32 shell icon extraction, image loading, texture cache
│   │   │   ├── modal.rs                   # Animated modal window framework
│   │   │   ├── overlay_renderer.rs        # GPU overlay element renderer
│   │   │   ├── script_highlighter.rs      # PowerShell, Batch and Lua syntax highlighting
│   │   │   └── widgets.rs                 # Custom canvas controls (buttons, inputs, cards, drag handles)
│   │   ├── views/                         # Application screens and panels
│   │   │   ├── action_catalog_modal.rs    # Action catalog with categories and search
│   │   │   ├── analytics_view.rs          # KPI cards, event telemetry and live log stream
│   │   │   ├── changelog_modal.rs         # Release notes and changelog viewer
│   │   │   ├── condition_catalog_modal.rs # Pre-condition catalog
│   │   │   ├── element_catalog_modal.rs   # Overlay element catalog
│   │   │   ├── guide_modal.rs             # Bilingual in-app Rule Builder guide
│   │   │   ├── header_bar.rs              # Custom titlebar and navigation tabs
│   │   │   ├── inspector.rs               # Rule inspector, trigger/action editor, drag-and-drop, undo/redo
│   │   │   ├── macro_modal.rs             # Macro timeline editor and presets
│   │   │   ├── overlay_modal.rs           # Overlay Studio artboard and element editor
│   │   │   ├── process_picker_modal.rs    # Process picker with filters and sorting
│   │   │   ├── rules_view.rs              # Rule list, tags, filters and sorting
│   │   │   ├── script_modal.rs            # PowerShell, CMD and Lua script editor
│   │   │   ├── settings_view.rs           # System, engine, appearance and update settings
│   │   │   └── trigger_catalog_modal.rs   # Trigger catalog with categories and search
│   │   ├── components.rs                  # Component module exports
│   │   ├── macro_recorder.rs              # Low-level keyboard/mouse macro recorder and Raw Input capture
│   │   ├── overlay_window.rs              # Standalone borderless GPU overlay window
│   │   ├── state.rs                       # GUI state, drafts, undo/redo, IPC synchronization
│   │   ├── theme.rs                       # Design tokens and Dark/Light/Chocolate palettes
│   │   └── views.rs                       # View module exports
│   ├── i18n/                              # Localization catalogs
│   │   ├── en.rs                          # English strings
│   │   └── ru.rs                          # Russian strings
│   ├── ipc/                               # Inter-process communication
│   │   ├── client.rs                      # Named pipe client
│   │   └── protocol.rs                    # IpcRequest/IpcResponse and DaemonState types
│   ├── autorun.rs                         # Task Scheduler autostart integration
│   ├── cli.rs                             # Rule authoring CLI (validate, apply, sync, share, ...)
│   ├── config.rs                          # Versioned split JSON persistence and read caches
│   ├── daemon.rs                          # Daemon entrypoint, loops and subsystem wiring
│   ├── gui.rs                             # GUI entrypoint and eframe application loop
│   ├── i18n.rs                            # Language registry and `Strings` catalog interface
│   ├── ipc.rs                             # `PIPE_NAME` and IPC re-exports
│   ├── logging.rs                         # Daily log files, levels, retention and panic capture
│   ├── main.rs                            # CLI dispatcher (overlay, rule CLI, status, elevation, tiers)
│   ├── models.rs                          # Domain model (Rule, Trigger, Action, conditions, overlays)
│   ├── sharing.rs                         # SYN1 share codes (Brotli + Base64, checksum, bounded decode)
│   ├── shortcut.rs                        # Windows shell link (.lnk) rule shortcuts
│   ├── single_instance.rs                 # Named mutex guards and window focus helpers
│   └── updater.rs                         # GitHub release checks, SHA-256 verification, self-update
├── build.rs                               # Windows resource compilation (icon, version, manifest)
├── CHANGELOG.md                           # Release history
├── Cargo.toml                             # Rust manifest and dependencies
├── LICENSE                                # MIT license
├── README.md                              # This file
└── THIRD-PARTY-NOTICES.md                 # Third-party licenses and attributions
```

How the pieces behave and interact — the two tiers, the data flow, the rule engine, overlays, input
simulation and process monitoring — is described in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Configuration

All user files live in **`%LOCALAPPDATA%\Synector\`**. Set `SYNECTOR_CONFIG_DIR` to relocate the whole
directory for portable or isolated runs.

| File / folder | Contents |
|---|---|
| `settings.json` | Window geometry, tray visibility, daemon masking, language, theme and update preferences |
| `rules.json` | Automation rule definitions (runtime counters are written zeroed) |
| `library.json` | Overlay and macro presets |
| `runtime.json` | Per-rule execution counters, last-run timestamps and custom quota-window anchors |
| `logs\` | Daily diagnostic logs, one file per day, 7-day retention and an 8 MiB cap per file |

Writes are atomic (temp file → flush → rename) and every section carries a schema version. A section
that fails to parse is quarantined as `<name>.corrupt-<timestamp>` before defaults replace it, so a
hand-edited file is never silently destroyed. While the daemon is reachable it is the only writer;
the GUI and CLI fall back to direct file writes only when IPC is unavailable. Back up or edit these
files while Synector is stopped.

## Rule authoring from the CLI

Rules are plain JSON and can be validated, applied, replaced, exported, shared and reset without
opening the GUI:

```powershell
Synector.exe --rules-schema > schema.json
Synector.exe --rules-template schedule:play_sound > rule.json
Synector.exe --rules-validate rule.json
Synector.exe --rules-apply rule.json
Synector.exe --rules-sync rule.json --dry-run --json
Synector.exe --rules-share my_rule
```

See [docs/RULE_AUTHORING.md](docs/RULE_AUTHORING.md) for the full command reference, quota semantics,
JSON report shapes and `SYN1:` share-code behavior.

## License

This project is licensed under the [MIT License](LICENSE). Third-party components:
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Credits

- **UI/UX Idea:** [ArtCraft Launcher](https://github.com/storytold/craft-launcher)

## Support & Contributing

For issues, questions, or feature requests, please open an issue on
[**GitHub Issues**](https://github.com/psyattack/Synector/issues).
