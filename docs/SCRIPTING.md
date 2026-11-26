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
      "script": "local name = syn.get(\"ProcessName\")\nif name == \"\" then\n  name = \"manual run\"\nend\nsyn.log(\"process: \" .. name)\nreturn \"hello from \" .. name"
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
`WindowTitle` when they are known. A pipeline started by `rule_control` adds `CallerRule` and
`CallerRuleId` and reports `ProcessName` as `Rule Call (from '<caller>')`. `{Clipboard}`, `{Date}`,
`{Time}` and `{Env:NAME}` are resolved too.

Substitution works on the source text, so a `{...}` fragment inside a string literal is replaced as
well. Quote values explicitly when they may contain spaces or quotes.

## API

The `syn` table is the only global Synector adds.

| Function | Returns | Notes |
|---|---|---|
| `syn.context` | table | Copy of the run context; keys as described above |
| `syn.get(key)` | string | `""` when the key is missing |
| `syn.set(key, value)` | — | Writes to a run-local table; return a value instead (see Output) |
| `syn.log(text)` | — | Writes `[LUA] text` to the daemon's standard output |
| `syn.run_cmd(command)` | string | Runs `cmd /C <command>` and returns its stdout. A non-zero exit, spawn failure, timeout or a stop of the running manual step test comes back as an `Error: ...` string instead of raising, so check the prefix; a non-zero exit also carries the trimmed stderr when there is one |
| `syn.overlay_show(id)` | — | Shows a saved overlay |
| `syn.overlay_hide(id)` | — | Hides it |
| `syn.overlay_toggle(id)` | — | Shows or hides it |
| `syn.overlay_set_text(id, text)` | — | Replaces the runtime text of an overlay |

Overlay lookups accept the overlay id or its name (case-insensitive). When `overlay_show` or
`overlay_toggle` gets an unknown id, it falls back to a default overlay instead of failing.

## Limits

Every execution is bounded so a runaway script cannot pin a daemon worker: 5,000,000 Lua
instructions and 3 seconds of wall-clock time enforced by the VM hook, and a 64 MiB Lua heap. The
hook is reached only between Lua instructions, so a native call such as `syn.run_cmd` is not
interrupted by it and is bounded separately by its own 5 s deadline; stopping the running manual
step test also terminates that call's process tree and returns `Error: command stopped`. One
execution can therefore hold a daemon worker for roughly the 3 s budget plus such a call. A returned
string longer than 64 KiB fails the step. `syn.set` stores at most 64 output variables, 256-byte
keys and 16 KiB values; exceeding a cap raises a Lua error.

`syn.run_cmd` captures at most 64 KiB of stdout — cut output ends with `[output truncated]` — and
2 KiB of stderr, and terminates the whole process tree after 5 seconds (`Error: command timed out
after 5s`) through a Windows Job Object, with `taskkill /T` as the fallback. A non-zero exit returns
`Error: exit code <code>`, followed by the trimmed stderr when it is not empty.

## Output

The value the chunk returns becomes the action output: strings are used as is, integers and floats are
converted, booleans become `true`/`false`, and anything else (tables, functions, `nil`) counts as
empty. The engine stores a non-empty result in the run context as `PrevOutput` and `StepN.Output`
(`Step1.Output` for the first step), so later actions can use `{PrevOutput}` or `{Step1.Output}`.
An If/Else step passes on the output of the last output-bearing action in the branch it took, so a
script nested in a branch feeds the parent pipeline's tokens like a top-level step. `syn.set` writes
into a run-local table that nothing reads back today — return the value instead.

## Errors

A Lua error fails the step with `Lua runtime error: ...`; whether the pipeline continues depends on
the rule's `stop_on_error`. Action errors may quote values from the script, so the engine sends them
through its sensitive log channel: the details appear in the in-app log stream, while the log file
records only that the step failed.

## Security

The interpreter is not a sandbox: `syn.run_cmd` runs through `cmd /C` with the daemon's rights, which
are administrator rights unless Synector was started with `--no-elevate`. Run only rules and share
codes you trust, and keep secrets out of `syn.log` — it writes to standard output rather than the log
file.
