# Rule authoring from the command line

Synector rules are plain JSON files. The CLI validates, applies, exports, shares and maintains them
without opening the GUI. While the daemon is running changes go through IPC and take effect
immediately; when it is not, the same operation is written to
`%LOCALAPPDATA%\Synector\rules.json` (and `runtime.json` where counters change) and applied on the
next daemon start.

Related documents: [documentation map](README.md) · [rule examples](EXAMPLES.md) ·
[Lua scripting](SCRIPTING.md) · [architecture](ARCHITECTURE.md#rule-engine).

## Commands

| Command | Description |
|---|---|
| `--rules-validate <file\|->` | Parse and validate a rule file; no changes are made |
| `--rules-apply <file\|->` | Create or update rules from a file (upsert by `id`) |
| `--rules-sync <file\|->` | Make the configured rules match the file exactly (upsert + delete missing) |
| `--rules-list [--json]` | List rules with state, execution counter and trigger |
| `--rules-export [<file>]` | Export all rules as JSON to stdout or a file |
| `--rules-delete <id>` | Delete one rule |
| `--rules-enable <id>`, `--rules-disable <id>` | Toggle one rule |
| `--rules-reset <id>` | Clear `execution_count`/`last_executed_at`/quota anchor and re-enable |
| `--rules-share [<id>]` | Print a `SYN1:` share code for one rule (id or name) or all rules |
| `--rules-import-share <code\|file\|->` | Import rules from a `SYN1:` share code |
| `--rules-example` | Print an example rule file |
| `--rules-schema` | Print the draft-07 JSON Schema for one rule object |
| `--rules-reference` | Print a human-readable catalogue of triggers/actions/conditions |
| `--rules-template <trigger>[:<action>]` | Print a ready-to-edit rule skeleton |
| `--help` | Print the command reference |

Flags:

- `--json` switches most commands (validate/apply/sync/list/export/delete/enable/disable/reset/share/
  import-share) to machine-readable output.
- `--dry-run` previews `--rules-apply` and `--rules-sync` without writing anything.
- `--enable-imported` makes `--rules-import-share` enable the imported rules. Without it, imported
  rules are created disabled.
- A path of `-` reads the rule file or share code from stdin.

Arguments are validated against the selected command. An unknown argument such as `--dry-runn`, two
command flags in one invocation, or the same command flag twice is a usage error: the command
reference is printed and the process exits with code 2. Other process flags, including
`--no-elevate`, are not valid together with a rule command, and a misspelled `--rules-*` command is
reported instead of falling through to a daemon or GUI start. `--run-rule <id>` is a runtime command,
not a rule-authoring one; a missing or empty id is likewise a usage error that exits with code 2.

## File format

A rule file is one rule object, an array of rules, or `{"rules": [...]}`. `id` is the upsert key and
must be unique and stable; `name` is required. The array and wrapper forms are file containers only —
`--rules-schema` describes a single rule object.

Runtime state is not part of the file format in practice: the daemon keeps `execution_count`,
`last_executed_at` and the custom quota window anchor in `runtime.json`, writes those fields zeroed
in `rules.json`, and preserves the live values when an existing rule is updated. Counters included
in a rule file are therefore ignored.

Keys are checked against the model before deserialization. An unknown key — a typo such as
`max_execution`, or a field removed from the format — is reported with its JSON path:
`--rules-validate`, `--rules-apply` and `--rules-import-share` reject the input, while the daemon
logs the rule id and keys and skips only that rule when it loads `rules.json`. Values are checked on
the same load with the per-rule validator the GUI Save and apply paths use: a rule with invalid
values (an out-of-range custom quota window, a wait outside 0..=86400 s, a malformed schedule) is
likewise skipped alone with its id and errors, so neither check drops a rule that would otherwise
be accepted. There is no migration: update the file. The removed fields are `schedule.delay_sec`,
the `system` trigger's `idle_duration_sec`, the `resource` condition's `duration_sec` and the
rule-level `on_exit_actions` pipeline. The `resource` trigger keeps its own `duration_sec`
(default 5 s), and the `event` condition keeps a rule-authored `idle_duration_sec`.

Minimal example:

```json
{
  "id": "rule_open_notepad",
  "name": "Open Notepad",
  "trigger": { "$trigger_type": "hotkey", "key_combination": "CTRL+ALT+N" },
  "actions": [
    { "$action_type": "start_process", "file_path": "C:\\Windows\\System32\\notepad.exe" }
  ]
}
```

A rule with a schedule trigger, a beep action and a three-run lifetime quota:

```json
{
  "id": "rule_beep_every_3s",
  "name": "Beep every 3 seconds",
  "trigger": { "$trigger_type": "schedule", "schedule_type": "interval", "interval_sec": 3 },
  "actions": [
    { "$action_type": "play_sound", "play_beep": true, "beep_frequency": 1000, "beep_duration_ms": 200 }
  ],
  "max_executions": 3
}
```

`max_executions` without `quota_period` is a lifetime quota: when it is reached the engine disables
the rule automatically; `--rules-reset <id>` clears the counter and enables it again.

Two field details are easy to get wrong. `start_process.arguments` is split with the Windows
command-line rules (`CommandLineToArgvW`), so a path with spaces must stay quoted and an embedded
quote is escaped as `\"` or `\"\"`. Audio actions address endpoints by display name:
`audio_format.device_name` must match an active endpoint, and `default_device` requires the
AudioDeviceCmdlets PowerShell module — a missing module or device is an action error, not a silent
no-op.

## Process trigger events

A `process` trigger fires the rule's one action pipeline on the lifecycle event named by `event`:

- `"started"` (the default) — the target process launches;
- `"exited"` — the target process closes.

Both choices run the same ordered `actions` pipeline with the same pre-conditions, cooldown and
quota. `single_instance: true` fires the start event for the first matching instance and the exit
event for the last one. `start_process` and `overlay` actions take a `lifetime` that says how long
the process or overlay they create may live:

- `"detach"` (the default) — leave it running or on screen after the run, as before. A detached
  process is not owned and is never stopped; a detached overlay still records its show, so stopping
  the rule hides that exact show (a later show by another rule is left alone);
- `"until_run_end"` — terminate the process or hide that overlay show when the run that created it
  ends, however it ends (finish, failure, abort or stop);
- `"until_trigger_process_exit"` — keep it until the process that triggered the rule exits. This is
  the old companion binding and is only valid for a process-start trigger with a live primary
  process; an exit-triggered rule cannot bind to a process that is already gone. A rule call, manual
  run or test run has no primary process, so the engine treats the value as `until_run_end` there.

The engine records the PID captured at spawn and the generation of the overlay show it started, so
teardown targets exactly that process or show instead of every process with the same name. A rule
whose pipeline ended but still owns a live process or overlay reports running in both the rule list
and `rule_running` / `rule_stopped`; a dead process or a hidden/replaced overlay show is pruned on
the process loop.

The former `close_on_exit` flags are gone: a rule file that still carries one is rejected as an
unknown key, and a share code containing it fails import validation. Use `lifetime` instead.

```json
{
  "id": "rule_game_closed",
  "name": "Clean up after the game",
  "trigger": { "$trigger_type": "process", "process_name": "game.exe", "event": "exited" },
  "actions": [
    { "$action_type": "notification", "title": "Game", "message": "Session ended" }
  ]
}
```

## Quotas

A quota that resets on its own sets `quota_period`:

```json
{
  "id": "rule_ten_runs_per_day",
  "name": "Ten runs per day",
  "trigger": { "$trigger_type": "hotkey", "key_combination": "CTRL+ALT+D" },
  "actions": [
    { "$action_type": "notification", "title": "Daily run", "message": "Quota unit consumed" }
  ],
  "max_executions": 10,
  "quota_period": "day"
}
```

| `quota_period` | Reset point |
|---|---|
| `"minute"`, `"hour"`, `"day"`, `"week"`, `"month"` | Calendar-aligned local time: the next minute, the next hour, midnight, Monday 00:00 (ISO week), the 1st at 00:00 |
| `{"custom": {"value": 90, "unit": "minutes"}}` | Fixed window of `value` `unit`s (`minutes`, `hours`, `days`, `weeks`, `months`); `value` must be between 1 and 100000. It starts with the rule's first execution and resets when the window ends. A `months` window is added as calendar months (e.g. first run on the 19th ends on the 19th; month-end days are clamped) |
| omitted | Lifetime quota |

A periodic quota never disables the rule: when the period is spent the engine skips executions until
the counter rolls over. `--rules-reset <id>` clears it immediately, and `--rules-list` shows the
progress plus the period, e.g. `exec 3/10 (daily)`.

Updating a rule preserves its counter unless the quota definition changes in a way that makes the old
counter meaningless:

- changing `quota_period` (adding, removing or replacing it) starts a fresh counting window and
  keeps `last_executed_at`, which schedule triggers use for deduplication;
- enabling `max_executions` where the rule previously had no limit also starts a fresh window;
- changing only the numeric limit (for example `10` → `20`) keeps the current counter and window
  anchor.

## Calling and stopping other rules

`rule_control` lets one rule drive another one. `operation` is required, `rule_id` names the target
and `wait` only matters for `call`:

```json
{
  "id": "rule_session_start",
  "name": "Start the session",
  "trigger": { "$trigger_type": "hotkey", "key_combination": "CTRL+ALT+S" },
  "actions": [
    { "$action_type": "rule_control", "rule_id": "rule_game_mode", "operation": "call", "wait": true },
    { "$action_type": "notification", "title": "Session", "message": "Game mode ready" }
  ]
}
```

```json
{ "$action_type": "rule_control", "rule_id": "rule_game_mode", "operation": "stop" }
```

- `call` starts the target's pipeline as an ordinary execution. It consumes the target's quota and
  honours its cooldown, conditions and `is_enabled`; a target that is disabled, cooling down, out of
  quota or has unmet conditions is skipped with a log entry and the caller keeps going. A skipped
  call is not an error. Calling a rule that is already running starts a second, parallel execution,
  so `stop` first when a restart is what you mean.
- The called pipeline sees `ProcessName` as `Rule Call (from '<caller>')` and gets the `{CallerRule}`
  and `{CallerRuleId}` tokens described in [SCRIPTING.md](SCRIPTING.md#context-and-tokens).
- `wait: true` pauses the caller until the called pipeline ends or is stopped; `wait: false` (the
  default) continues immediately. A skipped, failed or stopped call never blocks the caller forever.
- `stop` aborts every active execution of the target: running pipelines are cancelled, the overlay
  shows that target made are hidden (only while they are still the active show), its owned processes
  are terminated and simulated input is released. The target stays enabled, so its own triggers keep
  working; nothing has to re-enable it. Stopping a rule that is not running is a logged no-op.
- Direct recursion and cycles are rejected before the target starts: a rule cannot call itself, and a
  call to a rule that is already in the call chain is refused. A chain may contain at most eight
  rules.
- A rule called by another rule is not a manual run: it does not release the target's
  `until_trigger_process_exit` resources when its pipeline ends.

Share codes carry only the rules you export. Overlay button and toggle targets inside a bundled
overlay are rewritten to the final imported rule ids, but `rule_control` actions and rule-activity
conditions are not, so export both rules together to keep those references intact.

### Observing rule activity

Wait and check steps can observe a rule instead of a system event:

```json
{ "$condition_type": "rule_running", "rule_id": "rule_game_mode", "must_be_running": false }
```

```json
{
  "$action_type": "wait",
  "mode": "until_condition",
  "condition": { "type": "rule_started", "rule_id": "rule_game_mode" },
  "timeout_sec": 30,
  "timeout_policy": "abort"
}
```

- `rule_running` is an ordinary condition: it works in `conditions`, `if_else` and `check`.
  `rule_started` and `rule_stopped` are wait conditions.
- "Running" means the target has an action pipeline executing right now **or** still owns a live
  process or overlay from a finished run — the same state a `rule_control` stop ends. A rule whose
  companion has already exited is pruned and reports stopped.
- Waits poll every 100 ms. `rule_started` also completes when the target started at any point since
  the wait began, even if it finished before the next poll; `rule_stopped` completes as soon as the
  target has no running pipeline and no live owned resource. `rule_stopped` passes immediately when
  the target has not started, so it waits for the current run to end, not for the next one to begin.
- An empty `rule_id` never satisfies the condition. A `rule_id` that does not match a configured
  rule is simply never running (`rule_stopped` and `rule_running` with `must_be_running: false`
  pass); `--rules-validate` warns about both.
- A pipeline cannot observe its own stop: while it runs, and while a resource it owns is still
  alive, `rule_started` is true and `rule_stopped` stays false. To keep a rule from running twice,
  gate it with `rule_running` on its own id and `must_be_running: false` in its pre-conditions.

## Webhook trigger

Anything that can make a local HTTP request can start a rule: the daemon listens on
`http://127.0.0.1:48123/api/v1/trigger/<endpoint>`; the port is fixed. The listener exists only
while at least one enabled rule has a `webhook` trigger: disabling the last one or deleting it
closes the port again, and enabling a webhook rule starts it within about a second without
restarting the daemon.

- The tail of the path is matched against enabled rules: the rule `id`, or the `endpoint_path` of a
  `webhook` trigger. A leading slash in `endpoint_path` is tolerated, a trailing slash in the request
  is ignored, and the first matching rule in configuration order wins.
- The request method and body are ignored; only the path matters.
- Matching rule: `200` with `{"status":"triggered","rule":"<rule name>"}`.
- Unknown name or a disabled rule: `404` with
  `{"status":"error","message":"Rule '<target>' not found or disabled"}`.
- Any other path: `400` with
  `{"status":"error","message":"Invalid endpoint. Use /api/v1/trigger/<endpoint_or_rule_id>"}`.

A `200` means a rule was found, not that the pipeline ran: the engine still applies cooldowns and
quotas, so a request can be accepted and then skipped. The server is local-only; while a webhook
rule is enabled and another process owns the port, the bind error is logged and the daemon retries
with a capped backoff (1 s doubling to 30 s), so webhook rules start working once the port is free.

Outbound `http_webhook` actions send the rule's custom `headers` as HTTP headers, not through a
shell; the method and header names/values must be valid or the action fails. A non-2xx response is
an action error.

```powershell
Invoke-RestMethod http://127.0.0.1:48123/api/v1/trigger/backup
```

```bash
curl http://127.0.0.1:48123/api/v1/trigger/backup
```

## Validation and warnings

`--rules-validate` performs structural validation and exits non-zero on errors:

- `id` and `name` must be non-empty, and `id` must be unique within the file;
- `cooldown_seconds` must be finite and non-negative;
- triggers that need a target must have one (process name, hotkey, folder, interval);
- schedule triggers are validated per type: Daily and Monthly need a zero-padded `daily_time`
  (`HH:MM` or `HH:MM:SS`, 00:00–23:59), Monthly needs `day_of_month` 1..=31, OneShot needs a valid
  `target_datetime` (`YYYY-MM-DD HH:MM[:SS]`), Interval needs `interval_sec > 0`, and
  `days_of_week` is 1..=7;
- a time-range condition needs zero-padded `HH:MM` boundaries; an empty boundary stays open;
- an `until_condition` wait needs a condition, wait condition keys may not be empty, and
  `seconds`/`timeout_sec` must be finite, non-negative and at most 86400;
- a resource trigger or condition needs a non-empty `process_name` plus `cpu_percent` or
  `ram_mb`; every configured limit must hold, so with both set a process has to meet both, and a
  CPU-only limit fires on CPU alone;
- a custom quota window needs `value` between 1 and 100000;
- `lifetime: "until_trigger_process_exit"` needs a process trigger firing on `started`;
- tag colours must be six ASCII hex digits, written as `#RRGGBB` (the leading `#` is optional in a
  rule file);
- an infinite macro requires a global-hotkey trigger.

Non-blocking warnings cover rules that are valid but likely not what the author intended: no trigger,
an empty action pipeline, `max_executions: 0`, a `quota_period` without `max_executions`, missing
macro presets or overlays, an empty `start_process` path, a `rule_control` step with an empty target,
a missing target or a call to the rule itself, and a wait or check that observes an empty or unknown
rule.

The Rule Builder runs the same validator before it saves: the errors are listed in the inspector and
the editor stays open, so an invalid draft is never sent. The daemon also runs it before it applies
a change: a rule draft saved from the Rule Builder (`SaveRule`) and a `SyncRules` replace are
rejected with the validation message, and a rejected write leaves the stored rules untouched.
`--rules-sync` also validates the whole desired set before sending it, so a bad rule or a duplicate
id fails without changing the configuration.

## Discovering the schema

The format is self-describing, so an AI agent or a person without the source can author rules:

- `--rules-schema` prints the draft-07 JSON Schema for one rule object. It is generated with
  `schemars` from the same `models::Rule` types the daemon deserializes, so it cannot drift from the
  real format. The schema contains only the current fields; the strict key check rejects anything
  else. Validate generated rules against it with any JSON Schema validator before applying.
- `--rules-reference` prints a compact catalogue of every trigger, action, condition, wait condition
  and macro event with field names, types and required markers.
- `--rules-template <trigger>[:<action>]` prints a minimal valid skeleton with the required fields
  already filled in. The action defaults to `notification`; run the command without a value to list
  every available trigger and action.

```powershell
cmd /c "Synector.exe --rules-schema > schema.json"
cmd /c "Synector.exe --rules-template schedule:play_sound > rule.json"
# edit rule.json, then:
cmd /c "Synector.exe --rules-validate rule.json"
cmd /c "Synector.exe --rules-apply rule.json"
```

`cmd /c` is required here because release builds are GUI-subsystem executables: PowerShell would
neither wait for the process nor capture its output or exit code. See [Console notes](#console-notes).

## Config as code

`--rules-apply` only upserts the rules present in the file: rules missing from it stay in the
configuration. `--rules-sync` is the declarative variant and makes the configured rules match the
file exactly, deleting rules that are absent. An empty array (`[]` or `{"rules": []}`) is the exact
"no rules" file and deletes every configured rule, while `--rules-apply` and `--rules-validate`
still reject an empty set. While the daemon is running a sync is applied as a single replace:
surviving rules keep their runtime counters and hotkeys are reloaded once. Use `--dry-run` to
preview the result and `--json` for a machine-readable report:

```powershell
cmd /c "Synector.exe --rules-sync rules.json --dry-run --json"
cmd /c "Synector.exe --rules-sync rules.json --json"
```

An apply/sync JSON report has the shape
`{"ok": true, "target": "daemon"|"rules", "dry_run": false, "created": [...], "updated": [...],
"deleted": [...], "warnings": [...]}`. Every `--json` failure — validation, read, list/export,
import and IPC errors alike — uses one envelope
`{"ok": false, "error": "...", "errors": [...], "warnings": [...], "target": "daemon"|"rules"|null}`
and exits with a non-zero status. `--rules-list --json` prints the rule array itself; toggle and
reset commands return a small object with `ok` plus the affected rule ids, counts or counters.

## Share codes

`--rules-share [<id>]` prints one compact `SYN1:` code. With no selector it exports every configured
rule; with an id or rule name it exports that rule only. The code bundles the macro presets and
overlays the exported rules reference, resets sender-local runtime counters and drops custom icon,
overlay image and button icon paths (with a warning). `--json` returns
`{"ok": true, "rules": N, "bytes": N, "code": "SYN1:...", "warnings": [...]}`.

`--rules-import-share <code|file|->` decodes a code from the argument, a file or stdin, verifies its
SHA-256 checksum and bounded payload, and imports the rules plus the library entries they need. A
literal argument must start with `SYN1:`; any other non-`-` argument is read as a file path, and a
missing path is reported as a read error. Duplicates are skipped by content, colliding ids are
re-identified, and imported rules are created disabled unless `--enable-imported` is passed. Library
entries whose content already exists locally are reused instead of duplicated, so importing the
same code twice adds nothing; bundled overlay button and toggle targets are re-pointed at the final
rule ids, and a target that neither the code nor the local rules provide is reported as a warning.
One code carries at most 500 rules, 500 overlays and 500 macro presets. With a running daemon the
whole import is one `ImportShare` request; otherwise `library.json` is committed before `rules.json`
so an imported rule never references a missing preset or overlay.

## Ready-to-use recipes

See [EXAMPLES.md](EXAMPLES.md) for complete rules: owned companion processes with lifetimes, hotkey actions, webhooks
with quotas, file watchers and archives, weekday schedules, If/Else branching, rule chaining with
`rule_control`, sleep/resume audio toggles, and launch-wait-tune pipelines.

## Console notes

Release builds are Windows GUI-subsystem executables, so PowerShell does not wait for them and
cannot capture their output or exit code. Prefix CLI commands with `cmd /c` (or use
`Start-Process -Wait`) when scripting from PowerShell. Tools that spawn the process with piped
stdio and wait for it — Python `subprocess.run`, Node `child_process`, most agent runtimes — work
directly.

Reading from stdin works the same way as from a file:

```powershell
cmd /c "type rule.json | Synector.exe --rules-apply -"
```
