//! Schema, reference and template generation for `--rules-schema`, `--rules-reference` and
//! `--rules-template`.

use serde_json::{Map, Value};

use crate::models::rule_schema;

use super::json::fail;

pub(super) fn cmd_schema() -> i32 {
    match serde_json::to_string_pretty(&rule_schema()) {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(err) => fail(&err.to_string()),
    }
}

pub(super) fn cmd_reference() -> i32 {
    print!("{}", render_reference());
    0
}

/// Human-readable catalogue generated from the same JSON Schema as `--rules-schema`,
/// so the two can never drift apart.
fn render_reference() -> String {
    let root = rule_schema();
    let mut out = String::new();
    out.push_str("Synector rule reference\n");
    out.push_str("Machine-readable draft-07 JSON Schema: Synector.exe --rules-schema\n\n");
    out.push_str("RULE FIELDS\n");
    if let Some(properties) = root.get("properties").and_then(Value::as_object) {
        let required = required_names(&root);
        render_fields(&root, properties, &required, &mut out);
    }
    for (definition, tag, title) in [
        ("Trigger", "$trigger_type", "TRIGGERS"),
        ("Action", "$action_type", "ACTIONS"),
        ("RuleCondition", "$condition_type", "CONDITIONS"),
        ("WaitCondition", "type", "WAIT CONDITIONS"),
        ("MacroEvent", "type", "MACRO EVENTS"),
    ] {
        out.push('\n');
        out.push_str(title);
        out.push_str(&format!(" ({tag})\n"));
        render_variants(&root, definition, tag, &mut out);
    }
    out.push_str("\nNested objects (overlays, tags, branches) are described by --rules-schema.\n");
    out
}

fn required_names(schema: &Value) -> std::collections::BTreeSet<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn definition<'a>(root: &'a Value, name: &str) -> Option<&'a Value> {
    root.get("definitions")?.get(name)
}

