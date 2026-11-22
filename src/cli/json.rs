//! The `--json` envelope helpers and the shared failure/logging paths.
//!
//! Every failure envelope carries `ok`, `error`, `errors`, `warnings` and `target`, so a machine
//! consumer reads the same keys regardless of the failure class ([U13-011]).

use serde_json::{json, Value};

use super::rules_io::ApplyReport;

pub(super) fn report_json(
    ok: bool,
    target: &str,
    report: &ApplyReport,
    deleted: &[String],
    dry_run: bool,
    warnings: &[String],
) -> Value {
    json!({
        "ok": ok,
        "target": target,
        "dry_run": dry_run,
        "created": report.created,
        "updated": report.updated,
        "deleted": deleted,
        "warnings": warnings,
    })
}

pub(super) fn emit_json(value: Value) {
    match serde_json::to_string_pretty(&value) {
        Ok(text) => println!("{text}"),
        Err(_) => println!("{{}}"),
    }
}

/// One `--json` failure envelope for every failure path, so a machine consumer can always read
/// `.ok`, `.error`, `.errors`, `.warnings` and `.target` regardless of the failure class
/// ([U13-011]).
pub(super) fn error_json(
    error: &str,
    errors: &[String],
    warnings: &[String],
    target: Option<&str>,
) -> Value {
    json!({
        "ok": false,
        "error": error,
        "errors": errors,
        "warnings": warnings,
        "target": target,
    })
}

pub(super) fn emit_error_json(
    error: &str,
    errors: &[String],
    warnings: &[String],
    target: Option<&str>,
) -> i32 {
    emit_json(error_json(error, errors, warnings, target));
    1
}

/// Reports validation errors in `--json` mode. The first error is promoted to `error`; the full
/// list stays in `errors` ([U13-011]).
pub(super) fn fail_validation(errors: &[String], warnings: &[String], target: &str) -> i32 {
    let error = errors
        .first()
        .map(String::as_str)
        .unwrap_or("validation failed");
    emit_error_json(error, errors, warnings, Some(target))
}

pub(super) fn fail_with(json_output: bool, message: &str) -> i32 {
    if json_output {
        crate::logging::error("CLI", message);
        emit_error_json(message, &[message.to_string()], &[], None)
    } else {
        fail(message)
    }
}

pub(super) fn log_warnings(warnings: &[String]) {
    for warning in warnings {
        crate::logging::warn("CLI", warning);
    }
}

pub(super) fn log_errors(errors: &[String]) {
    for error in errors {
        crate::logging::error("CLI", error);
    }
}

pub(super) fn fail(message: &str) -> i32 {
    crate::logging::error("CLI", message);
    eprintln!("error: {message}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_json_shape() {
        let report = ApplyReport {
            created: vec!["a".to_string()],
            updated: vec!["b".to_string()],
        };
        let value = report_json(true, "daemon", &report, &["c".to_string()], true, &[]);
        assert_eq!(value["ok"], serde_json::json!(true));
        assert_eq!(value["target"], serde_json::json!("daemon"));
        assert_eq!(value["dry_run"], serde_json::json!(true));
        assert_eq!(value["created"][0], serde_json::json!("a"));
        assert_eq!(value["updated"][0], serde_json::json!("b"));
        assert_eq!(value["deleted"][0], serde_json::json!("c"));
    }

    #[test]
    fn test_json_error_envelope_shape() {
        // U13-011: validation failures used to emit {ok:false, errors} with no "error" key.
        let errors = vec!["rule #1 (typo): bad".to_string()];
        let warnings = vec!["no trigger".to_string()];
        let value = error_json("rule #1 (typo): bad", &errors, &warnings, Some("rules"));
        assert_eq!(value["ok"], serde_json::json!(false));
        assert_eq!(value["error"], serde_json::json!("rule #1 (typo): bad"));
        assert_eq!(value["errors"][0], serde_json::json!("rule #1 (typo): bad"));
        assert_eq!(value["warnings"][0], serde_json::json!("no trigger"));
        assert_eq!(value["target"], serde_json::json!("rules"));

        let value = error_json("boom", &["boom".to_string()], &[], None);
        assert!(value["target"].is_null());
        assert!(
            value.as_object().expect("object").contains_key("target"),
            "every failure envelope names the target key"
        );
    }
}
