# Lua scripting

Synector embeds Lua 5.4.9 for rules that need more than a fixed action pipeline. A script runs as the
`execute_script` action with `"interpreter": "lua"`. Every execution starts a fresh interpreter, so
globals do not leak between runs and there is no state to clean up.

Rule format, quotas and the action pipeline are documented in
[RULE_AUTHORING.md](RULE_AUTHORING.md); this file covers the script side only.

## Running a script

```json
{
  "id": "rule_lua_greeting",
  "name": "Greet the process",
  "trigger": { "$trigger_type": "manual" },
  "actions": [
    {
      "$action_type": "execute_script",
      "interpreter": "lua",
      "script": "local name = ph.get(\"ProcessName\")\nif name == \"\" then\n  name = \"manual run\"\nend\nph.log(\"process: \" .. name)\nreturn \"hello from \" .. name"
    },
    {
      "$action_type": "notification",
      "title": "Lua",
      "message": "{PrevOutput}"
    }
  ]
}
```

The second step shows what the first one returned: a script's return value becomes the run output.

## Context and tokens

Before the chunk is evaluated, Synector substitutes `{...}` tokens in the script text with values from
the run context — the same tokens other actions accept. The context always has `ProcessName`,
`ProcessId` and `Timestamp`; process and window triggers add `ProcessPath`, `ProcessDir` and
`WindowTitle` when they are known. `{Clipboard}`, `{Date}`, `{Time}` and `{Env:NAME}` are resolved too.

Substitution works on the source text, so a `{...}` fragment inside a string literal is replaced as
well. Quote values explicitly when they may contain spaces or quotes.

## API

The `ph` table is the only global Synector adds.

| Function | Returns | Notes |
|---|---|---|
| `ph.context` | table | Copy of the run context; keys as described above |
| `ph.get(key)` | string | `""` when the key is missing |
| `ph.set(key, value)` | — | Writes to a run-local table; return a value instead (see Output) |
| `ph.log(text)` | — | Writes `[LUA] text` to the daemon's standard output |
| `ph.run_cmd(command)` | string | Runs `cmd /C <command>`, returns stdout; a failure comes back as an `Error: ...` string instead of raising |
| `ph.overlay_show(id)` | — | Shows a saved overlay |
| `ph.overlay_hide(id)` | — | Hides it |
| `ph.overlay_toggle(id)` | — | Shows or hides it |
| `ph.overlay_set_text(id, text)` | — | Replaces the runtime text of an overlay |

Overlay lookups accept the overlay id or its name (case-insensitive). When `overlay_show` or
`overlay_toggle` gets an unknown id, it falls back to a default overlay instead of failing.

## Output

The value the chunk returns becomes the action output: strings are used as is, integers and floats are
converted, booleans become `true`/`false`, and anything else (tables, functions, `nil`) counts as
empty. The engine stores a non-empty result in the run context as `PrevOutput` and `StepN.Output`
(`Step1.Output` for the first step), so later actions can use `{PrevOutput}` or `{Step1.Output}`.
`ph.set` writes into a run-local table that nothing reads back today — return the value instead.

## Errors

A Lua error fails the step with `Lua runtime error: ...`; whether the pipeline continues depends on
the rule's `stop_on_error`. Action errors may quote values from the script, so the engine sends them
through its sensitive log channel: the details appear in the in-app log stream, while the log file
records only that the step failed.

## Security

The interpreter is not a sandbox: `ph.run_cmd` runs through `cmd /C` with the daemon's rights, which
are administrator rights unless Synector was started with `--no-elevate`. Run only rules and share
codes you trust, and keep secrets out of `ph.log` — it writes to standard output rather than the log
file.