fn definition_variants<'a>(root: &'a Value, name: &str) -> &'a [Value] {
    definition(root, name)
        .and_then(|definition| definition.get("oneOf"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn render_fields(
    root: &Value,
    properties: &Map<String, Value>,
    required: &std::collections::BTreeSet<String>,
    out: &mut String,
) {
    for (name, schema) in properties {
        let marker = if is_required(required, name) {
            " (required)"
        } else {
            ""
        };
        out.push_str(&format!("  {name}: {}{marker}\n", type_label(root, schema, 0)));
    }
}

fn render_variants(root: &Value, definition: &str, tag: &str, out: &mut String) {
    for variant in definition_variants(root, definition) {
        let Some(properties) = variant.get("properties").and_then(Value::as_object) else {
            continue;
        };
        let variant_name = properties
            .get(tag)
            .and_then(tag_value)
            .unwrap_or_else(|| "<unknown>".to_string());
        let required = required_names(variant);
        let mut fields = Vec::new();
        for (name, schema) in properties {
            if name == tag {
                continue;
            }
            let marker = if is_required(&required, name) {
                " (required)"
            } else {
                ""
            };
            fields.push(format!("{name}: {}{marker}", type_label(root, schema, 0)));
        }
        if fields.is_empty() {
            out.push_str(&format!("  {variant_name} - no fields\n"));
        } else {
            out.push_str(&format!("  {variant_name} - {}\n", fields.join("; ")));
        }
    }
}

fn is_required(required: &std::collections::BTreeSet<String>, name: &str) -> bool {
    required.contains(name)
}

fn tag_value(schema: &Value) -> Option<String> {
    if let Some(value) = schema.get("const").and_then(Value::as_str) {
        return Some(value.to_string());
    }
    schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn type_label(root: &Value, schema: &Value, depth: usize) -> String {
    if depth > 6 {
        return "...".to_string();
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        if let Some(definition) = definition(root, name) {
            if let Some(values) = definition.get("enum").and_then(Value::as_array) {
                if values.iter().all(Value::is_string) {
                    return values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("|");
                }
            }
        }
        return name.to_string();
    }
    if let Some(instance_types) = schema.get("type") {
        return match instance_types {
            Value::String(instance_type) if instance_type == "array" => {
                array_label(root, schema, depth)
            }
            Value::String(instance_type) => instance_type.clone(),
            Value::Array(types) => types
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" or "),
            _ => "any".to_string(),
        };
    }
    if schema.get("items").is_some() {
        return array_label(root, schema, depth);
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if values.iter().all(Value::is_string) {
            return values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("|");
        }
    }
    if let Some(candidates) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
    {
        let labels: Vec<String> = candidates
            .iter()
            .map(|candidate| type_label(root, candidate, depth + 1))
            .collect();
        let non_null: Vec<&String> = labels
            .iter()
            .filter(|label| label.as_str() != "null")
            .collect();
        if labels.len() > 1 && non_null.len() == 1 {
            return format!("{} or null", non_null[0]);
        }
        return labels.join(" or ");
    }
    if let Some(first) = schema
        .get("allOf")
        .and_then(Value::as_array)
        .and_then(|schemas| schemas.first())
    {
        return type_label(root, first, depth + 1);
    }
    if schema.get("properties").is_some() {
        return "object".to_string();
    }
    "any".to_string()
}

fn array_label(root: &Value, schema: &Value, depth: usize) -> String {
    match schema.get("items") {
        Some(Value::Array(items)) => items
            .first()
            .map(|first| format!("array of {}", type_label(root, first, depth + 1)))
            .unwrap_or_else(|| "array".to_string()),
        Some(items @ Value::Object(_)) => {
            format!("array of {}", type_label(root, items, depth + 1))
        }
        _ => "array".to_string(),
    }
}

pub(super) fn cmd_template(spec: Option<&str>) -> i32 {
    let root = rule_schema();
    let triggers = variant_names(&root, "Trigger", "$trigger_type");
    let actions = variant_names(&root, "Action", "$action_type");
    let Some(spec) = spec else {
        println!("Available triggers: {}", triggers.join(", "));
        println!("Available actions: {}", actions.join(", "));
        println!("Usage: --rules-template <trigger>[:<action>]   (action defaults to notification)");
        return 0;
    };
    let (trigger, action) = parse_template_spec(spec);
    if !triggers.contains(&trigger) {
        return fail(&format!(
            "unknown trigger \"{trigger}\"; available: {}",
            triggers.join(", ")
        ));
    }
    if !actions.contains(&action) {
        return fail(&format!(
            "unknown action \"{action}\"; available: {}",
            actions.join(", ")
        ));
    }
    match template_rule(&root, &trigger, &action) {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(err) => fail(&err.to_string()),
        },
        Err(err) => fail(&err),
    }
}

fn parse_template_spec(spec: &str) -> (String, String) {
    let spec = spec.trim();
    let (trigger, action) = match spec.split_once(':') {
        Some((trigger, action)) => (trigger.trim(), action.trim()),
        None => (spec, ""),
    };
    let trigger = if trigger.is_empty() {
        "manual".to_string()
    } else {
        trigger.to_string()
    };
    let action = if action.is_empty() {
        "notification".to_string()
    } else {
        action.to_string()
    };
    (trigger, action)
}

/// Builds a minimal but valid rule for the requested trigger/action pair.
fn template_rule(root: &Value, trigger: &str, action: &str) -> Result<Value, String> {
    let mut rule = serde_json::Map::new();
    rule.insert(
        "id".to_string(),
        Value::String(format!("rule_{trigger}_{action}")),
    );
    rule.insert(
        "name".to_string(),
        Value::String(format!("New {trigger} rule")),
    );
    rule.insert(
        "trigger".to_string(),
        variant_value(root, "Trigger", "$trigger_type", trigger, 0)?,
    );
    rule.insert(
        "actions".to_string(),
        Value::Array(vec![variant_value(
            root,
            "Action",
            "$action_type",
            action,
            0,
        )?]),
    );
    Ok(Value::Object(rule))
}

fn variant_names(root: &Value, definition: &str, tag: &str) -> Vec<String> {
    definition_variants(root, definition)
        .iter()
        .filter_map(|variant| {
            variant
                .get("properties")
                .and_then(|properties| properties.get(tag))
                .and_then(tag_value)
        })
        .collect()
}

fn variant_value(
    root: &Value,
    definition: &str,
    tag: &str,
    variant_name: &str,
    depth: usize,
) -> Result<Value, String> {
    if depth > 8 {
        return Ok(Value::Null);
    }
    let variants = definition_variants(root, definition);
    if variants.is_empty() {
        return Err(format!("schema is missing the {definition} definition"));
    }
    let variant = variants
        .iter()
        .find(|variant| {
            variant
                .get("properties")
                .and_then(|properties| properties.get(tag))
                .and_then(tag_value)
                .as_deref()
                == Some(variant_name)
        })
        .ok_or_else(|| format!("unknown {definition} variant \"{variant_name}\""))?;
    let properties = variant
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{definition} variant \"{variant_name}\" is not an object"))?;
    let mut fields = serde_json::Map::new();
    fields.insert(tag.to_string(), Value::String(variant_name.to_string()));
    for required in required_names(variant) {
        if required == tag {
            continue;
        }
        if let Some(schema) = properties.get(&required) {
            fields.insert(
                required.clone(),
                placeholder_value(root, schema, &required, depth + 1),
            );
        }
    }
    for (name, value) in extra_fields(variant_name) {
        if properties.contains_key(name) {
            fields.insert(name.to_string(), value);
        }
    }
    Ok(Value::Object(fields))
}

