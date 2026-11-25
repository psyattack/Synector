# Changelog

## [3.2.0](https://github.com/psyattack/Synector/compare/v3.1.1...v3.2.0) - 2026-11-25

### Added
- Process Trigger Events: a process trigger now switches between On Process Start and On Process Exit, so one rule can run its action chain when a program closes as well as when it opens. Pre-conditions, cooldown, quota and analytics apply to both events, and the rule list marks each rule with PROC START or PROC EXIT. With Single Instance, start fires for the first instance and exit for the last one.
- Rule Builder Glyphs: every step gains a change glyph that opens the action library in replace mode, including steps nested in If/Else branches, and each pre-condition gains a clone glyph that duplicates it in place. Test Run and Change on the trigger header use the same glyphs.

### Changed
- Exit Actions Removed (breaking): the separate exit pipeline is gone, and a rule file that still contains `on_exit_actions` is rejected, so that rule is skipped on load with its id in the log. Rebuild it as a second rule with a process trigger set to On Process Exit.
- Rule Details: tags sit in a full-width well with the add chip in the same row, and the run settings share one two-column panel with the switches on the left and the cooldown and execution quota on the right. Cooldown is now a toggle over the same value, so no rule field changes.
- Script Editor: the Copy Script button and its confirmation toast are gone; the editor, highlighting and line numbers stay.

### Fixed
- Process Picker: the close button is the quiet header glyph other dialogs use, the search field fills the space between the categories and that glyph, and the select buttons no longer draw over the next row.
- Execution Quota: the panel inside Rule Details takes the inset corner radius instead of square corners.
- Catalog Dialogs: the trigger, action, condition and overlay libraries hide their scrollbar; the mouse wheel and the keyboard still scroll them.

<details><summary>Detailed Changelog</summary>

### Added
- **gui:** add change and clone glyph actions to the rule builder (by @psyattack in f9059b7)
- **gui:** recompose the rule details card (by @psyattack in ce23695)

### Changed
- **engine:** merge process start and exit into one pipeline (by @psyattack in ffcf4df)

### Fixed
- **gui:** fit the process picker header and row buttons (by @psyattack in c826839)
- **gui:** remove the copy script button from the script modal (by @psyattack in ca27f0c)
- **gui:** round the execution quota panel (by @psyattack in c3b74ce)
- **gui:** hide the catalog card scrollbar (by @psyattack in 1111742)

</details>


## [3.1.1](https://github.com/psyattack/Synector/compare/v3.1.0...v3.1.1) - 2026-11-25

### Changed
- Rule Icon Reset: a rule's custom icon is now cleared by hovering the icon tile and clicking the reset glyph shown over it, instead of a separate reset button next to the rule name.

### Fixed
- Rule Details Row: rules with a custom icon no longer push the name field past the right edge of the rule-details card.

<details><summary>Detailed Changelog</summary>

### Fixed
- **gui:** reset a rule's custom icon from the tile itself (by @psyattack in d5f979c)

</details>


## [3.1.0](https://github.com/psyattack/Synector/compare/v3.0.1...v3.1.0) - 2026-11-25

### Added
- Interface Skins and Accent Colour: Settings > Appearance offers a Classic or Modern skin that changes control sizes, density and spacing independently of the dark or light palette, plus a custom accent colour for selection, focus rings and active controls.
- Header Search and Dialogs: the navigation tabs are gone. The rules list is the only page, the header holds the single search field with glyph buttons for the analytics dashboard and settings, which open as dialogs. Typing in the header opens a grouped global search across rules, settings, the activity log, overlay and macro presets and the rule-builder catalogs, and applying a result opens that surface with the query already typed in.
- Searchable Settings: settings are one scrolling column of blocks instead of tabs, and a search field filters every row by title and description, hiding a block when none of its rows match.
- Windows Language and Theme: a first launch adopts the Windows interface language, Russian for a Russian Windows and English for every other, and keeps that choice from then on. Follow Windows Theme renders the palette Windows uses for its own apps, keeps tracking it at most once a second, and shades the tray menu with the same palette; picking a palette by hand turns the switch off.
- Write Logs to Disk: a new Settings > General switch stops new log events without deleting files that already exist, and turning it back on resumes the same day's file.
- Analytics Dialog: the dashboard opens from the header and shows rule-counter, session, live-load and success-rate cards, a live 60-second CPU/RAM chart together with the daemon's own CPU, memory and disk use, executions by source, 24-hour activity, top rules, rules that need attention and the in-app log stream. The daemon keeps the rolling history, so every finished or stopped run is counted exactly once.

