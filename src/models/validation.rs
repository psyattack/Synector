/// Longest wait a rule may express, in seconds. A day-long pipeline is already an authoring
/// mistake, and an unbounded `seconds` is what makes `Duration::from_secs_f64` panic.
pub const MAX_WAIT_SECONDS: f64 = 86_400.0;

/// Fallible conversion of a rule-authored second count into a [`std::time::Duration`].
///
/// A rule file is untrusted input and `Duration::from_secs_f64` panics on negative,
/// non-finite and huge values. Every runtime use of `seconds`/`timeout_sec` goes through
/// this helper so a hand-edited rule becomes a normal action error instead of a dead task.
pub fn duration_from_secs(seconds: f64) -> Result<std::time::Duration, String> {
    if !seconds.is_finite() {
        return Err("duration must be a finite number of seconds".to_string());
    }
    if seconds < 0.0 {
        return Err("duration must not be negative".to_string());
    }
    if seconds > MAX_WAIT_SECONDS {
        return Err(format!(
            "duration must not exceed {MAX_WAIT_SECONDS:.0} seconds"
        ));
    }
    Ok(std::time::Duration::from_secs_f64(seconds))
}

fn parse_two_digits(part: &str) -> Option<u8> {
    if part.len() != 2 || !part.as_bytes().iter().all(u8::is_ascii_digit) {
        return None;
    }
    part.parse().ok()
}

/// True for a zero-padded `HH:MM` between 00:00 and 23:59.
pub fn is_valid_hh_mm(value: &str) -> bool {
    let Some((hour, minute)) = value.split_once(':') else {
        return false;
    };
    matches!(
        (parse_two_digits(hour), parse_two_digits(minute)),
        (Some(hour), Some(minute)) if hour < 24 && minute < 60
    )
}

/// True for a zero-padded `HH:MM` or `HH:MM:SS` between 00:00:00 and 23:59:59.
pub fn is_valid_hh_mm_ss(value: &str) -> bool {
    let parts: Vec<&str> = value.split(':').collect();
    match parts.as_slice() {
        [hour, minute] => matches!(
            (parse_two_digits(hour), parse_two_digits(minute)),
            (Some(hour), Some(minute)) if hour < 24 && minute < 60
        ),
        [hour, minute, second] => matches!(
            (parse_two_digits(hour), parse_two_digits(minute), parse_two_digits(second)),
            (Some(hour), Some(minute), Some(second)) if hour < 24 && minute < 60 && second < 60
        ),
        _ => false,
    }
}

/// True for a local datetime in the wire format `YYYY-MM-DD HH:MM[:SS]`.
pub fn is_valid_target_datetime(value: &str) -> bool {
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").is_ok()
        || chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").is_ok()
}

/// True when every weekday number is in the ISO range 1 (Monday) ..= 7 (Sunday).
pub fn days_of_week_are_valid(days: &[u8]) -> bool {
    days.iter().all(|day| (1..=7).contains(day))
}

/// True when a monthly schedule day is in 1 ..= 31.
pub fn is_valid_day_of_month(day: u8) -> bool {
    (1..=31).contains(&day)
}

/// Shared value checks for the resource trigger and the resource condition. Both need a
/// process and at least one threshold; without one they would silently degrade to
/// "process is running", which `process_running` already expresses.
pub fn resource_threshold_errors(
    process_name: &str,
    cpu_percent: Option<f32>,
    ram_mb: Option<u64>,
) -> Vec<&'static str> {
    let mut errors = Vec::new();
    if process_name.trim().is_empty() {
        errors.push("resource threshold needs a non-empty process_name");
    }
    if cpu_percent.is_none() && ram_mb.is_none() {
        errors.push("resource threshold needs cpu_percent or ram_mb");
    }
    if let Some(cpu) = cpu_percent {
        if !cpu.is_finite() || cpu < 0.0 {
            errors.push("cpu_percent must be a finite non-negative number");
        }
    }
    if ram_mb == Some(0) {
        errors.push("ram_mb must be greater than 0");
    }
    errors
}

