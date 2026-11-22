//! Updater glue: the worker event pump and the client-side advertisement grace that keeps a
//! fresh manual check from being erased by an in-flight daemon snapshot.

use std::time::Instant;

use crate::ipc::{IpcRequest, IpcResponse};

use super::ipc::{IpcJobKind, MutationFailure};
use super::GuiState;

/// How long a locally advertised release is protected from a daemon snapshot produced before
/// the client's `SetAvailableUpdate` notification reached the daemon.
const UPDATE_ADVERTISE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Updater worker channel and the client-side release advertisement bookkeeping.
pub(super) struct Updates {
    tx: std::sync::mpsc::Sender<crate::updater::UpdateEvent>,
    rx: std::sync::mpsc::Receiver<crate::updater::UpdateEvent>,
    /// When the client last changed the advertisement itself (manual check, cleared release).
    advertised_at: Option<Instant>,
}

impl Updates {
    pub(super) fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            tx,
            rx,
            advertised_at: None,
        }
    }

    /// Records a local change to the advertised release.
    fn advertise(&mut self) {
        self.advertised_at = Some(Instant::now());
    }

    /// The daemon is the authority, but an in-flight snapshot may predate our notification:
    /// only let it drop the local advertisement after that notification had time to arrive.
    pub(super) fn daemon_may_clear(&self) -> bool {
        self.advertised_at
            .is_none_or(|at| at.elapsed() >= UPDATE_ADVERTISE_GRACE)
    }
}

impl GuiState {
    /// Drops a release the daemon no longer advertises (nothing found by a manual check, the
    /// daemon restarted, the update was applied) so the footer and changelog stop offering it.
    pub(super) fn clear_available_update(&mut self) {
        self.updates.advertise();
        if self.available_update.take().is_some() {
            self.show_changelog_modal = false;
        }
    }