### Changed
- Rule Editor: the builder's first section is RULE DETAILS, section headings use the guide's display face without numeric indices, the footer sits on one 52 px strip in both skins, and the app consistently calls configured items rules across the list, share messages and the in-app guide.
- Lower Idle Load: the webhook port is bound only while at least one enabled webhook rule exists and released when the last one is disabled or removed, the hotkey listener sleeps until a hotkey is registered, and process, focus and system-event sampling slow down when no rule consumes them.
- Removed Chocolate Palette: the chocolate theme is gone, and a settings file that still names it cannot be read, so the settings section falls back to defaults with the old file kept as a quarantine copy. Choose Dark or Light in Settings > Appearance.

### Fixed
- Copy and Paste: the Windows clipboard now works in both directions, so copied text reaches other applications and text copied elsewhere can be pasted into Synector.
- Long Text: a long activity-log message no longer stretches the analytics page past the dialog edge, and a long preset name no longer widens a builder step card or runs over its banner title.
- Rule Tags: the editor and picker previews show tag names exactly as entered, matching the rule cards.
- Guide Navigation: a chapter opens at the top instead of keeping the previous chapter's scroll offset.
- Studio Headers: Macro Studio actions sit after the title, and the Overlay Studio header matches its fill, geometry and title style, with Presets placed before Position on Screen.
- Footer Separator: the status footer's hairline is drawn on the panel border instead of a row below it, and the footer keeps one height in both skins.
- Catalog Search: the search fields in the catalog dialogs use the same corner radius as their cards.
- Documentation: the README now opens with a demo video in place of the screenshot grid.

<details><summary>Detailed Changelog</summary>

### Added
- adopt the Windows UI language on first run and follow its theme (by @psyattack in 645404d)
- **gui:** replace the navigation tabs with a search header and dialogs (by @psyattack in 7c1052e)
- **gui:** replace the settings tabs with searchable blocks (by @psyattack in 759d97d)
- **logging:** add a setting to stop writing logs to disk (by @psyattack in eb06a3e)
- **gui:** use rule terminology and refine the rule editor (by @psyattack in ee8ec2d)
- **gui:** split the token layer and add skins with a custom accent (by @psyattack in ad75b46)

### Changed
- enable client-area animations before running tests (by @psyattack in 136c326)
- **readme:** embed the demo video in the header (by @psyattack in e89dc83)
- **daemon:** block the hotkey listener until a hotkey is registered (by @psyattack in d4b431a)

### Fixed
- **gui:** stop long content from widening panels and step cards (by @psyattack in 4ffe777)
- **gui:** unify studio headers and catalog search rounding (by @psyattack in 4148710)
- **gui:** draw the footer separator on the panel border (by @psyattack in 6e8cc1e)
- **gui:** show rule tags verbatim in the inspector (by @psyattack in ca794e7)
- **gui:** reset the guide scroll on chapter change (by @psyattack in 6dc5d13)
- **gui:** restore system clipboard copy and paste (by @psyattack in 6814aed)
- **daemon:** keep idle polling loops asleep and gate the webhook server (by @psyattack in af55914)

</details>


## [3.0.1](https://github.com/psyattack/Synector/compare/v3.0.0...v3.0.1) - 2026-11-22

### Changed
- Scripting API: Lua scripts reach every helper through `syn` (`syn.get`, `syn.set`, `syn.log`, `syn.run_cmd`, `syn.overlay_*`) instead of `ph`, and PowerShell scripts read the run context from `SYN_*` environment variables instead of `PH_*`. The old names are removed rather than aliased, so a script that still uses `ph` needs updating; the script editor and the in-app guide already show the new names.

<details><summary>Detailed Changelog</summary>

### Changed
- **scripting:** rename the ph scripting API to syn (by @psyattack in a9d614e)

</details>


## [3.0.0](https://github.com/psyattack/Synector/compare/v2.9.2...v3.0.0) - 2026-11-22

