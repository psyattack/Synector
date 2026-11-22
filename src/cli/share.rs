//! Share-code command bodies and the English CLI wording for share notices.

use serde_json::json;

use crate::config::RulesFile;
use crate::ipc::IpcClient;
use crate::validation::validate_rules;

use super::commands::target_name;
use super::json::{emit_json, fail_validation, fail_with, log_errors};
use super::rules_io::{import_share_offline, read_input};
use super::target::{current_config, import_share_via_daemon};

/// Prints a `SYN1:` share code for one rule (by id or name) or for every rule.
pub(super) fn cmd_share(selector: Option<&str>, json_output: bool) -> i32 {
    let cfg = match current_config() {
        Ok((cfg, _)) => cfg,
        Err(err) => return fail_with(json_output, &err),
    };

    let rules = match selector {
        Some(selector) => match cfg
            .rules
            .iter()
            .find(|rule| rule.id == selector || rule.name.eq_ignore_ascii_case(selector))
        {
            Some(rule) => vec![rule.clone()],
            None => return fail_with(json_output, &format!("rule \"{selector}\" not found")),
        },
        None => cfg.rules.clone(),
    };
    if rules.is_empty() {
        return fail_with(json_output, "no rules configured to share");
    }

    let count = rules.len();
    let (payload, warnings) = crate::sharing::build_payload(rules, &cfg.overlays, &cfg.macro_presets);
    let warning_texts: Vec<String> = warnings
        .iter()
        .map(share_warning_text_cli)
        .collect();

    match crate::sharing::compress_and_encode(&payload) {
        Ok(code) => {
            if json_output {
                emit_json(json!({
                    "ok": true,
                    "rules": count,
                    "bytes": code.len(),
                    "code": code,
                    "warnings": warning_texts,
                }));
            } else {
                for warning in &warning_texts {
                    eprintln!("warning: {warning}");
                }
                // The code alone goes to stdout so it can be piped or redirected safely.
                println!("{code}");
                eprintln!("{count} rule(s), {} byte(s)", code.len());
            }
            0
        }
        Err(err) => fail_with(
            json_output,
            &format!("cannot build share code: {}", crate::sharing::error_text(&err)),
        ),
    }
}

/// Imports a share code from a literal argument, a file path, or stdin (`-`).
pub(super) fn cmd_import_share(raw_argument: &str, json_output: bool, enable: bool) -> i32 {
    let raw = match read_share_argument(raw_argument) {
        Ok(raw) => raw,
        Err(err) => return fail_with(json_output, &err),
    };
    let payload = match crate::sharing::decode_and_decompress(&raw) {
        Ok(payload) => payload,
        Err(err) => {
            return fail_with(
                json_output,
                &format!("invalid share code: {}", crate::sharing::error_text(&err)),
            )
        }
    };

    let (cfg, from_daemon) = match current_config() {
        Ok(value) => value,
        Err(err) => return fail_with(json_output, &err),
    };
    let prepared = match crate::sharing::prepare_import(
        payload.clone(),
        &cfg.rules,
        &cfg.overlays,
        &cfg.macro_presets,
        enable,
    ) {
        Ok(prepared) => prepared,
        Err(err) => {
            return fail_with(
                json_output,
                &format!("cannot import share code: {}", crate::sharing::error_text(&err)),
            )
        }
    };

    let warning_texts: Vec<String> = prepared
        .warnings
        .iter()
        .map(share_warning_text_cli)
        .collect();
    let errors = validate_rules(&prepared.rules);
    if !errors.is_empty() {
        log_errors(&errors);
        if json_output {
            return fail_validation(&errors, &warning_texts, target_name(from_daemon));
        }
        for error in &errors {
            eprintln!("error: {error}");
        }
        eprintln!("Nothing was imported.");
        return 1;
    }

    let mut target = target_name(from_daemon);
    let summary = if from_daemon {
        match import_share_via_daemon(&payload, enable, json_output) {
            Ok(summary) => summary,
            Err(exit_code) => return exit_code,
        }
    } else if IpcClient::is_daemon_running() {
        // The daemon became reachable after the snapshot: one IPC request gets the import right
        // instead of writing the sections under it ([U13-003]).
        target = "daemon";
        match import_share_via_daemon(&payload, enable, json_output) {
            Ok(summary) => summary,
            Err(exit_code) => return exit_code,
        }
    } else {
        match import_share_offline(&cfg, &prepared, enable, json_output) {
            Ok(summary) => summary,
            Err(exit_code) => return exit_code,
        }
    };

    for warning in &warning_texts {
        crate::logging::warn("CLI", warning);
        if !json_output {
            eprintln!("warning: {warning}");
        }
    }

    if json_output {
        emit_json(json!({
            "ok": true,
            "imported": summary.imported,
            "skipped_duplicates": summary.skipped_duplicates,
            "overlays": summary.overlays,
            "macro_presets": summary.macro_presets,
            "enabled": summary.enabled,
            "target": target,
            "warnings": warning_texts,
        }));
    } else {
        println!(
            "Imported {} automation(s); {} duplicate(s) skipped.",
            summary.imported, summary.skipped_duplicates
        );
        if summary.overlays > 0 || summary.macro_presets > 0 {
            println!(
                "Added {} overlay(s) and {} macro preset(s) to the shared library.",
                summary.overlays, summary.macro_presets
            );
        }
        if summary.imported == 0 && summary.overlays == 0 && summary.macro_presets == 0 {
            println!("Nothing new was imported.");
        } else if !enable && summary.imported > 0 {
            println!(
                "Imported rules are disabled; pass --enable-imported or use --rules-enable <id> to activate them."
            );
        }
        if target == "rules" {
            println!("Applied to {} (daemon is offline).", RulesFile::path().display());
        }
    }
    0
}