// ---------------------------------------------------------------------------
// Strict JSON key checking
// ---------------------------------------------------------------------------

/// One key in a rule document that is not part of the rule model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownRuleKey {
    /// JSON path of the object carrying the key, for example `rule.actions[0]`.
    pub path: String,
    /// The key itself.
    pub key: String,
}

impl std::fmt::Display for UnknownRuleKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown key \"{}\" in {}", self.key, self.path)
    }
}

/// Rejects unknown keys in one rule object before it is deserialized.
///
/// Serde silently drops unknown keys, so a typo such as `max_execution` used to apply and
/// run with the default. This pass walks the raw JSON against the same key sets the types
/// declare and reports every unknown key with its path. Callers decide the policy:
/// `--rules-apply` and share import refuse the rule, the daemon load logs and skips it.
pub fn validate_rule_keys(value: &serde_json::Value) -> Result<(), Vec<UnknownRuleKey>> {
    let mut errors = Vec::new();
    check_rule_keys(value, "rule", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn check_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
    path: &str,
    errors: &mut Vec<UnknownRuleKey>,
) {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            errors.push(UnknownRuleKey {
                path: path.to_string(),
                key: key.clone(),
            });
        }
    }
}

fn variant_keys<'a>(
    tag: &str,
    table: &'a [(&'static str, &'static [&'static str])],
) -> Option<&'a [&'static str]> {
    table
        .iter()
        .find(|(name, _)| *name == tag)
        .map(|(_, keys)| *keys)
}

fn check_rule_keys(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    check_keys(object, RULE_KEYS, path, errors);
    if let Some(trigger) = object.get("trigger") {
        check_trigger_keys(trigger, &format!("{path}.trigger"), errors);
    }
    if let Some(conditions) = object.get("conditions") {
        check_condition_items(conditions, &format!("{path}.conditions"), errors);
    }
    for field in ["actions", "on_exit_actions"] {
        if let Some(actions) = object.get(field) {
            check_action_list(actions, &format!("{path}.{field}"), errors);
        }
    }
    if let Some(tags) = object.get("tags").and_then(serde_json::Value::as_array) {
        for (index, tag) in tags.iter().enumerate() {
            let Some(tag) = tag.as_object() else { continue };
            check_keys(tag, TAG_KEYS, &format!("{path}.tags[{index}]"), errors);
        }
    }
    if let Some(period) = object.get("quota_period") {
        check_quota_period(period, &format!("{path}.quota_period"), errors);
    }
}

fn check_quota_period(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    check_keys(object, QUOTA_PERIOD_KEYS, path, errors);
    if let Some(custom) = object.get("custom").and_then(serde_json::Value::as_object) {
        check_keys(custom, CUSTOM_QUOTA_KEYS, &format!("{path}.custom"), errors);
    }
}

fn check_trigger_keys(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    let Some(tag) = object.get("$trigger_type").and_then(serde_json::Value::as_str) else {
        return;
    };
    if let Some(allowed) = variant_keys(tag, TRIGGER_VARIANTS) {
        check_keys(object, allowed, path, errors);
    }
}

fn check_condition_items(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(items) = value.as_array() else {
        return;
    };
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let Some(object) = item.as_object() else {
            continue;
        };
        if object.contains_key("condition") {
            check_keys(object, CONDITION_ITEM_KEYS, &item_path, errors);
            if let Some(condition) = object.get("condition") {
                check_condition_keys(condition, &format!("{item_path}.condition"), errors);
            }
        } else {
            check_condition_keys(item, &item_path, errors);
        }
    }
}

fn check_condition_keys(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    let Some(tag) = object.get("$condition_type").and_then(serde_json::Value::as_str) else {
        return;
    };
    if let Some(allowed) = variant_keys(tag, CONDITION_VARIANTS) {
        check_keys(object, allowed, path, errors);
    }
}

fn check_action_list(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(actions) = value.as_array() else {
        return;
    };
    for (index, action) in actions.iter().enumerate() {
        check_action(action, &format!("{path}[{index}]"), errors);
    }
}

