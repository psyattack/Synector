# Rule examples

Complete rules you can validate and apply without opening the GUI. The JSON format, field semantics
and quota rules are documented in [RULE_AUTHORING.md](RULE_AUTHORING.md); this file is a recipe
book. Replace paths and process names with values from your own machine.

Every example validates with:

```powershell
cmd /c "Synector.exe --rules-validate rule.json"
cmd /c "Synector.exe --rules-apply rule.json"
```

Use `--rules-sync` instead of `--rules-apply` when the file should become the complete rule set.
`--dry-run` previews the result.

## Companion process bound to a game

Starts a helper tool with the game and closes it when the game exits. `single_instance: true`
makes the rule fire only for the first matching instance and defers cleanup until every tracked
instance has exited.

```json
{
  "id": "rule_game_companion",
  "name": "Start the companion with the game",
  "description": "Runs a helper HUD while the game is open and closes it with the game.",
  "trigger": {
    "$trigger_type": "process",
    "process_name": "game.exe",
    "match_mode": "exact",
    "single_instance": true
  },
  "actions": [
    {
      "$action_type": "start_process",
      "file_path": "C:\\Tools\\CompanionHud.exe",
      "arguments": "--attach game.exe",
      "working_directory": "C:\\Tools",
      "close_on_exit": true
    }
  ]
}
```

Apply it with `cmd /c "Synector.exe --rules-apply rule.json"` after saving the block as `rule.json`.

## Run cleanup when a game exits

A process trigger fires the same action pipeline on the lifecycle event selected by `event`. With
`"exited"` the rule runs after the game closes; pre-conditions, cooldown and quota apply exactly as
they do on launch.

```json
{
  "id": "rule_game_closed_cleanup",
  "name": "Clean up after the game",
  "trigger": {
    "$trigger_type": "process",
    "process_name": "game.exe",
    "match_mode": "exact",
    "event": "exited"
  },
  "actions": [
    {
      "$action_type": "notification",
      "title": "Game closed",
      "message": "Session ended"
    }
  ]
}
```

## Free memory on a hotkey

Trims the working set of one process. A cooldown prevents repeated trimming while the hotkey is
held.

```json
{
  "id": "rule_trim_browser_memory",
  "name": "Trim browser working set",
  "trigger": {
    "$trigger_type": "hotkey",
    "key_combination": "CTRL+ALT+R"
  },
  "actions": [
    {
      "$action_type": "process_tuning",
      "target_process": "chrome.exe",
      "trim_memory": true
    }
  ],
  "cooldown_seconds": 5
}
```

Apply it with `cmd /c "Synector.exe --rules-apply rule.json"` after saving the block as `rule.json`.

## Webhook trigger with an hourly quota

Anything on the machine can call
`http://127.0.0.1:48123/api/v1/trigger/backup`. The quota allows five runs per
clock hour and then skips requests until the hour rolls over; the rule stays enabled.

```json
{
  "id": "rule_webhook_backup",
  "name": "Backup on webhook",
  "trigger": {
    "$trigger_type": "webhook",
    "endpoint_path": "backup"
  },
  "actions": [
    {
      "$action_type": "notification",
      "title": "Backup",
      "message": "Webhook received, starting the backup."
    },
    {
      "$action_type": "execute_script",
      "interpreter": "power_shell",
      "script": "Write-Output 'backup placeholder'"
    }
  ],
  "max_executions": 5,
  "quota_period": "hour"
}
```

Apply it with `cmd /c "Synector.exe --rules-apply rule.json"` after saving the block as `rule.json`.

Replace the placeholder script with your own command. Script bodies are never written to the log.

## Archive a folder when a new file arrives

Watches an inbox folder and packs it into a ZIP. The cooldown coalesces bursts of file events into
one archive operation.

```json
{
  "id": "rule_archive_logs",
  "name": "Archive new logs",
  "trigger": {
    "$trigger_type": "file_watcher",
    "folder_path": "C:\\Logs\\Inbox",
    "filter_extension": "log",
    "on_created": true
  },
  "actions": [
    {
      "$action_type": "archive",
      "operation": "pack",
      "format": "zip",
      "source_path": "C:\\Logs\\Inbox",
      "destination_path": "C:\\Logs\\Archive\\logs.zip",
      "overwrite": true
    }
  ],
  "cooldown_seconds": 30
}
```

Apply it with `cmd /c "Synector.exe --rules-apply rule.json"` after saving the block as `rule.json`.

## Switch power plans on a weekday schedule

Two rules in one file. `--rules-apply` accepts an array, a single object, or `{"rules": [...]}`.

