<p align="center">
  <img src="assets/app.svg" alt="Synector Logo" width="120" height="120">
</p>

<h1 align="center">Synector</h1>

<p align="center">Lightweight, fast, native Windows process automation service and task monitor built in 100% pure Rust.</p>

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

## Features

Two tiers, one executable: a silent tray daemon runs automation 24/7, while the hardware-accelerated
desktop GUI is only needed to author and monitor rules. All of it is native Windows API — no browser
engine, no polling loops, no telemetry.

* **Dual-tier, pure Rust** — the tray daemon owns rules, triggers and system monitoring; close the GUI and automation keeps running with near-zero CPU.
* **Event-driven, not polling** — kernel ETW process tracing (elevated), low-level global hotkeys, window focus, file, webhook, power and hardware events.
* **Visual rule pipelines** — drag-and-drop actions with undo/redo, timers and event waits with timeout policies, mid-pipeline checks, If/Else branching, per-rule quotas and cooldowns.
* **Companion process mode** — bind helper tools, HUDs or recorders to a primary game or app: they start with it, are supervised while it runs, and are terminated when it exits.
* **Overlay Studio (HUD)** — WYSIWYG GPU-accelerated overlays: crosshairs, telemetry badges, interactive widgets, multi-monitor placement and anti-capture streaming protection.
* **Macro Studio** — keyboard and mouse recording with DirectInput-compatible scan codes, humanized motion jitter, camera-rotation support, and reusable presets with Run/Stop/Toggle operations.
* **Scripting and CLI automation** — embedded Lua 5.4 with a `ph` API, PowerShell and Batch actions; JSON rule authoring via `--rules-apply`/`--rules-sync` with schema, template and reference discovery ([CLI reference](docs/RULE_AUTHORING.md)), desktop `.lnk` shortcuts and `--run-rule`.
* **System and device control** — working-set trimming, CPU priority and affinity, firewall toggles, window actions, master/per-app volume, channel balance and audio endpoint switching.
* **Observability** — live analytics dashboard with KPI cards, real-time in-app log stream, and daily rotating log files with configurable verbosity.
* **Installer and portable builds** — a native Windows installer with an uninstaller, Start Menu shortcut and `App Paths` integration, or a single portable executable; both update themselves through daemon-managed GitHub release checks, tray progress and optional auto-install.

## Installation

Requires Windows 10 / 11 (x64).