### Added
- Linked Rules: one rule can call another rule and stop a running one. A call starts the target's pipeline immediately or waits for it to finish, respects the target's enabled state, conditions, cooldown and quota, refuses direct self-calls and cycles, and tells the called pipeline which rule called it.
- Rule Activity Conditions: conditions, If/Else branches, waits and checks can observe another rule, for example "wait until Game Mode starts" or "run only while Recording is active". A wait for a start also succeeds when the rule started and finished between two polls.
- Legal Files: the installer and the portable archive now include `LICENSE` and `NOTICES.md` next to the executable.

### Changed
- Rule File Validation: a rule with an unknown key or an invalid value is rejected with the rule id and the offending keys instead of being silently accepted, and fields removed from the format (`schedule.delay_sec`, the system trigger's `idle_duration_sec`, the resource condition's `duration_sec`) are no longer migrated. Loading a rule file skips only the bad rule instead of the whole file.
- Protected Configuration: a configuration file written by a newer Synector version is read but never overwritten, so an older build cannot silently downgrade it.
- Responsive Interface: clearing logs, testing a step, terminating a process, saving settings or presets and refreshing overlay previews no longer freeze the window while the daemon answers.
- Safer Webhooks and Shortcuts: webhook steps send their headers and body from the app itself and rule shortcuts are written through the Windows shortcut API, so neither passes rule data to a shell; invalid methods and headers are reported as clear errors.
- Verified Updates: an update whose release does not publish a SHA-256 digest is refused instead of installed, and a portable update with process masking targets the executable that launched the daemon and restores the previous one when applying fails.
- Strict Command Line: an unknown flag or a value flag without a value now prints usage and exits with code 2 instead of starting the interface.

### Fixed
- Scheduled Rules: a daily or interval rule that also has a cooldown now fires instead of silently skipping every scheduled time.
- Resource Rules: rules that watch a single metric such as CPU now trigger; a missing companion metric no longer keeps them idle.
- Cancel Versus Stop: cancelling a test run, releasing a held hotkey or stopping a rule from another rule now aborts only the current run and leaves the rule enabled; only the Stop button pauses it.
- Overlay Timers: stopwatch and countdown elements advance with real time, a countdown fires its finish action once per run, and Reset Timer and Toggle Timer act on the overlay's own timers.
- Recorded Input: recorded presses replay in pairs with the 30-35 ms hold that games register, and stopping a recording releases every key and button still held.
- Dialog Dimming: a dialog opened over another one now dims the card below it instead of only darkening the previous scrim.
- Interface Language: labels that stayed in English regardless of the selected language now follow it.
- Share Codes: imported codes re-point bundled overlay buttons at the imported rules, carry none of the sender's local file paths, and oversized codes are rejected cleanly.
- Daemon Writes: a failed save reports an error instead of claiming success, deleted rules disappear after a sync, and shutdown flushes runtime counters and overlay positions.
- Offline `--run-rule` now applies the rule's conditions and quota, so a test without the daemon behaves like a real run.

<details><summary>Detailed Changelog</summary>

### Added
- **rules:** add rule activity conditions to wait and check (by @psyattack in f39e67c)
- **rules:** add rule-to-rule calls and stops (by @psyattack in 848c73e)

### Changed
- **arch:** finish the module split and remove dead surface (by @psyattack in af93009)
- **daemon:** split the daemon core into owned module trees (by @psyattack in 05d5c98)
- **gui:** split the GUI core into owned modules (by @psyattack in a5ae521)
- **arch:** split the foundation modules into owned module trees (by @psyattack in 4b985d2)
- **release:** ship LICENSE and NOTICES.md with the app (by @psyattack in 6b61712)
- consolidate the documentation set and align it with the code (by @psyattack in 828219b)

### Fixed
- **gui:** keep a nested modal's scrim above the card below (by @psyattack in 6e21e3a)
- **cli:** keep the executable path out of the argument parser (by @psyattack in fdfde6d)
- **audit:** resolve all findings from the 2.9.2 code audit (by @psyattack in 9ae2249)

</details>


## [2.9.2](https://github.com/psyattack/Synector/compare/v2.9.1...v2.9.2) - 2026-11-20

### Changed
- Updated Libraries: The interface framework and supporting libraries were upgraded to their latest versions, bringing upstream bug fixes and stability improvements.

### Fixed
- Interface Symbols: Battery, sound, image and similar icons now always use the bundled emoji fonts, so no glyphs appear as missing characters.

<details><summary>Detailed Changelog</summary>