/// Fields that are optional in the schema but required by runtime validation rules, plus
/// illustrative defaults that make the generated skeleton immediately useful.
fn extra_fields(variant_name: &str) -> Vec<(&'static str, Value)> {
    match variant_name {
        "resource" => vec![("cpu_percent", Value::from(80.0))],
        "schedule" => vec![
            ("schedule_type", Value::String("interval".to_string())),
            ("interval_sec", Value::from(60)),
        ],
        "play_sound" => vec![
            ("play_beep", Value::Bool(true)),
            ("beep_frequency", Value::from(1000)),
            ("beep_duration_ms", Value::from(200)),
        ],
        "wait" => vec![
            ("mode", Value::String("timer".to_string())),
            ("seconds", Value::from(1.0)),
        ],
        _ => Vec::new(),
    }
}

fn placeholder_value(root: &Value, schema: &Value, field_name: &str, depth: usize) -> Value {
    if depth > 8 {
        return Value::Null;
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        if let Some(definition) = definition(root, name) {
            if let Some(values) = definition.get("enum").and_then(Value::as_array) {
                if let Some(first) = values.first() {
                    return first.clone();
                }
            }
            if let Some(first) = definition_variants(root, name).first() {
                if let Some(properties) = first.get("properties").and_then(Value::as_object) {
                    if let Some(tag) = discover_tag_key(properties) {
                        if let Some(tag_name) = properties.get(&tag).and_then(tag_value) {
                            if let Ok(value) = variant_value(root, name, &tag, &tag_name, depth + 1)
                            {
                                return value;
                            }
                        }
                    }
                }
            }
            if let Some(properties) = definition.get("properties").and_then(Value::as_object) {
                let mut fields = serde_json::Map::new();
                for required in required_names(definition) {
                    if let Some(field_schema) = properties.get(&required) {
                        fields.insert(
                            required.clone(),
                            placeholder_value(root, field_schema, &required, depth + 1),
                        );
                    }
                }
                return Value::Object(fields);
            }
        }
        return Value::String(name.to_string());
    }
    if let Some(instance_types) = schema.get("type") {
        return match instance_types {
            Value::String(instance_type) => single_type_placeholder(instance_type, field_name),
            Value::Array(instance_types) => instance_types
                .iter()
                .filter_map(Value::as_str)
                .find(|instance_type| *instance_type != "null")
                .map(|instance_type| single_type_placeholder(instance_type, field_name))
                .unwrap_or(Value::Null),
            _ => Value::Null,
        };
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if let Some(first) = values.first() {
            return first.clone();
        }
    }
    if let Some(candidates) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
    {
        for candidate in candidates {
            if is_null_type(candidate) {
                continue;
            }
            return placeholder_value(root, candidate, field_name, depth + 1);
        }
    }
    if let Some(first) = schema
        .get("allOf")
        .and_then(Value::as_array)
        .and_then(|schemas| schemas.first())
    {
        return placeholder_value(root, first, field_name, depth + 1);
    }
    if schema.get("items").is_some() {
        return Value::Array(Vec::new());
    }
    if schema.get("properties").is_some() {
        return Value::Object(serde_json::Map::new());
    }
    Value::String("value".to_string())
}

fn single_type_placeholder(instance_type: &str, field_name: &str) -> Value {
    match instance_type {
        "null" => Value::Null,
        "boolean" => Value::Bool(false),
        "integer" => Value::from(1),
        "number" => Value::from(1.0),
        "string" => Value::String(string_placeholder(field_name)),
        "array" => Value::Array(Vec::new()),
        "object" => Value::Object(serde_json::Map::new()),
        _ => Value::Null,
    }
}