fn check_action(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    let Some(tag) = object.get("$action_type").and_then(serde_json::Value::as_str) else {
        return;
    };
    if let Some(allowed) = variant_keys(tag, ACTION_VARIANTS) {
        check_keys(object, allowed, path, errors);
    }
    match tag {
        "if_else" => {
            if let Some(items) = object.get("conditions") {
                check_condition_items(items, &format!("{path}.conditions"), errors);
            }
            if let Some(items) = object.get("then_actions") {
                check_action_list(items, &format!("{path}.then_actions"), errors);
            }
            if let Some(branches) = object
                .get("else_if_branches")
                .and_then(serde_json::Value::as_array)
            {
                for (index, branch) in branches.iter().enumerate() {
                    let branch_path = format!("{path}.else_if_branches[{index}]");
                    let Some(branch) = branch.as_object() else {
                        continue;
                    };
                    check_keys(branch, ELSE_IF_KEYS, &branch_path, errors);
                    if let Some(items) = branch.get("conditions") {
                        check_condition_items(items, &format!("{branch_path}.conditions"), errors);
                    }
                    if let Some(items) = branch.get("actions") {
                        check_action_list(items, &format!("{branch_path}.actions"), errors);
                    }
                }
            }
            if let Some(items) = object.get("else_actions") {
                check_action_list(items, &format!("{path}.else_actions"), errors);
            }
        }
        "wait" => {
            if let Some(condition) = object.get("condition") {
                check_wait_condition(condition, &format!("{path}.condition"), errors);
            }
        }
        "check" => {
            if let Some(condition) = object.get("condition") {
                check_condition_keys(condition, &format!("{path}.condition"), errors);
            }
        }
        "macro" => {
            if let Some(events) = object.get("events").and_then(serde_json::Value::as_array) {
                for (index, event) in events.iter().enumerate() {
                    let event_path = format!("{path}.events[{index}]");
                    let Some(event) = event.as_object() else {
                        continue;
                    };
                    if let Some(tag) = event.get("type").and_then(serde_json::Value::as_str) {
                        if let Some(allowed) = variant_keys(tag, MACRO_EVENT_VARIANTS) {
                            check_keys(event, allowed, &event_path, errors);
                        }
                    }
                }
            }
        }
        "overlay" => {
            if let Some(overlay) = object.get("custom_overlay") {
                check_overlay_definition(overlay, &format!("{path}.custom_overlay"), errors);
            }
        }
        _ => {}
    }
}

fn check_wait_condition(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    let Some(tag) = object.get("type").and_then(serde_json::Value::as_str) else {
        return;
    };
    if let Some(allowed) = variant_keys(tag, WAIT_CONDITION_VARIANTS) {
        check_keys(object, allowed, path, errors);
    }
}

fn check_overlay_definition(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    check_keys(object, OVERLAY_DEFINITION_KEYS, path, errors);
    if let Some(rect) = object.get("rect").and_then(serde_json::Value::as_object) {
        check_keys(rect, OVERLAY_RECT_KEYS, &format!("{path}.rect"), errors);
    }
    if let Some(style) = object.get("style").and_then(serde_json::Value::as_object) {
        check_keys(style, OVERLAY_STYLE_KEYS, &format!("{path}.style"), errors);
    }
    if let Some(elements) = object.get("elements").and_then(serde_json::Value::as_array) {
        for (index, element) in elements.iter().enumerate() {
            let element_path = format!("{path}.elements[{index}]");
            let Some(element) = element.as_object() else {
                continue;
            };
            let Some(tag) = element.get("type").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if let Some(allowed) = variant_keys(tag, OVERLAY_ELEMENT_VARIANTS) {
                check_keys(element, allowed, &element_path, errors);
            }
            if tag == "interactive_control" {
                if let Some(control) = element.get("control").and_then(serde_json::Value::as_object) {
                    let control_path = format!("{element_path}.control");
                    if let Some(kind) = control.get("type").and_then(serde_json::Value::as_str) {
                        if let Some(allowed) = variant_keys(kind, INTERACTIVE_CONTROL_VARIANTS) {
                            check_keys(control, allowed, &control_path, errors);
                        }
                    }
                    for action_field in ["on_enable", "on_disable", "action"] {
                        if let Some(action) = control.get(action_field) {
                            check_overlay_click_action(
                                action,
                                &format!("{control_path}.{action_field}"),
                                errors,
                            );
                        }
                    }
                }
            }
            for action_field in ["on_click", "on_finish"] {
                if let Some(action) = element.get(action_field) {
                    check_overlay_click_action(
                        action,
                        &format!("{element_path}.{action_field}"),
                        errors,
                    );
                }
            }
        }
    }
}