```json
[
  {
    "id": "rule_power_high_morning",
    "name": "High performance on weekday mornings",
    "trigger": {
      "$trigger_type": "schedule",
      "schedule_type": "daily",
      "daily_time": "09:00:00",
      "days_of_week": [1, 2, 3, 4, 5]
    },
    "actions": [
      {
        "$action_type": "power_plan",
        "plan": "high_performance"
      }
    ]
  },
  {
    "id": "rule_power_balanced_evening",
    "name": "Balanced power in the evening",
    "trigger": {
      "$trigger_type": "schedule",
      "schedule_type": "daily",
      "daily_time": "19:00:00",
      "days_of_week": [1, 2, 3, 4, 5]
    },
    "actions": [
      {
        "$action_type": "power_plan",
        "plan": "balanced"
      }
    ]
  }
]
```

Apply it with `cmd /c "Synector.exe --rules-apply rule.json"` after saving the block as `rule.json`.

`days_of_week` uses `1` = Monday through `7` = Sunday.

## Branch on a condition

One hotkey either restores an existing window or starts the application. Conditions use the
`$condition_type` tag; the same condition type is valid in a top-level `conditions` list.

```json
{
  "id": "rule_focus_or_start_notepad",
  "name": "Focus or start Notepad",
  "trigger": {
    "$trigger_type": "hotkey",
    "key_combination": "CTRL+ALT+N"
  },
  "actions": [
    {
      "$action_type": "if_else",
      "conditions": [
        {
          "$condition_type": "process_running",
          "process_name": "notepad.exe",
          "must_be_running": true
        }
      ],
      "then_actions": [
        {
          "$action_type": "window",
          "target_process": "notepad.exe",
          "window_action": "restore"
        }
      ],
      "else_actions": [
        {
          "$action_type": "start_process",
          "file_path": "C:\\Windows\\System32\\notepad.exe"
        }
      ]
    }
  ]
}
```

Apply it with `cmd /c "Synector.exe --rules-apply rule.json"` after saving the block as `rule.json`.

## Chain rules: call and stop

A rule without a trigger is a reusable pipeline other rules can run. `wait: true` keeps the caller
paused until the called pipeline finishes, which is what makes the notification in the next example
honest.

```json
[
  {
    "id": "rule_prepare_workspace",
    "name": "Prepare the workspace",
    "description": "Library rule: called by the start rule, no trigger of its own.",
    "actions": [
      { "$action_type": "power_plan", "plan": "high_performance" },
      { "$action_type": "start_process", "file_path": "C:\\Windows\\System32\\notepad.exe" },
      { "$action_type": "start_process", "file_path": "C:\\Windows\\System32\\calc.exe" }
    ]
  },
  {
    "id": "rule_start_workspace",
    "name": "Start the workspace",
    "trigger": { "$trigger_type": "hotkey", "key_combination": "CTRL+ALT+W" },
    "actions": [
      {
        "$action_type": "rule_control",
        "rule_id": "rule_prepare_workspace",
        "operation": "call",
        "wait": true
      },
      { "$action_type": "notification", "title": "Workspace", "message": "Ready" }
    ]
  }
]
```

`stop` aborts what the target is running right now without disabling it. Here a hotkey ends a gaming
companion that is bound to `game.exe`, while the game itself keeps running:

```json
[
  {
    "id": "rule_game_companion",
    "name": "Helper HUD with the game",
    "trigger": { "$trigger_type": "process", "process_name": "game.exe" },
    "actions": [
      { "$action_type": "start_process", "file_path": "C:\\Tools\\hud.exe", "close_on_exit": true }
    ]
  },
  {
    "id": "rule_end_session",
    "name": "End the session",
    "trigger": { "$trigger_type": "hotkey", "key_combination": "CTRL+ALT+E" },
    "actions": [
      { "$action_type": "rule_control", "rule_id": "rule_game_companion", "operation": "stop" },
      { "$action_type": "notification", "title": "Session", "message": "Companion stopped" }
    ]
  }
]
```

A call that the target rejects (disabled, cooling down, out of quota or with unmet conditions) is
logged and does not fail the caller. Direct self-calls and cycles are refused, so apply both rules
of a pair together.

A rule's pipeline can also be observed without calling it. `rule_started` and `rule_stopped` are wait
conditions; "running" means the target's action pipeline is executing right now, not that a companion
process or overlay it launched is still alive. Here a watcher waits for a long-running mode to begin
and then for it to end:

