# Rule authoring from the command line

Synector rules are plain JSON files. The CLI validates, applies, exports and maintains them without
opening the GUI. While the daemon is running changes go through IPC and take effect immediately; when
it is not, the same upsert is written to `%LOCALAPPDATA%\Synector\config.json` and applied on the
next daemon start.

## Commands

| Command | Description |
|---|---|
| `--rules-validate <file\|->` | Parse and validate a rule file; no changes are made |
| `--rules-apply <file\|->` | Create or update rules from a file (upsert by `id`) |
| `--rules-sync <file\|->` | Make the config match the file exactly (upsert + delete missing) |
| `--rules-list [--json]` | List rules with state, execution counter and trigger |
| `--rules-export [<file>]` | Export all rules as JSON |
| `--rules-delete <id>` | Delete one rule |
| `--rules-enable <id>`, `--rules-disable <id>` | Toggle one rule |
| `--rules-reset <id>` | Clear `execution_count`/`last_executed_at` and re-enable |
| `--rules-example` | Print an example rule file |
| `--rules-schema` | Print the draft-07 JSON Schema for one rule |
| `--rules-reference` | Print a human-readable catalogue of triggers/actions/conditions |
| `--rules-template <trigger>[:<action>]` | Print a ready-to-edit rule skeleton |
| `--help` | Print the command reference |

`--json` switches validate/apply/sync/delete/enable/disable/reset reports to machine-readable JSON.
`--dry-run` previews `--rules-apply`/`--rules-sync` without writing anything. A path of `-` reads the
rule file from stdin.

## File format

A rule file is one rule object, an array of rules, or `{"rules": [...]}`. `id` is the upsert key and
must be unique and stable, `name` is required. Applying a file never resets the daemon-owned runtime
counters (`execution_count`, `last_executed_at`) of rules it updates, so editing a rule file does not
reset its quota progress. `--rules-schema` describes a single rule object; the array and wrapper
forms are only file containers.

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

A rule with a schedule trigger, a beep action and a three-run quota:

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

When `max_executions` is reached the engine disables the rule automatically; `--rules-reset <id>`
clears the counter and enables it again.

## Discovering the schema

The format is self-describing, so an AI agent or a person without the source can author rules:

- `--rules-schema` prints the draft-07 JSON Schema for one rule. It is generated with `schemars` from
  the same `models::Rule` types the daemon deserializes, so it cannot drift from the real format.
  Validate generated rules against it with any JSON Schema validator before applying them.
- `--rules-reference` prints a compact catalogue of every trigger, action, condition, wait condition
  and macro event with field names, types and required markers.
- `--rules-template <trigger>[:<action>]` prints a minimal valid skeleton with the required fields
  already filled in. The action defaults to `notification`; run the command without a value to list
  every available trigger and action.

```powershell
Synector.exe --rules-schema > schema.json
Synector.exe --rules-template schedule:play_sound > rule.json
# edit rule.json, then:
Synector.exe --rules-validate rule.json
Synector.exe --rules-apply rule.json
```

## Config as code

`--rules-apply` only upserts the rules present in the file: rules missing from it stay in the
configuration. `--rules-sync` is the declarative variant and makes the configuration match the file
exactly, deleting rules that are absent. Use `--dry-run` to preview the result and `--json` for a
machine-readable report:

```powershell
Synector.exe --rules-sync rules.json --dry-run --json
Synector.exe --rules-sync rules.json --json
```

A JSON report has the shape `{"ok": true, "target": "daemon"|"config", "dry_run": false,
"created": [...], "updated": [...], "deleted": [...], "warnings": [...]}`. Failures report
`{"ok": false, "error": "...", "errors": ["..."]}` and exit with a non-zero status.

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