fn check_overlay_click_action(
    value: &serde_json::Value,
    path: &str,
    errors: &mut Vec<UnknownRuleKey>,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    check_keys(object, OVERLAY_CLICK_ACTION_KEYS, path, errors);
    let Some(tag) = object.get("type").and_then(serde_json::Value::as_str) else {
        return;
    };
    let payload_keys: &[&str] = match tag {
        "trigger_rule" | "toggle_rule" => &["rule_id"],
        "kill_process" | "trim_memory" => &["process_name"],
        _ => return,
    };
    if let Some(payload) = object.get("payload").and_then(serde_json::Value::as_object) {
        check_keys(payload, payload_keys, &format!("{path}.payload"), errors);
    }
}

const RULE_KEYS: &[&str] = &[
    "id",
    "name",
    "description",
    "is_enabled",
    "cooldown_seconds",
    "max_executions",
    "quota_period",
    "execution_count",
    "stop_on_error",
    "trigger",
    "conditions",
    "actions",
    "on_exit_actions",
    "custom_icon_path",
    "auto_process_icon",
    "last_executed_at",
    "quota_window_start",
    "tags",
];

const QUOTA_PERIOD_KEYS: &[&str] = &["custom"];
const CUSTOM_QUOTA_KEYS: &[&str] = &["value", "unit"];
const TAG_KEYS: &[&str] = &["name", "color", "icon"];
const CONDITION_ITEM_KEYS: &[&str] = &["operator", "condition"];
const ELSE_IF_KEYS: &[&str] = &["conditions", "actions"];

const TRIGGER_VARIANTS: &[(&str, &[&str])] = &[
    ("manual", &["$trigger_type"]),
    (
        "process",
        &["$trigger_type", "process_name", "match_mode", "single_instance"],
    ),
    (
        "window_focus",
        &[
            "$trigger_type",
            "process_name",
            "on_focus_gained",
            "on_focus_lost",
        ],
    ),
    (
        "resource",
        &[
            "$trigger_type",
            "process_name",
            "cpu_percent",
            "ram_mb",
            "duration_sec",
        ],
    ),
    (
        "hotkey",
        &[
            "$trigger_type",
            "key_combination",
            "hold_mode",
            "repeat_while_held",
        ],
    ),
    (
        "system",
        &["$trigger_type", "event_type", "device_id_filter"],
    ),
    (
        "schedule",
        &[
            "$trigger_type",
            "schedule_type",
            "interval_sec",
            "daily_time",
            "days_of_week",
            "day_of_month",
            "target_datetime",
        ],
    ),
    (
        "file_watcher",
        &[
            "$trigger_type",
            "folder_path",
            "filter_extension",
            "on_created",
            "on_modified",
            "on_deleted",
        ],
    ),
    ("webhook", &["$trigger_type", "endpoint_path"]),
];

const CONDITION_VARIANTS: &[(&str, &[&str])] = &[
    (
        "process_running",
        &[
            "$condition_type",
            "process_name",
            "match_mode",
            "must_be_running",
        ],
    ),
    (
        "window_focused",
        &[
            "$condition_type",
            "process_name",
            "match_mode",
            "must_be_focused",
        ],
    ),
    (
        "time_range",
        &["$condition_type", "start_time", "end_time", "days_of_week"],
    ),
    (
        "event",
        &["$condition_type", "event_type", "idle_duration_sec"],
    ),
    (
        "resource",
        &["$condition_type", "process_name", "cpu_percent", "ram_mb"],
    ),
    (
        "file_watcher",
        &["$condition_type", "path", "filter_extension", "must_exist"],
    ),
    (
        "rule_running",
        &["$condition_type", "rule_id", "must_be_running"],
    ),
];