fn string_placeholder(field_name: &str) -> String {
    let name = field_name.to_ascii_lowercase();
    if name.contains("key_combination") || name == "hotkey" {
        "CTRL+ALT+N".to_string()
    } else if name.contains("folder_path") {
        "C:\\Path\\To\\Folder".to_string()
    } else if name.contains("file_path") || name.ends_with("path") {
        "C:\\Windows\\System32\\notepad.exe".to_string()
    } else if name.contains("process") {
        "your_process.exe".to_string()
    } else if name.contains("url") {
        "https://example.com/webhook".to_string()
    } else if name.contains("title") {
        "Title".to_string()
    } else if name.contains("message") {
        "Message".to_string()
    } else if name.contains("script") {
        "echo hello".to_string()
    } else if name.contains("name") {
        "name".to_string()
    } else {
        "value".to_string()
    }
}

fn discover_tag_key(object: &Map<String, Value>) -> Option<String> {
    object.iter().find_map(|(name, schema)| {
        let single_value_enum = schema
            .get("enum")
            .and_then(Value::as_array)
            .map(|values| values.len() == 1)
            .unwrap_or(false);
        (schema.get("const").is_some() || single_value_enum).then(|| name.clone())
    })
}

fn is_null_type(schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(instance_type)) => instance_type == "null",
        Some(Value::Array(instance_types)) => {
            !instance_types.is_empty()
                && instance_types
                    .iter()
                    .all(|instance_type| instance_type.as_str() == Some("null"))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::rules_io::parse_rules_json;
    use crate::models::{Action, Rule, Trigger};
    use crate::validation::validate_rules;

    #[test]
    fn test_reference_covers_all_trigger_and_action_variants() {
        let root = rule_schema();
        let reference = render_reference();
        for (definition, tag) in [("Trigger", "$trigger_type"), ("Action", "$action_type")] {
            let variants = definition_variants(&root, definition);
            assert!(variants.len() >= 9, "{definition:?} has too few variants");
            for variant in variants {
                let properties = variant
                    .get("properties")
                    .and_then(Value::as_object)
                    .expect("variant object");
                let name = properties
                    .get(tag)
                    .and_then(tag_value)
                    .expect("variant tag");
                assert!(
                    reference.contains(&format!("  {name}")),
                    "reference is missing {definition:?} variant {name}"
                );
            }
        }
    }

    #[test]
    fn test_reference_mentions_rule_fields_and_sections() {
        let reference = render_reference();
        for needle in [
            "RULE FIELDS",
            "max_executions",
            "quota_period",
            "TRIGGERS",
            "ACTIONS",
            "CONDITIONS",
            "WAIT CONDITIONS",
            "MACRO EVENTS",
        ] {
            assert!(reference.contains(needle), "reference missing {needle}");
        }
    }

    #[test]
    fn test_template_generates_valid_rule_for_every_combination() {
        let root = rule_schema();
        let triggers = variant_names(&root, "Trigger", "$trigger_type");
        let actions = variant_names(&root, "Action", "$action_type");
        assert_eq!(triggers.len(), 9, "{triggers:?}");
        assert_eq!(actions.len(), 26, "{actions:?}");
        for trigger in &triggers {
            for action in &actions {
                let value = template_rule(&root, trigger, action).expect("template");
                let rule: Rule = serde_json::from_value(value.clone()).unwrap_or_else(|err| {
                    panic!("{trigger}:{action} does not parse: {err}\n{value}")
                });
                let errors = validate_rules(std::slice::from_ref(&rule));
                assert!(
                    errors.is_empty(),
                    "{trigger}:{action} template is invalid: {errors:?}\n{value}"
                );
            }
        }
    }

    #[test]
    fn test_template_spec_shorthand_and_errors() {
        assert_eq!(
            parse_template_spec("hotkey"),
            ("hotkey".to_string(), "notification".to_string())
        );
        assert_eq!(
            parse_template_spec(":play_sound"),
            ("manual".to_string(), "play_sound".to_string())
        );
        assert_eq!(
            parse_template_spec("schedule:wait"),
            ("schedule".to_string(), "wait".to_string())
        );
        let root = rule_schema();
        assert!(template_rule(&root, "nope", "notification").is_err());
        assert!(template_rule(&root, "manual", "nope").is_err());
    }

    #[test]
    fn test_template_play_sound_roundtrip() {
        let root = rule_schema();
        let value = template_rule(&root, "schedule", "play_sound").expect("template");
        let json = serde_json::to_string(&value).expect("json");
        let rules = parse_rules_json(&json).expect("parse");
        assert_eq!(rules.len(), 1);
        assert!(matches!(
            rules[0].actions[0],
            Action::PlaySound { play_beep: true, .. }
        ));
        assert!(matches!(
            rules[0].trigger,
            Some(Trigger::Schedule {
                interval_sec: 60,
                ..
            })
        ));
    }
}