    pub fn check_for_updates(&mut self, is_manual: bool) {
        if self.is_checking_update || self.is_downloading_update {
            return;
        }
        self.is_checking_update = true;
        let tx = self.updates.tx.clone();
        let current_ver = env!("CARGO_PKG_VERSION").to_string();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(crate::updater::UpdateEvent::CheckComplete {
                        result: Box::new(Err(format!("Runtime error: {e}"))),
                        is_manual,
                    });
                    return;
                }
            };

            let result = rt.block_on(async {
                crate::updater::check_github_release(crate::updater::GITHUB_REPO, &current_ver).await
            });

            let _ = tx.send(crate::updater::UpdateEvent::CheckComplete {
                result: Box::new(result),
                is_manual,
            });
        });
    }

    pub fn start_update(&mut self, info: crate::updater::ReleaseInfo) {
        if self.is_downloading_update {
            return;
        }
        self.is_downloading_update = true;
        self.gui_closed_during_update = false;
        crate::updater::set_gui_closed_during_update(false);
        self.update_download_progress = 0.0;
        self.set_toast(self.loc().toast_update_downloading.replace("{}", &info.version));

        self.enqueue_ipc(IpcJobKind::StartUpdate, IpcRequest::StartUpdate, None);
    }

    /// The optimistic "downloading" flag is rolled back when the daemon never accepted the
    /// request, so the footer stops claiming progress that will never happen.
    pub(super) fn apply_start_update_result(&mut self, result: Result<IpcResponse, String>) {
        if let Some(failure) = Self::mutation_failure(result) {
            self.is_downloading_update = false;
            self.update_download_progress = 0.0;
            crate::logging::warn(
                "GUI",
                &format!("StartUpdate was not applied: {}", failure.diagnostic()),
            );
            self.set_toast(match &failure {
                MutationFailure::UnexpectedResponse(_) => {
                    self.loc().toast_update_failed_unexpected.to_string()
                }
                MutationFailure::Detail(_) => {
                    self.loc().toast_update_failed.replace("{}", failure.diagnostic())
                }
            });
        }
        self.daemon_sync.force();
    }

    pub fn poll_update_events(&mut self) {
        while let Ok(event) = self.updates.rx.try_recv() {
            match event {
                crate::updater::UpdateEvent::CheckStarted => {
                    self.is_checking_update = true;
                }
                crate::updater::UpdateEvent::CheckComplete { result, is_manual } => {
                    self.is_checking_update = false;
                    match *result {
                        Ok(Some(info)) => {
                            let ver = info.version.clone();
                            self.available_update = Some(info.clone());
                            self.updates.advertise();
                            self.enqueue_ipc(
                                IpcJobKind::Background,
                                IpcRequest::SetAvailableUpdate(Some(info)),
                                None,
                            );
                            if is_manual {
                                self.set_toast(self.loc().toast_update_available.replace("{}", &ver));
                            }
                        }
                        Ok(None) => {
                            self.enqueue_ipc(
                                IpcJobKind::Background,
                                IpcRequest::SetAvailableUpdate(None),
                                None,
                            );
                            // The manual check found nothing: stop offering the release the
                            // client remembered from an older daemon session.
                            self.clear_available_update();
                            if is_manual {
                                self.set_toast(self.loc().toast_latest_version.replace("{}", env!("CARGO_PKG_VERSION")));
                            }
                        }
                        Err(err) => {
                            if is_manual {
                                self.set_toast(self.loc().toast_update_check_failed.replace("{}", &err));
                            }
                        }
                    }
                }
                crate::updater::UpdateEvent::DownloadProgress(pct) => {
                    self.update_download_progress = pct;
                }
                crate::updater::UpdateEvent::DownloadComplete(result) => {
                    self.is_downloading_update = false;
                    match result {
                        Ok(()) => {
                            // The binary was swapped and PowerShell restart helper spawned.
                            // Exit GUI cleanly so PowerShell continues immediately.
                            std::process::exit(0);
                        }
                        Err(err) => {
                            self.set_toast(self.loc().toast_update_failed.replace("{}", &err));
                            if self.gui_closed_during_update {
                                std::process::exit(1);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::DaemonState;

    fn release_info() -> crate::updater::ReleaseInfo {
        crate::updater::ReleaseInfo {
            version: "9.9.9".into(),
            tag_name: "v9.9.9".into(),
            name: "v9.9.9".into(),
            body: "Notes".into(),
            html_url: "http://example.com".into(),
            portable: None,
            installer: None,
        }
    }

    fn daemon_state_json(value: serde_json::Value) -> DaemonState {
        serde_json::from_value(value).expect("daemon state deserializes")
    }

    /// I18N-GAP: the update path reports an unexpected response with its own localized line
    /// instead of embedding the English protocol detail in `toast_update_failed`.
    #[test]
    fn test_update_unexpected_response_toast_is_localized() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);

        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        state.apply_start_update_result(Ok(IpcResponse::Pong));
        let toast = state.toast.as_ref().expect("failure is reported").0.clone();
        assert_eq!(toast, en.toast_update_failed_unexpected);
        assert!(!toast.contains("Pong"), "the response name stays in the log only: {toast}");
        assert!(!toast.contains("{}"), "the generic update toast must not keep a placeholder: {toast}");

        state.language = crate::i18n::Language::Russian;
        state.apply_start_update_result(Ok(IpcResponse::Pong));
        assert_eq!(
            state.toast.as_ref().expect("failure is reported").0.as_str(),
            ru.toast_update_failed_unexpected
        );

        // A daemon rejection still carries its (already safe) detail through the generic line.
        state.language = crate::i18n::Language::English;
        state.apply_start_update_result(Ok(IpcResponse::Error("startup refused".into())));
        assert_eq!(
            state.toast.as_ref().expect("failure is reported").0.as_str(),
            en.toast_update_failed.replace("{}", "startup refused")
        );
    }

    #[test]
    fn test_manual_check_without_update_clears_the_stale_release() {
        let mut state = GuiState::new();
        state.available_update = Some(release_info());
        state.show_changelog_modal = true;

        state
            .updates
            .tx
            .send(crate::updater::UpdateEvent::CheckComplete {
                result: Box::new(Ok(None)),
                is_manual: true,
            })
            .expect("send check result");
        state.poll_update_events();

        assert!(state.available_update.is_none(), "the manual check found nothing");
        assert!(!state.show_changelog_modal, "the changelog must stop offering the release");
        assert!(state.toast.is_some());
    }

    #[test]
    fn test_daemon_state_without_update_clears_the_stale_release() {
        let mut state = GuiState::new();
        state.available_update = Some(release_info());
        state.show_changelog_modal = true;

        state.apply_daemon_state(daemon_state_json(serde_json::json!({
            "autorun_enabled": false,
            "uptime_secs": 0,
            "daemon_memory_bytes": 0,
            "daemon_cpu_usage": 0.0,
            "active_rules_count": 0,
            "total_rules_count": 0,
            "config": { "rules": [] },
            "logs": []
        })));

        assert!(state.available_update.is_none(), "the daemon is the authority on updates");
        assert!(!state.show_changelog_modal);
    }

    #[test]
    fn test_fresh_manual_result_survives_a_snapshot_taken_before_the_notification() {
        let mut state = GuiState::new();
        // The manual check just advertised a release; its notification is still in flight.
        state.available_update = Some(release_info());
        state.updates.advertise();

        state.apply_daemon_state(daemon_state_json(serde_json::json!({
            "autorun_enabled": false,
            "uptime_secs": 0,
            "daemon_memory_bytes": 0,
            "daemon_cpu_usage": 0.0,
            "active_rules_count": 0,
            "total_rules_count": 0,
            "config": { "rules": [] },
            "logs": []
        })));

        assert!(
            state.available_update.is_some(),
            "a stale snapshot must not erase the release the client just found"
        );
    }
}