const ACTION_VARIANTS: &[(&str, &[&str])] = &[
    (
        "start_process",
        &[
            "$action_type",
            "file_path",
            "arguments",
            "working_directory",
            "run_as_admin",
            "only_if_not_running",
            "run_hidden",
            "close_on_exit",
        ],
    ),
    (
        "kill_process",
        &["$action_type", "process_name", "kill_tree", "force"],
    ),
    (
        "window",
        &[
            "$action_type",
            "target_process",
            "window_action",
            "x",
            "y",
            "width",
            "height",
            "transparency",
        ],
    ),
    (
        "process_tuning",
        &["$action_type", "target_process", "priority", "affinity_mask", "trim_memory"],
    ),
    ("power_plan", &["$action_type", "plan"]),
    (
        "play_sound",
        &["$action_type", "file_path", "play_beep", "beep_frequency", "beep_duration_ms"],
    ),
    ("firewall", &["$action_type", "process_name", "action"]),
    ("notification", &["$action_type", "title", "message"]),
    (
        "http_webhook",
        &["$action_type", "url", "method", "headers", "payload"],
    ),
    (
        "audio_control",
        &[
            "$action_type",
            "target",
            "process_name",
            "action_type",
            "volume_level",
            "left_volume",
            "right_volume",
        ],
    ),
    (
        "audio_format",
        &["$action_type", "target", "device_name", "sample_rate", "bit_depth", "channels"],
    ),
    ("default_device", &["$action_type", "target", "device_name"]),
    (
        "execute_script",
        &["$action_type", "interpreter", "script", "run_in_window"],
    ),
    (
        "macro",
        &["$action_type", "preset_id", "operation", "events", "repeat", "speed", "infinite"],
    ),
    ("system_power", &["$action_type", "power_type"]),
    (
        "wait",
        &["$action_type", "mode", "seconds", "condition", "timeout_sec", "timeout_policy"],
    ),
    ("check", &["$action_type", "condition"]),
    (
        "display",
        &["$action_type", "width", "height", "refresh_rate", "monitor_index"],
    ),
    (
        "if_else",
        &["$action_type", "conditions", "then_actions", "else_if_branches", "else_actions"],
    ),
    (
        "rule_control",
        &["$action_type", "rule_id", "operation", "wait"],
    ),
    (
        "trim_working_set",
        &["$action_type", "target_process", "soft_limit_mb", "skip_if_focused"],
    ),
    (
        "discord_rpc_set",
        &[
            "$action_type",
            "app_id",
            "details",
            "state",
            "large_image",
            "large_text",
            "small_image",
            "small_text",
            "show_elapsed",
            "buttons",
            "auto_clear_on_process_exit",
        ],
    ),
    ("discord_rpc_clear", &["$action_type"]),
    (
        "archive",
        &["$action_type", "operation", "format", "source_path", "destination_path", "compression_level", "overwrite"],
    ),
    (
        "file_system",
        &["$action_type", "operation", "path", "target_path", "content", "create_parents", "overwrite"],
    ),
    (
        "overlay",
        &["$action_type", "overlay_id", "custom_overlay", "operation", "override_text", "duration_ms", "close_on_exit"],
    ),
];

const WAIT_CONDITION_VARIANTS: &[(&str, &[&str])] = &[
    (
        "window_opened",
        &["type", "process_name", "match_mode"],
    ),
    (
        "window_closed",
        &["type", "process_name", "match_mode"],
    ),
    (
        "process_started",
        &["type", "process_name", "match_mode"],
    ),
    (
        "process_exited",
        &["type", "process_name", "match_mode"],
    ),
    ("hotkey", &["type", "key_combination"]),
    ("file", &["type", "path", "condition"]),
    ("rule_started", &["type", "rule_id"]),
    ("rule_stopped", &["type", "rule_id"]),
];