### Changed
- **deps:** update all dependencies to latest versions (by @psyattack in 72f9090)

### Fixed
- **ui:** restore bundled emoji fonts after egui 0.37 update (by @psyattack in 5c4edef)

</details>


## [2.9.1](https://github.com/psyattack/Synector/compare/v2.9.0...v2.9.1) - 2026-11-20

### Added
- System Event Triggers: Rules can now react to sleep, resume, monitor on/off and plug-and-play device connect/disconnect events.
- Documentation: Added a Russian README, a screenshot gallery of the main views, an FAQ, ready-to-use rule examples and a documentation index.
- Release Checksums: Every release now publishes `SHA256SUMS.txt` for manual verification of downloaded artifacts.

### Changed
- Documentation Navigation: The README, architecture and rule-authoring references now link to the owning document instead of repeating the same details.

<details><summary>Detailed Changelog</summary>

### Added
- **daemon:** add sleep, display and device event notifications (by @psyattack in 72d5b6c)

### Changed
- **repo:** polish documentation, community files and release assets (by @psyattack in bb5f2b0)

</details>


## [2.9.0](https://github.com/psyattack/Synector/compare/v2.8.0...v2.9.0) - 2026-11-19

### Added
- Windows Installer: Synector now ships as `Synector-Setup-<version>-Windows-x64.exe` next to the portable zip. Installed copies update by running the new setup silently, downloads are verified against GitHub release SHA-256 digests, and a new `install.cmd` / `install.ps1` bootstrap offers install, portable, update, and uninstall options from a single menu.
- Compact Share Codes: Export any rule set as one `SYN1:` text code that bundles the presets and overlays it references. Every code carries a checksum and is imported atomically in one click; imported rules are added disabled and their runtime counters start fresh.
- Periodic Execution Quotas: Limit a rule to a number of runs per minute, hour, day, week, or month, or define a custom first-run window. When the limit for the current period is reached, the rule skips executions until the counter resets instead of pausing itself.
- Unit-Aware Durations: Duration fields now accept milliseconds, seconds, minutes, hours, and days, automatically select the most readable unit, and remember your choice for each field.
- Rule Authoring CLI: Validate, apply, sync, list, export, enable, disable, reset, and delete rules from JSON files or stdin, with dry-run previews, JSON reports, a generated JSON Schema, and a starter template (`--rules-schema`, `--rules-template`).
- Diagnostic Log Files: Synector writes daily logs to `%LOCALAPPDATA%\Synector\logs\` with configurable levels, 7-day retention, an 8 MiB size cap, and crash capture for troubleshooting.
- Accessibility: Editor controls are now associated with labels and exposed through the accessibility layer, making the interface usable with screen readers and UI automation tools.

### Changed
- Configuration Storage: Settings, rules, presets, and runtime counters now live in separate versioned files written atomically, and an unreadable file is quarantined instead of being silently replaced. Configuration from 2.8.0 and older is not migrated, so upgrade using exported rules or share codes.
- Share Code Compatibility: Codes created by older versions (`PH1:`) are no longer accepted; re-export them from the updated app.
- Lower Disk Load: Configuration reads are cached and runtime counters are flushed in batches, so long automation sessions no longer write to disk on every execution.
- Smoother Window Handling: Main-window geometry is saved after dragging stops instead of on every frame, and overlay positions are persisted without blocking the UI.

### Fixed
- Rule Card Layout: Fixed the pipeline container width so action buttons no longer overlap the step stream, with symmetric margins on both sides.

<details><summary>Detailed Changelog</summary>

### Added
- **ui:** add unit-aware duration inputs (by @psyattack in fe3a65e)
- **rules:** add periodic execution quotas (by @psyattack in d0a2e76)
- **sharing:** replace PH1 codes with checksummed SYN1 share codes (by @psyattack in 8b2b12d)
- **installer:** add Windows installer with installer-driven updates (by @psyattack in 413ab44)
- **logging:** add rotating file logs with levels and panic capture (by @psyattack in 3fd3eb2)
- **cli:** add rule authoring CLI with JSON schema and config-as-code commands (by @psyattack in ebafc31)

### Changed
- **repo:** add issue forms for bug reports and feature requests (by @psyattack in 8ee64c8)
- **config:** split config into versioned section files with safe writes (by @psyattack in f4df3fe)
- **repo:** add architecture doc, line-ending policy, and CI clippy (by @psyattack in 3ae9dff)

### Fixed
- **lint:** simplify boolean filters for clippy 1.100 (by @psyattack in 323b0d1)
- **ui:** fix pipeline container width and symmetric margins on rule card (by @psyattack in 7850b31)

</details>


## [2.8.0](https://github.com/psyattack/Synector/compare/v2.7.0...v2.8.0) - 2026-11-15

### Added
- Template Presets for Overlays and Macros: Save and reuse HUD overlays and macro sequences across rules using the new Preset library. Easily browse, load, delete, or detach presets into independent copies directly within Overlay Studio, Macro Studio, or the Rule Inspector.
- Advanced Macro and Overlay Operations: Trigger macros with dedicated Run, Stop, and Toggle actions, including active thread cancellation. Dynamically update text on active HUD overlays at runtime with the new Set Text overlay operation.
- Universal Wait Conditions: Replaced simple delay pauses with a versatile Wait step. Pause for a fixed duration or hold execution until a target window opens or closes, a process starts or terminates, a hotkey is pressed, or a file is created or deleted, with configurable timeouts and timeout policies.
- Inline Check Step: Insert condition checks anywhere within an action pipeline. If conditions are not met, the remaining steps gracefully stop without recording an error or interrupting other rules.
- Windows Desktop Shortcuts: Create native Windows desktop shortcuts (.lnk) for any automation rule with a single click. Launch rules directly from your desktop, Start menu, or custom keybindings.
- Direct Rule Execution CLI: Launch specific automation rules from the command line using `--run-rule <name_or_id>`, communicating with the background daemon or executing autonomously.
- Action Toggle Modes: Toggle network traffic blocking in Windows Firewall, or toggle window visibility (show/hide) and state (minimize/restore) with a single action step.
- Background Daemon Updates: Software updates are now handled seamlessly by the background daemon. Enjoy optional silent auto-installation, update progress in the tray menu, and non-intrusive GPU overlay alerts.

### Changed
- Streamlined Rule Cards: Rule titles and colored tags now appear neatly in a fixed-height row with status toggle switches aligned to the far right for cleaner visual scanning.
- Polished Trigger Badges: Removed redundant label prefixes on schedule and process badges across all language catalogs for a more concise interface.
- Theme-Adaptive Tray Icons: Tray context menu icons now automatically adapt to Windows light and dark themes.
- Enhanced Process Matching: Automatic .exe extension normalization and improved window focus tracking ensure reliable process detection regardless of how application names are entered.

### Fixed
- Resolved companion satellite processes occasionally failing to terminate cleanly upon primary application exit.
- Other improvements.

<details><summary>Detailed Changelog</summary>

### Added
- **presets:** add overlay and macro templates, control operations, and dynamic text updates (by @psyattack in 55a0e57)
- **rules:** add wait conditions, pipeline check, toggle actions, and rule shortcuts (by @psyattack in a362ca3)
- **updater:** daemon-managed updates, tray integration, overlay alerts, and auto-install (by @psyattack in 5cca2b7)
- **ui:** streamline rule card layout, inline tags, clean trigger badges, and polish tray icons (by @psyattack in 8b9c064)
- **daemon:** enhance process normalization, bound lifecycle cleanup, window focus loop, and session lock detection (by @psyattack in f2f5688)

### Changed
- **readme:** update screen (by @psyattack in 59391f7)

</details>


## [2.7.0](https://github.com/psyattack/Synector/compare/v2.6.0...v2.7.0) - 2026-11-03

### Added
- Rule Tagging and Filtering: Organize automation rules with custom colored tags and icons. Tag filter dropdowns allow quickly viewing rules by category, with tag management integrated directly into the Inspector.
- Drag-and-Drop Card Reordering: Reorder automation rules visually with precision drag handles; custom order is automatically saved and synchronized with the background engine.
- Multi-Mode Rule Sorting: Sort automations by name (ascending or descending), trigger type, action count, or active/paused status.
- Quick Card Action Buttons: Execute, stop, edit, clone, share, or delete rules directly from individual rule cards with dedicated action buttons.
- Kernel ETW Process Monitoring: Subscribes to Windows Kernel Event Tracing (ETW) for instantaneous, zero-polling detection of process launches and terminations when running elevated.
- Task Scheduler Autostart: Startup registration now uses Windows Task Scheduler with highest privileges, launching Synector automatically on logon without UAC prompts.
- Smooth Overlay Fade Transitions: On-screen HUD overlays now transition smoothly into and out of view with smootherstep fade animations.
- Non-Activating HUD Controls: Overlay widgets now process clicks and adjustments without stealing focus from active games or foreground windows.
- Condition Editing in Branches: Swap and configure condition checks inside If/Else action branches using the Condition Catalog modal.
- Modern System Tray Styling: Tray context menu is now styled dynamically to match the active visual theme (Dark, Light, or Chocolate) with custom rendered icons.

### Changed
- Project Rebranding: Renamed the project from ProcessHandler to Synector, updating binaries, named pipes, single-instance mutexes, and documentation.
- Consolidated Interactive Overlay Controls: Unified buttons, toggle switches, and sliders into a streamlined Interactive Control model in Overlay Studio.
- Smart Log Deduplication: Central engine now aggregates consecutive duplicate logs into single entries with execution counters to eliminate log flooding.
- Idle Power Efficiency: Enhanced background engine to remain dormant when inactive, waking instantly on hotkeys or system events with near-zero CPU usage.
- Per-Monitor V2 DPI Manifest: Embedded application manifest for native high-DPI multi-monitor awareness and seamless scaling on Windows 10 and 11.

### Fixed
- Fixed interval schedule cooldown checking to verify elapsed intervals before evaluating rule pre-conditions.

<details><summary>Detailed Changelog</summary>

### Added
- **rules:** add rule tagging, custom sorting, drag-and-drop reordering, and card actions (by @psyattack in 65091a6)
- **rules:** enhance condition editing, pipeline visualization, and log deduplication (by @psyattack in 9bf82cc)
- **overlay:** add smooth fade transitions, non-activating clicks, and streamline element models (by @psyattack in 26fcb72)
- **daemon:** add theme-aware popup styling and custom icons to tray menu (by @psyattack in 6455c7f)
- **daemon:** implement hybrid kernel ETW, task scheduler autostart, and idle dormancy (by @psyattack in 2c3a7fa)

### Changed
- rename project from ProcessHandler to Synector (by @psyattack in e557382)
- **readme:** README and THIRD-PARTY-NOTICES updated (by @psyattack in c9ab8bc)

</details>


## [2.6.0](https://github.com/psyattack/ProcessHandler/compare/v2.5.0...v2.6.0) - 2026-10-30

### Added
- Standalone GPU Overlay Window: Overlays now run in an isolated, high-performance borderless window with hardware acceleration, seamless non-client hit testing, and smooth 60 FPS animations.
- Element Catalog Modal: Added a categorized in-context modal with instant search for browsing and adding overlay elements across visual, interactive, telemetry, and gaming categories.
- Interactive HUD Controls: Added clickable buttons, toggle switches, and sliders to overlays for adjusting volume, toggling rules, muting audio, controlling playback, and managing processes.
- Real-Time Hardware Telemetry: Added on-screen monitoring widgets for system CPU, RAM, network throughput, battery status, and microphone input levels.
- Live Performance Graphs: Added historical charts supporting line, filled area, and histogram styles for tracking system and process metrics over time.
- Multi-Mode Timers and Media Widgets: Added digital clocks, stopwatches, countdown timers, uptime counters, and media player controls to on-screen HUDs.
- Key Visualizer Widgets: Added customizable on-screen keyboard and mouse input visualizers with dedicated layouts for FPS, Action RPG, movement, and text streaming.
- Multi-Monitor Artboard Support: Overlay Studio now automatically detects all connected monitors, their resolutions, and orientations (landscape or portrait) for precise element placement across displays.
- Studio Undo and Redo: Added full multi-level history (Ctrl+Z / Ctrl+Y) to Overlay Studio for tracking element additions, moves, and property changes.
- Companion Overlay Lifecycle: Added an option for overlays to automatically start and close in sync with a target companion application.

### Changed
- Enhanced Memory Trimming: Added configurable soft working set limits, active foreground window fallback, and a safeguard option to skip trimming the currently focused application.
- Updated Rule Inspector and Action Previews with refined layouts, inline checkbox controls, and updated action summaries.
- Expanded the in-app user guide and architectural documentation with comprehensive coverage of Overlay Studio widgets, telemetry, and memory tuning options.
- Updated English and Russian localization catalogs across all new overlay elements, telemetry widgets, interactive controls, and settings.

<details><summary>Detailed Changelog</summary>

### Added
- **overlay:** add standalone egui overlay window and interactive studio elements (by @psyattack in 3ec4724)
- **actions:** enhance memory trimming with soft limits and focus skip (by @psyattack in bc6d316)

</details>


## [2.5.0](https://github.com/psyattack/ProcessHandler/compare/v2.4.0...v2.5.0) - 2026-10-28

### Added
- HUD Overlay Studio: Added an on-screen overlay visual designer featuring a 1:1 canvas, snap-to-grid alignment, pan and zoom navigation, element rotation, and live screen calibration.
- Screen Overlays (HUD): Added a dedicated screen overlay action supporting customizable crosshairs, dynamic text with token interpolation, status badges, progress bars, vector shapes, and images.
- Anti-Capture Protection: Overlay windows support hardware click-through and native Windows anti-capture protection (WDA_EXCLUDEFROMCAPTURE) to keep overlays visible on-screen while invisible to screen recording and streaming tools.
- Trigger and Condition Catalogs: Added categorized studio modals with instant search for discovering and selecting triggers and rule pre-conditions.
- Manual Execution Mode: Added a manual trigger option allowing rules to be executed directly on-demand via the interface without requiring background event polling.
- Live Rule Abort: Added the ability to instantly cancel active long-running rules, loops, and test pipelines directly from the UI.
- Stealth Mode and Process Masking: Added an optional stealth tray mode to hide the notification icon, alongside PE process spoofing to disguise the background daemon in Windows Task Manager.
- Network Connectivity Triggers and Conditions: Added network online and offline state transitions to system event triggers and rule pre-conditions.
- Lua Overlay API: Expanded the embedded Lua scripting engine with dedicated functions (ph.overlay_show, ph.overlay_hide, ph.overlay_toggle, ph.overlay_flash) for programmatic overlay control.

### Changed
- Updated Rule Inspector and rule cards with direct manual run triggers and real-time active overlay tracking.
- Improved rule cloning logic to automatically generate fresh, collision-free identifiers for embedded overlay actions.
- Expanded the in-app interactive guide and documentation with comprehensive Overlay Studio and catalog walkthroughs.
- Updated English and Russian localization catalogs across all new overlay tools, modal catalogs, and stealth settings.

<details><summary>Detailed Changelog</summary>

### Added
- **overlay:** add HUD overlay studio, trigger/condition catalogs, and stealth process masking (by @psyattack in a551729)

### Changed
- **readme:** README updated (by @psyattack in abe8ece)

</details>


## [2.4.0](https://github.com/psyattack/ProcessHandler/compare/v2.3.0...v2.4.0) - 2026-10-25

### Added
- Discord Rich Presence: Added actions to broadcast custom game and application status directly to Discord via local IPC, including activity details, custom image assets, elapsed session timers, interactive buttons, and automatic status clearing when a target process exits.
- Archive Compression and Extraction: Added an archive management action supporting packing and unpacking files and directories across ZIP, 7Z, TAR, TAR.GZ, and RAR formats with configurable compression levels and overwrite protection.
- Filesystem Operations: Added automated file and directory management actions, including creating files or folders, copying, moving, renaming, deleting, and writing or appending text with dynamic token interpolation.
- Process Memory Trimming: Added a memory optimization action that forcefully flushes process working set RAM to free up system memory for targeted applications.
- Monthly Schedule Trigger: Added monthly recurring triggers allowing rules to execute automatically on a designated day of the month at a specific time.

### Changed
- Expanded the Action Catalog and Rule Inspector with dedicated visual editors for Discord status, file archiving, filesystem management, and memory trimming.
- Updated the in-app user guide, architecture documentation, and README with guides for the new automation actions.
- Expanded English and Russian localization catalogs across all new actions, triggers, and configuration options.

<details><summary>Detailed Changelog</summary>

### Added
- **actions:** add discord rich presence, archive compression, filesystem ops, and memory trimming (by @psyattack in efe4028)

</details>


## [2.3.0](https://github.com/psyattack/ProcessHandler/compare/v2.2.0...v2.3.0) - 2026-10-24

### Added
- Extended Rule Pre-conditions: Added system event checks (user idle duration, session lock/unlock, AC/battery power) and resource thresholds (monitoring sustained process CPU % and RAM MB) to rule prerequisites and conditional branches.
- File Watcher Extension Filtering: File pre-conditions now support directory monitoring with specific file extension filters (such as *.log or *.txt).
- Audio Format Switching: Added an audio format action allowing dynamic switching of sample rates (Hz), bit depths, and channel configurations on audio playback and recording devices.
- Independent Channel Balance: Added separate Left and Right channel volume controls for fine-grained audio balance adjustment.
- Unified Application Audio: Per-process volume adjustments and mute toggling are now integrated directly into the unified Audio Control action.
- Customizable Beep Tone Action: Added tone generator actions with configurable frequency (Hz) and duration (ms) alongside standard audio file playback.
- Macro Studio Anti-Bot Humanization: Added a one-click Humanize mode in Macro Studio that injects natural Gaussian delays, key hold variance, and mouse tremor across recorded sequences.
- Global Hotkey Continuous Loops: Hotkey triggers now support toggling continuous execution loops in addition to hold-and-repeat mode.
- Contextual Empty State Hints: Added helpful guidance messages when rule search or filter queries yield no results.

### Changed
- Unified action models across audio controls, default device switching, and script execution to streamline rule creation.
- Updated in-app user guide and documentation with guidance on Windows UIPI privilege elevation for global game hotkeys.
- Expanded English and Russian localization catalogs across all new pre-conditions, audio formats, and macro settings.

<details><summary>Detailed Changelog</summary>

### Added
- **rules:** expand pre-conditions, advance audio controls, and add macro humanization (by @psyattack in d68978b)

</details>


## [2.2.0](https://github.com/psyattack/ProcessHandler/compare/v2.1.0...v2.2.0) - 2026-10-23

### Added
- Rule Pre-conditions: Rules can now be gated by prerequisites before running. Supported checks include running process status, active foreground window focus, time and day-of-week schedule windows, power source (AC power vs battery), and file or directory existence.
- Logical AND / OR Condition Chaining: Pre-conditions and branch checks support customizable boolean AND / OR operators for combining multiple conditions.
- Conditional Branching (If / Else If / Else): Added a dedicated branching action allowing pipelines to evaluate conditions dynamically at runtime and execute corresponding action branches, including repeatable Else If branches and a fallback Else sequence.
- Nested Drag-and-Drop Pipeline Reordering: Full drag-and-drop reordering with animated drop laser guides, auto-scrolling, and floating card previews is now supported directly within nested If / Else action branches.
- Macro Studio Organic Mouse Smoothing: Added an organic motion mode for smooth mouse movements, simulating natural human hand movement and speed fluctuations.
- Expandable Mouse Move Groups in Macro Timeline: Grouped mouse movement blocks in the Macro Studio timeline can now be expanded into individual steps for detailed coordinate inspection and fine-tuning.
- Extended Process Picker Integration: The running process selector modal now connects directly with rule pre-conditions, nested branch conditions, and sub-actions inside If / Else blocks.

### Changed
- Action catalog now restricts recursive logic nesting to keep pipelines structured and clean.
- Updated the interactive user guide, architecture documentation, and README with conditional automation walkthroughs.
- Expanded English and Russian localization catalogs across all new conditions, branching tools, and Macro Studio controls.

<details><summary>Detailed Changelog</summary>

### Added
- **rules:** add pre-conditions and dynamic if-else branching with dnd pipeline (by @psyattack in 3b9ba64)

</details>


## [2.1.0](https://github.com/psyattack/ProcessHandler/compare/v2.0.0...v2.1.0) - 2026-10-23

### Added
- In-game 3D camera turn and relative mouse movement support in macro recorder and playback.
- Hardware keyboard scan codes support for reliable input in games and DirectInput applications.
- Hotkey hold mode with continuous execution loops while the key combination is held down.
- Integrated automatic update checker and in-app updater via GitHub Releases.
- Safety key release on macro cancel or stop to prevent stuck keys during playback.

### Changed
- Improved macro playback timing and hold durations to ensure smooth detection in high-refresh and frame-capped games.
- Enhanced settings, analytics, and process inspector user interface layouts.
- Updated in-app documentation and rule guides with the latest automation capabilities.

<details><summary>Detailed Changelog</summary>

### Added
- **macro:** support in-game camera turns, hotkey hold loops, and centralize i18n (by @psyattack in 4497659)

</details>