- **Setup** — `Synector-Setup-<version>-Windows-x64.exe` from [**GitHub Releases**](https://github.com/psyattack/Synector/releases/latest).
- **Portable** — `Synector-v<version>-Windows-x64.zip`: extract anywhere and run `Synector.exe`.
- **Installer script** — one line in PowerShell: `irm https://raw.githubusercontent.com/psyattack/Synector/main/installer/install.ps1 | iex`.

> [!IMPORTANT]
> **Administrator Privileges**: Synector launches elevated by default (override with `--no-elevate`). Required for:
> * **Kernel ETW**: Zero-polling, real-time process lifecycle events.
> * **UIPI Bypass**: Hardware input simulation into elevated games and apps.
> * **Process Control & Autostart**: Memory/affinity tuning and silent elevated logon via Task Scheduler.

> [!NOTE]
> Automatically registers for Windows startup on first launch (configurable in Settings).

## Development

<details>
<summary><b>Setup</b></summary>

#### Prerequisites
- **Rust** (1.75+)

#### Build & Run Commands

```powershell
# Clone the repository
git clone https://github.com/psyattack/Synector.git
cd Synector

# Run in development mode (spawns daemon and opens GUI)
cargo run

# Run only the GUI (connects to an existing background daemon)
cargo run -- --gui

# Run only the background daemon (tray mode)
cargo run -- --daemon

# Execute test suite
cargo test

# Build optimized release binary
cargo build --release
```

</details>

<details>
<summary><b>Stack</b></summary>

#### Frontend / UI
- **egui**
- **eframe**

#### Backend / Engine
- **Tokio**
- **mlua**
- **windows-rs**
- **reqwest**
- **Serde + Serde JSON**

</details>

<details>
<summary><b>Structure</b></summary>

```
Synector/
├── assets/                                # Application assets & branding
│   ├── fonts/                             # Bundled variable fonts (Archivo, GeistMono, Inter)
│   ├── app.ico                            # Win32 application icon
│   ├── app.png                            # High-res RGBA icon
│   └── app.svg                            # Vector logo
├── installer/
│   ├── install.cmd                        # Console bootstrap (Setup / Portable / Uninstall)
│   ├── install.ps1                        # Bootstrap logic, release download and SHA-256 check
│   └── synector.iss                       # Inno Setup installer script (built in CI)
├── docs/
│   ├── ARCHITECTURE.md                    # Subsystem behavior and design conventions
│   └── RULE_AUTHORING.md                  # Command-line rule authoring reference
├── src/                                   # Source code
│   ├── daemon/                            # Background daemon service
│   │   ├── actions.rs                     # Execution handlers (Process, Window, Tuning, Audio, Scripts, Macros)
│   │   ├── discord_rpc.rs                 # Discord Rich Presence client over named pipe
│   │   ├── engine.rs                      # Central automation state engine & rule evaluator
│   │   ├── file_watcher.rs                # Filesystem polling & directory event watcher
│   │   ├── hotkeys.rs                     # Low-level global hotkey listener & hold-mode engine
│   │   ├── ipc_server.rs                  # Named pipe listener & connection worker
│   │   ├── lua_runner.rs                  # Embedded Lua 5.4 execution runner with custom `ph` API
│   │   ├── overlay_manager.rs             # Background overlay process lifecycle & multi-monitor manager
│   │   ├── pe_mask.rs                     # Windows PE process spoofing & Task Manager masking (RuntimeBroker.exe)
│   │   ├── system_events.rs               # Win32 idle time & AC/battery power monitor
│   │   ├── telemetry.rs                   # System telemetry provider (CPU, RAM, network, battery, mic)
│   │   ├── tray.rs                        # Win32 system tray manager, menu & message loop
│   │   ├── watcher.rs                     # Hybrid process watcher: Kernel ETW (elevated) & Toolhelp32 snapshot polling
│   │   └── webhook_server.rs              # Local HTTP webhook server (port 48123)
│   ├── gui/                               # egui desktop user interface
│   │   ├── components/                    # UI components
│   │   │   ├── icons.rs                   # Win32 shell icon extractor, image loader & texture cache
│   │   │   ├── modal.rs                   # Unified animated modal window framework
│   │   │   ├── overlay_renderer.rs        # GPU-accelerated overlay HUD element renderer
│   │   │   ├── script_highlighter.rs      # PowerShell, Batch & Lua syntax highlighter
│   │   │   └── widgets.rs                 # Custom canvas controls (buttons, inputs, cards, drag handles, glyphs)
│   │   ├── views/                         # Application screens & panels
│   │   │   ├── action_catalog_modal.rs    # Studio modal for categorized action browsing & search
│   │   │   ├── analytics_view.rs          # Real-time event telemetry stream & KPI metric cards
│   │   │   ├── changelog_modal.rs         # GitHub release notes & changelog viewer
│   │   │   ├── condition_catalog_modal.rs # Studio modal for categorized pre-condition browsing & search
│   │   │   ├── element_catalog_modal.rs   # Studio modal for categorized overlay HUD elements & widgets
│   │   │   ├── guide_modal.rs             # GitBook-style interactive documentation & recipes
│   │   │   ├── header_bar.rs              # Custom window titlebar & navigation tabs
│   │   │   ├── inspector.rs               # Rule inspector, trigger & action editor (drag-and-drop, undo/redo)
│   │   │   ├── macro_modal.rs             # Macro timeline editor & playback studio
│   │   │   ├── overlay_modal.rs           # Overlay Studio (WYSIWYG artboard, HUD elements, snap-to-grid)
│   │   │   ├── process_picker_modal.rs    # In-context process picker modal with filter & sorting
│   │   │   ├── rules_view.rs              # Rule management list & cards
│   │   │   ├── script_modal.rs            # Expandable script editor modal (PowerShell, CMD, Lua)
│   │   │   ├── settings_view.rs           # Balanced 2x2 settings (System, Engine, Appearance, Updates)
│   │   │   └── trigger_catalog_modal.rs   # Studio modal for categorized trigger browsing & search
│   │   ├── macro_recorder.rs              # Win32 low-level keyboard/mouse macro recorder
│   │   ├── overlay_window.rs              # Standalone borderless GPU-accelerated overlay window
│   │   ├── state.rs                       # GUI state management & IPC synchronization
│   │   └── theme.rs                       # Design system tokens (Dark, Light, Chocolate palettes)
│   ├── i18n/                              # Internationalization & localization
│   │   ├── en.rs                          # English string catalog
│   │   └── ru.rs                          # Russian string catalog
│   ├── ipc/                               # Inter-process communication
│   │   ├── client.rs                      # Named pipe IPC client
│   │   └── protocol.rs                    # Request & response message types (Serde)
│   ├── autorun.rs                         # Windows Task Scheduler autorun integration
│   ├── cli.rs                             # Command-line rule authoring (validate, apply, list, export)
│   ├── config.rs                          # Split JSON configuration persistence (settings, rules, library, runtime)
│   ├── daemon.rs                          # Daemon entrypoint & service lifecycle
│   ├── gui.rs                             # GUI entrypoint & eframe application loop
│   ├── i18n.rs                            # Internationalization loader & language switcher
│   ├── logging.rs                         # Process-wide daily file logging (rotation, SYNECTOR_LOG levels, panics)
│   ├── models.rs                          # Domain data models (Rule, Trigger, Action, Process)
│   ├── sharing.rs                         # Portable automation compression, encoding & clipboard sharing
│   ├── shortcut.rs                        # Windows shell link (.lnk) desktop shortcut generator
│   ├── single_instance.rs                 # Win32 Named Mutex guards for daemon and GUI
│   ├── updater.rs                         # GitHub releases auto-update & self-replacement engine
│   └── main.rs                            # CLI dispatcher (--daemon, --gui, --run-rule, default)
├── build.rs                               # Windows resource compilation (icon, manifest)
└── Cargo.toml                             # Rust manifest & dependencies
```

How these pieces behave and interact — the two tiers, the data flow, and the design
conventions behind the rule engine, overlays, input simulation and process monitoring — is
described in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

</details>

## Configuration

All user files live in **`%LOCALAPPDATA%\Synector\`**:

| File / folder | Contents |
|---|---|
| `settings.json` | Window, tray, language, theme and update preferences |
| `rules.json` | Automation rules |
| `library.json` | Overlay and macro presets |
| `runtime.json` | Execution counters and last-run timestamps |
| `logs\` | Daily diagnostic logs, one file per day |

These files are managed by the daemon and rewritten as you make changes — back them up or edit them
only while Synector is stopped.

## License

This project is licensed under the [MIT License](LICENSE). Third-party components: [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Credits

- **UI/UX Idea:** [ArtCraft Launcher](https://github.com/storytold/craft-launcher)

## Support & Contributing

For issues, questions, or feature requests, please open an issue on [**GitHub Issues**](https://github.com/psyattack/Synector/issues).