const MACRO_EVENT_VARIANTS: &[(&str, &[&str])] = &[
    ("key_down", &["type", "key"]),
    ("key_up", &["type", "key"]),
    ("key_press", &["type", "key"]),
    ("text", &["type", "text", "delay_ms", "organic"]),
    (
        "mouse_move",
        &["type", "x", "y", "coord_mode", "smooth", "duration_ms", "organic"],
    ),
    ("mouse_down", &["type", "button"]),
    ("mouse_up", &["type", "button"]),
    (
        "mouse_click",
        &["type", "button", "clicks", "x", "y", "coord_mode", "smooth"],
    ),
    ("mouse_wheel", &["type", "delta"]),
    ("delay", &["type", "ms", "organic", "jitter_percent"]),
];

const OVERLAY_DEFINITION_KEYS: &[&str] = &[
    "id",
    "name",
    "rect",
    "anchor",
    "click_through",
    "draggable",
    "opacity",
    "style",
    "elements",
    "auto_hide_seconds",
    "target_process",
    "hide_from_capture",
    "fade_in_ms",
    "fade_out_ms",
    "monitor_index",
    "close_with_target",
];
const OVERLAY_RECT_KEYS: &[&str] = &["x", "y", "width", "height"];
const OVERLAY_STYLE_KEYS: &[&str] = &[
    "bg_color",
    "border_color",
    "border_width",
    "corner_radius",
    "glow",
];
const OVERLAY_CLICK_ACTION_KEYS: &[&str] = &["type", "payload"];

const OVERLAY_ELEMENT_VARIANTS: &[(&str, &[&str])] = &[
    (
        "text",
        &["type", "id", "text", "font_size", "color", "x", "y", "width", "height", "lock_aspect", "bold", "font_family", "rotation", "on_click"],
    ),
    (
        "image",
        &["type", "id", "path", "x", "y", "width", "height", "opacity", "lock_aspect", "rotation", "on_click"],
    ),
    (
        "crosshair",
        &["type", "id", "style", "size", "color", "center_dot", "thickness", "gap", "x", "y", "rotation"],
    ),
    (
        "shape",
        &["type", "id", "shape_type", "x", "y", "width", "height", "thickness", "color", "filled", "fill_color", "corner_radius", "lock_aspect", "rotation", "on_click"],
    ),
    (
        "graph",
        &["type", "id", "metric", "style", "sample_count", "color", "fill_color", "bg_color", "show_current_val", "x", "y", "width", "height", "rotation"],
    ),
    (
        "clock_timer",
        &["type", "id", "mode", "format_str", "countdown_total_ms", "countdown_remaining_ms", "stopwatch_elapsed_ms", "is_running", "font_size", "color", "bg_color", "bold", "font_family", "x", "y", "width", "height", "rotation", "on_finish"],
    ),
    (
        "interactive_control",
        &["type", "id", "control", "color", "bg_color", "active_color", "x", "y", "width", "height", "rotation"],
    ),
    (
        "system_telemetry",
        &["type", "id", "kind", "style", "color", "bg_color", "x", "y", "width", "height", "rotation", "on_click"],
    ),
    (
        "media_widget",
        &["type", "id", "show_controls", "compact", "color", "bg_color", "x", "y", "width", "height", "rotation"],
    ),
    (
        "key_visualizer",
        &["type", "id", "layout", "max_keys", "decay_seconds", "show_modifiers_only", "color", "bg_color", "active_color", "x", "y", "width", "height", "rotation"],
    ),
];

const INTERACTIVE_CONTROL_VARIANTS: &[(&str, &[&str])] = &[
    ("button", &["type", "label", "icon", "action"]),
    (
        "toggle_switch",
        &["type", "label", "is_active", "bind_rule_id", "on_enable", "on_disable"],
    ),
    ("slider", &["type", "label", "value", "min", "max", "bind_target"]),
];
