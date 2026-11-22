//! Share codes: one-at-a-time background export queueing, clipboard copy/import and the
//! localized error/warning text tables that stay state-owned.

use crate::ipc::{IpcRequest, IpcResponse};
use crate::models::Rule;

use super::ipc::IpcJobKind;
use super::GuiState;

type ShareExportOutcome = (
    Result<String, crate::sharing::ShareError>,
    usize,
    Vec<crate::sharing::ShareWarning>,
);

/// One share-code compression job waiting behind the running one.
struct ShareExportJob {
    payload: crate::sharing::SharePayload,
    warnings: Vec<crate::sharing::ShareWarning>,
}

/// Share-code export bookkeeping: one compression runs at a time and further copies queue
/// instead of overwriting the running receiver (which used to lose the earlier result).
#[derive(Default)]
pub(super) struct ShareExports {
    rx: Option<std::sync::mpsc::Receiver<ShareExportOutcome>>,
    queued: std::collections::VecDeque<ShareExportJob>,
}

impl GuiState {
    pub fn copy_single_rule_to_clipboard(&mut self, _ctx: &egui::Context, rule: &Rule) {
        self.start_share_export(vec![rule.clone()]);
    }

    /// Copies every automation currently visible in the rules view: the same tag filter and
    /// search that drive the list, so sharing a subset no longer requires deleting rules.
    pub fn copy_all_rules_to_clipboard(&mut self, _ctx: &egui::Context) {
        let rules = self.visible_rules();
        if rules.is_empty() {
            self.set_toast(self.loc().toast_copy_all_empty);
            return;
        }
        self.start_share_export(rules);
    }

    fn share_library(&self) -> (Vec<crate::models::OverlayDefinition>, Vec<crate::models::MacroPreset>) {
        match &self.daemon_state {
            Some(ds) => (ds.config.overlays.clone(), ds.config.macro_presets.clone()),
            None => {
                let library = crate::config::Library::read();
                (library.overlays, library.macro_presets)
            }
        }
    }

    fn start_share_export(&mut self, rules: Vec<Rule>) {
        let (overlays, presets) = self.share_library();
        let (payload, warnings) = crate::sharing::build_payload(rules, &overlays, &presets);
        for warning in &warnings {
            // Notices are advisory: they go to the log and are summarized in the toast, but
            // copying stays a single click.
            crate::logging::warn("SHARE", &self.share_warning_text(warning));
        }
        if self.share.rx.is_some() {
            // A second Copy must not drop the running export's receiver: the earlier code would
            // have swallowed its result and only the second code reached the clipboard.
            let rule_count = payload.rules.len();
            self.share.queued.push_back(ShareExportJob { payload, warnings });
            crate::logging::info(
                "SHARE",
                &format!("Queued share code for {rule_count} automation(s) behind the running export"),
            );
            return;
        }
        self.spawn_share_export(payload, warnings);
    }