/// A share argument is a literal `SYN1:` code, a file path, or `-` for stdin. Anything else is a
/// path, so a mistyped path reports a read error instead of a misleading decode error ([U13-015]).
fn read_share_argument(value: &str) -> Result<String, String> {
    if value.starts_with(crate::sharing::SHARE_PREFIX) {
        Ok(value.to_string())
    } else {
        read_input(value)
    }
}

/// English CLI wording for a share-code notice (the GUI localizes its own).
fn share_warning_text_cli(warning: &crate::sharing::ShareWarning) -> String {
    use crate::sharing::ShareWarning as W;
    match warning {
        W::MissingMacroPreset(id) => {
            format!("macro preset \"{id}\" is missing and will not work")
        }
        W::MissingOverlay(id) => format!("overlay \"{id}\" is missing and will not work"),
        W::SensitiveScript => "contains scripts that run code when executed".to_string(),
        W::SensitiveAdminProcess => "starts processes with administrator rights".to_string(),
        W::SensitiveWebhook => "sends HTTP requests and may contain tokens".to_string(),
        W::CustomIconDropped => "custom icon is not included".to_string(),
        W::OverlayAssetDropped => "overlay images and icons are not included".to_string(),
        W::MissingRule(id) => {
            format!("rule \"{id}\" is missing; overlay buttons that use it will not work")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_share_argument_is_literal_only_for_syn1_codes() {
        assert_eq!(
            read_share_argument("SYN1:abc").expect("literal"),
            "SYN1:abc"
        );
        // Anything that is not a SYN1: code is a path, so a typo reports a read error instead of
        // a misleading "invalid share code" ([U13-015]).
        let err = read_share_argument("synector_missing_code_file_12345")
            .expect_err("missing path must not become a literal");
        assert!(err.contains("cannot read"), "{err}");
        assert!(err.contains("synector_missing_code_file_12345"), "{err}");
    }

    #[test]
    fn test_share_warning_cli_text() {
        assert!(share_warning_text_cli(&crate::sharing::ShareWarning::SensitiveScript)
            .contains("script"));
        assert!(share_warning_text_cli(&crate::sharing::ShareWarning::MissingOverlay(
            "ov".into()
        ))
        .contains("ov"));
        assert_eq!(
            share_warning_text_cli(&crate::sharing::ShareWarning::CustomIconDropped),
            "custom icon is not included"
        );
    }
}
