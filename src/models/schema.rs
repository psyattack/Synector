//! Draft-07 JSON Schema for the rule wire format.
//!
//! The schema is generated from the model types themselves, so `--rules-schema`, the template
//! generator and the human-readable reference can never drift from what serde deserializes.

/// Generates the draft-07 JSON Schema for one rule object.
pub fn rule_schema() -> serde_json::Value {
    schemars::generate::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<crate::models::Rule>()
        .to_value()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rules_schema_has_trigger_and_action_definitions() {
        let json = rule_schema();
        assert!(json["definitions"]["Trigger"].is_object());
        assert!(json["definitions"]["Action"].is_object());
        assert!(json["definitions"]["RuleCondition"].is_object());
        let text = json.to_string();
        for needle in [
            "$trigger_type",
            "$action_type",
            "$condition_type",
            "play_sound",
            "schedule",
            "max_executions",
            "quota_period",
        ] {
            assert!(text.contains(needle), "schema missing {needle}");
        }
    }
}