    fn spawn_share_export(
        &mut self,
        payload: crate::sharing::SharePayload,
        warnings: Vec<crate::sharing::ShareWarning>,
    ) {
        let rule_count = payload.rules.len();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::sharing::compress_and_encode(&payload);
            let _ = tx.send((result, rule_count, warnings));
        });
        self.share.rx = Some(rx);
        // Keep a frame coming so the result reaches the clipboard without waiting for input.
        if let Some(ctx) = &self.ipc.frame_ctx {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        crate::logging::info(
            "SHARE",
            &format!("Building share code for {rule_count} automation(s) in the background"),
        );
    }

    fn start_next_queued_export(&mut self) {
        if self.share.rx.is_none() {
            if let Some(next) = self.share.queued.pop_front() {
                self.spawn_share_export(next.payload, next.warnings);
            }
        }
    }

    /// Drains the background export channel; called once per frame from the app loop.
    pub fn poll_share_events(&mut self, ctx: &egui::Context) {
        self.ipc.attach_frame_ctx(ctx);
        enum Polled {
            Outcome(ShareExportOutcome),
            Empty,
            Closed,
        }
        let polled = match self.share.rx.as_ref().map(|rx| rx.try_recv()) {
            None => {
                self.start_next_queued_export();
                return;
            }
            Some(Ok(outcome)) => Polled::Outcome(outcome),
            Some(Err(std::sync::mpsc::TryRecvError::Empty)) => Polled::Empty,
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => Polled::Closed,
        };
        match polled {
            Polled::Outcome((Ok(code), count, warnings)) => {
                self.share.rx = None;
                crate::sharing::copy_text_to_clipboard(ctx, &code);
                let mut message = self.loc().toast_share_copied.replace("{}", &count.to_string());
                if !warnings.is_empty() {
                    let notices = self
                        .loc()
                        .toast_share_warnings
                        .replace("{}", &warnings.len().to_string());
                    message = format!("{message} · {notices}");
                }
                self.set_toast(message);
                crate::logging::info(
                    "SHARE",
                    &format!(
                        "Share code copied: {count} automation(s), {} byte(s), {} notice(s)",
                        code.len(),
                        warnings.len()
                    ),
                );
                self.start_next_queued_export();
            }
            Polled::Outcome((Err(error), _, _)) => {
                self.share.rx = None;
                let detail = self.share_error_text(&error);
                self.set_toast(self.loc().toast_share_failed.replace("{}", &detail));
                crate::logging::warn("SHARE", &format!("Share code export failed: {error}"));
                self.start_next_queued_export();
            }
            Polled::Empty => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            Polled::Closed => {
                self.share.rx = None;
                self.start_next_queued_export();
            }
        }
    }

    /// Imports the share code currently in the clipboard in one step.
    ///
    /// Rules are created disabled; the daemon applies the code atomically and reports how many
    /// rules, library entries and duplicates it handled, which the toast summarizes.
    pub fn import_automations_from_clipboard(&mut self, _ctx: &egui::Context) {
        let text = match crate::sharing::read_text_from_clipboard() {
            Ok(text) => text,
            Err(error) => {
                self.set_toast(format!("{}: {}", self.loc().toast_import_clipboard_err, error));
                return;
            }
        };

        let payload = match crate::sharing::decode_and_decompress(&text) {
            Ok(payload) => payload,
            Err(error) => {
                let detail = self.share_error_text(&error);
                crate::logging::warn("SHARE", &format!("Share code rejected: {error}"));
                self.set_toast(format!("{}: {}", self.loc().toast_import_invalid, detail));
                return;
            }
        };

        let request = IpcRequest::ImportShare {
            payload: Box::new(payload),
            enable: false,
        };
        self.enqueue_ipc(IpcJobKind::ImportShare, request, None);
    }

    /// Applies the daemon's answer to a decoded share code. Imported rules are disabled and the
    /// toast summarizes what the daemon actually applied.
    pub(super) fn apply_import_result(&mut self, result: Result<IpcResponse, String>) {
        match result {
            Ok(IpcResponse::ShareImportResult(summary)) => {
                let mut message = if summary.imported == 0 {
                    self.loc().share_import_nothing_new.to_string()
                } else {
                    self.loc()
                        .toast_import_success
                        .replace("{}", &summary.imported.to_string())
                };
                if summary.skipped_duplicates > 0 {
                    message.push_str(" · ");
                    message.push_str(
                        &self
                            .loc()
                            .toast_import_skipped
                            .replace("{}", &summary.skipped_duplicates.to_string()),
                    );
                }
                if summary.imported > 0 {
                    message.push_str(" · ");
                    message.push_str(self.loc().toast_import_disabled);
                }
                self.set_toast(message);
                crate::logging::info(
                    "SHARE",
                    &format!(
                        "Import applied: {} rule(s), {} overlay(s), {} preset(s), {} skipped",
                        summary.imported,
                        summary.overlays,
                        summary.macro_presets,
                        summary.skipped_duplicates
                    ),
                );
                self.daemon_sync.force();
            }
            Ok(IpcResponse::Error(error)) => {
                crate::logging::warn("SHARE", &format!("Import rejected by daemon: {error}"));
                self.set_toast(self.loc().toast_import_failed.replace("{}", &error));
            }
            Ok(_) => {
                self.set_toast(self.loc().toast_request_failed_unexpected);
            }
            Err(error) => {
                crate::logging::warn("SHARE", &format!("Import IPC failed: {error}"));
                self.set_toast(self.loc().toast_import_failed.replace("{}", &error));
            }
        }
    }

    /// Localized explanation of a decoding failure.
    pub fn share_error_text(&self, error: &crate::sharing::ShareError) -> String {
        use crate::sharing::ShareError as E;
        let loc = self.loc();
        match error {
            E::EmptyInput => loc.share_err_empty.to_string(),
            E::LegacyFormat => loc.share_err_legacy.to_string(),
            E::MissingPrefix => loc.share_err_missing_prefix.to_string(),
            E::CodeTooLong => loc.share_err_code_too_long.to_string(),
            E::InvalidBase64 => loc.share_err_invalid_base64.to_string(),
            E::CorruptedPayload => loc.share_err_corrupted.to_string(),
            E::ChecksumMismatch => loc.share_err_checksum.to_string(),
            E::PayloadTooLarge => loc.share_err_payload_too_large.to_string(),
            E::TooManyRules => loc.share_err_too_many_rules.to_string(),
            E::TooManyLibraryItems => loc.share_err_too_many_library_items.to_string(),
            E::EmptyPayload => loc.share_err_empty_payload.to_string(),
            E::InvalidPayload => loc.share_err_invalid_payload.to_string(),
            E::UnsupportedVersion(version) => {
                format!("{} (v{version})", loc.share_err_unsupported_version)
            }
        }
    }

    /// Localized line for a share-code notice.
    pub fn share_warning_text(&self, warning: &crate::sharing::ShareWarning) -> String {
        use crate::sharing::ShareWarning as W;
        let loc = self.loc();
        match warning {
            W::MissingMacroPreset(id) => loc.share_warn_missing_macro.replace("{}", id),
            W::MissingOverlay(id) => loc.share_warn_missing_overlay.replace("{}", id),
            W::SensitiveScript => loc.share_warn_script.to_string(),
            W::SensitiveAdminProcess => loc.share_warn_admin.to_string(),
            W::SensitiveWebhook => loc.share_warn_webhook.to_string(),
            W::CustomIconDropped => loc.share_warn_icon.to_string(),
            W::OverlayAssetDropped => loc.share_warn_asset.to_string(),
            W::MissingRule(id) => loc.share_warn_missing_rule.replace("{}", id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ipc::IpcJobResult;

    #[test]
    fn test_share_error_text_is_localized() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        assert_eq!(
            state.share_error_text(&crate::sharing::ShareError::LegacyFormat),
            crate::i18n::get(crate::i18n::Language::English).share_err_legacy
        );
        state.language = crate::i18n::Language::Russian;
        assert_eq!(
            state.share_error_text(&crate::sharing::ShareError::ChecksumMismatch),
            crate::i18n::get(crate::i18n::Language::Russian).share_err_checksum
        );
        let version = state.share_error_text(&crate::sharing::ShareError::UnsupportedVersion(9));
        assert!(version.contains('9'));
    }

    #[test]
    fn test_import_unexpected_response_toast_is_localized() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::Russian;
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::ImportShare,
            result: Ok(IpcResponse::Pong),
        });
        assert_eq!(
            state.toast.as_ref().expect("failure is reported").0.as_str(),
            crate::i18n::get(crate::i18n::Language::Russian).toast_request_failed_unexpected
        );
    }

    #[test]
    fn test_second_share_copy_queues_instead_of_dropping_the_running_export() {
        let mut state = GuiState::new();
        let (tx, rx) = std::sync::mpsc::channel();
        state.share.rx = Some(rx);

        state.start_share_export(vec![]);
        assert_eq!(state.share.queued.len(), 1, "the second copy is queued");
        assert!(state.share.rx.is_some(), "the running receiver is kept");

        // The first export ends without a result; the queued one takes over.
        drop(tx);
        state.poll_share_events(&egui::Context::default());
        assert!(state.share.queued.is_empty());
        assert!(state.share.rx.is_some(), "the queued export starts after the running one");
    }
}