```json
[
  {
    "id": "rule_night_shift",
    "name": "Night shift",
    "description": "Long-running mode that another rule observes.",
    "trigger": { "$trigger_type": "hotkey", "key_combination": "CTRL+ALT+N" },
    "actions": [
      { "$action_type": "power_plan", "plan": "power_saver" },
      { "$action_type": "wait", "mode": "timer", "seconds": 3600 }
    ]
  },
  {
    "id": "rule_night_shift_watcher",
    "name": "Announce the end of the night shift",
    "trigger": { "$trigger_type": "hotkey", "key_combination": "CTRL+ALT+W" },
    "actions": [
      {
        "$action_type": "wait",
        "mode": "until_condition",
        "condition": { "type": "rule_started", "rule_id": "rule_night_shift" },
        "timeout_sec": 600,
        "timeout_policy": "abort"
      },
      {
        "$action_type": "wait",
        "mode": "until_condition",
        "condition": { "type": "rule_stopped", "rule_id": "rule_night_shift" },
        "timeout_sec": 0,
        "timeout_policy": "abort"
      },
      { "$action_type": "notification", "title": "Shift", "message": "Night shift is over" }
    ]
  }
]
```

`rule_running` is the same idea as a regular condition and fits pre-conditions, `if_else` and
`check`. This pre-condition keeps a maintenance rule from overlapping with another rule:

```json
{
  "$condition_type": "rule_running",
  "rule_id": "rule_backup",
  "must_be_running": false
}
```

## Mute before sleep and unmute after resume

Sleep, resume, display state and device changes are pushed by Windows through a hidden notification
window, so these rules fire without polling.

```json
[
  {
    "id": "rule_mute_on_sleep",
    "name": "Mute playback before sleep",
    "trigger": {
      "$trigger_type": "system",
      "event_type": "system_sleep"
    },
    "actions": [
      {
        "$action_type": "audio_control",
        "target": "playback",
        "action_type": "mute"
      }
    ]
  },
  {
    "id": "rule_unmute_on_resume",
    "name": "Unmute playback after resume",
    "trigger": {
      "$trigger_type": "system",
      "event_type": "system_resume"
    },
    "actions": [
      {
        "$action_type": "audio_control",
        "target": "playback",
        "action_type": "unmute"
      }
    ]
  }
]
```

Apply it with `cmd /c "Synector.exe --rules-apply rule.json"` after saving the block as `rule.json`.

## Launch, wait for the process, then tune it

The wait step blocks only this execution. `timeout_policy: "abort"` stops the remaining pipeline if
the process does not appear within 60 seconds; use `"continue"` to run the following steps anyway.

```json
{
  "id": "rule_launch_game_priority",
  "name": "Launch the game with high priority",
  "trigger": {
    "$trigger_type": "hotkey",
    "key_combination": "CTRL+ALT+G"
  },
  "actions": [
    {
      "$action_type": "start_process",
      "file_path": "D:\\Games\\Example\\game.exe",
      "working_directory": "D:\\Games\\Example"
    },
    {
      "$action_type": "wait",
      "mode": "until_condition",
      "condition": {
        "type": "process_started",
        "process_name": "game.exe"
      },
      "timeout_sec": 60,
      "timeout_policy": "abort"
    },
    {
      "$action_type": "process_tuning",
      "target_process": "game.exe",
      "priority": "high"
    }
  ]
}
```

Apply it with `cmd /c "Synector.exe --rules-apply rule.json"` after saving the block as `rule.json`.

## More patterns

- A **toggle hotkey** uses `hold_mode: true` to repeat the pipeline while held, or
  `repeat_while_held: true` without hold mode to loop until pressed again.
- A **process trigger** fires its one action pipeline on `event: "started"` (the default) or
  `event: "exited"`; `single_instance` makes the start run fire for the first instance and the exit
  run for the last one. `close_on_exit` companions bind only to a start-triggered run.
- **Runtime counters** (`execution_count`, `last_executed_at`, the custom quota anchor) are owned by
  `runtime.json`; values in a rule file are ignored when an existing rule is updated.
- **Share codes** (`SYN1:`) bundle the presets and overlays a rule references; imported rules are
  created disabled unless `--enable-imported` is passed.
- **Quoted process arguments** — `start_process.arguments` is split with the Windows command-line
  rules, so `-m "C:\\Program Files\\App\\mod.ini"` stays one argument. Quote paths with spaces and
  escape an embedded quote as `\"` or `\"\"`.
- **Rule control** (`rule_control`) chains rules: `operation: "call"` runs another pipeline,
  `operation: "stop"` aborts its executions without disabling it. Share codes do not rewrite these
  references, so export a calling rule and its target together.
- **Rule activity** (`rule_running`, wait `rule_started`/`rule_stopped`) observes another rule's
  pipeline; it reports stopped once the pipeline ends, even if a bound companion process is still
  alive.
