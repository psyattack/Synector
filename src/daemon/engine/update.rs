use crate::updater::ReleaseInfo;

use super::DaemonEngine;

/// Daemon-side updater display state: the release the tray/GUI offer, the download progress and
/// the two one-shot handshakes the GUI acknowledges. The tray keeps its own process-wide counters
/// for the icon/menu; this value is the source of truth projected into [`crate::ipc::DaemonState`].
pub(crate) struct UpdateStatus {
    available: Option<ReleaseInfo>,
    downloading: bool,
    progress: f32,
    status: Option<String>,
    pending_open_modal: bool,
    pending_auto_start: bool,
}

impl UpdateStatus {
    pub(super) fn new() -> Self {
        Self {
            available: None,
            downloading: false,
            progress: 0.0,
            status: None,
            pending_open_modal: false,
            pending_auto_start: false,
        }
    }

    pub(super) fn available(&self) -> Option<&ReleaseInfo> {
        self.available.as_ref()
    }

    pub(super) fn set_available(&mut self, info: Option<ReleaseInfo>) {
        self.available = info;
    }

    pub(super) fn begin_download(&mut self) {
        self.downloading = true;
        self.progress = 0.0;
    }

    pub(super) fn set_progress(&mut self, progress: f32) {
        self.progress = progress;
    }

    pub(super) fn set_status(&mut self, status: String) {
        self.status = Some(status);
    }

    /// Terminal download failure: the download is no longer running, but the last progress value is
    /// kept so the narrow overlay query only stops reporting progress (`downloading` is false).
    pub(super) fn fail(&mut self, error: String) {
        self.downloading = false;
        self.status = Some(error);
    }

    /// The updater pair `GetUpdateProgress` and [`DaemonState`](crate::ipc::DaemonState) project.
    pub(super) fn progress_snapshot(&self) -> (Option<f32>, Option<String>) {
        (
            self.downloading.then_some(self.progress),
            self.status.clone(),
        )
    }

    pub(super) fn pending_open_modal(&self) -> bool {
        self.pending_open_modal
    }

    pub(super) fn clear_pending_open_modal(&mut self) {
        self.pending_open_modal = false;
    }

    pub(super) fn request_open_modal(&mut self) {
        self.pending_open_modal = true;
    }

    pub(super) fn pending_auto_start(&self) -> bool {
        self.pending_auto_start
    }

    pub(super) fn clear_pending_auto_start(&mut self) {
        self.pending_auto_start = false;
    }
}

impl DaemonEngine {
    /// Installs the release discovered by the update check (also mirrored into the tray statics).
    pub(crate) fn set_available_update(&mut self, info: Option<ReleaseInfo>) {
        self.update.set_available(info);
    }

    /// Starts a daemon-side download: the tray hands the status strings in, so the tray wording
    /// stays in one place.
    pub(crate) fn begin_update_download(&mut self) {
        self.update.begin_download();
    }

    pub(crate) fn set_update_progress(&mut self, progress: f32) {
        self.update.set_progress(progress);
    }

    pub(crate) fn set_update_status(&mut self, status: String) {
        self.update.set_status(status);
    }

    /// Terminal failure as reported by the tray download task (`Error: {err}` is formatted there).
    pub(crate) fn fail_update(&mut self, error: String) {
        self.update.fail(error);
    }

    /// Asks the GUI to open the changelog/update modal on its next sync.
    pub(crate) fn request_open_update_modal(&mut self) {
        self.update.request_open_modal();
    }
}
