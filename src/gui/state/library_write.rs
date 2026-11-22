//! The preset-library write contract shared by the Overlay Studio and the Macro Studio.
//!
//! While the daemon is reachable it is the only writer: a rejection or an unexpected answer is
//! reported instead of being overwritten behind its back. A direct `library.json` write happens
//! only after a confirmed-unreachable probe (U02-008).

use crate::models::OverlayDefinition;

/// One queued preset-library operation. It is executed on the IPC worker, so the frame thread
/// never waits for the daemon (or for the offline direct write) during rendering.
#[derive(Clone)]
pub(crate) enum LibraryWriteRequest {
    SaveOverlay(Box<OverlayDefinition>),
    DeleteOverlay(String),
    SaveMacroPreset(Box<crate::models::MacroPreset>),
    DeleteMacroPreset(String),
}

/// A queued library write plus the preset name used by the success toast.
pub(crate) struct PendingLibraryWrite {
    pub(crate) request: LibraryWriteRequest,
    pub(crate) name: String,
}

/// Runs one queued library operation on the IPC worker.
pub(crate) fn execute(request: &LibraryWriteRequest) -> LibraryWriteOutcome {
    match request {
        LibraryWriteRequest::SaveOverlay(definition) => save_overlay_preset(definition),
        LibraryWriteRequest::DeleteOverlay(overlay_id) => delete_overlay_preset(overlay_id),
        LibraryWriteRequest::SaveMacroPreset(preset) => save_macro_preset(preset),
        LibraryWriteRequest::DeleteMacroPreset(preset_id) => delete_macro_preset(preset_id),
    }
}

/// Log line for the confirmed-offline direct write; the overlay and macro paths keep their
/// existing wording.
const OVERLAY_LIBRARY_LOG: &str = "Daemon unreachable; overlay library written directly";
const MACRO_LIBRARY_LOG: &str = "Daemon unreachable; macro library written directly";

/// Result of one overlay-library mutation attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LibraryWriteOutcome {
    /// The daemon, or a confirmed-offline direct write, persisted the change.
    Persisted,
    /// Nothing was persisted. The string is a log-safe reason (identifiers only, no payloads).
    Failed(String),
}

/// Whether a failed IPC call may be retried as a direct write of `library.json`. The daemon
/// owns the section while it is reachable, so only a confirmed-unreachable daemon may be
/// bypassed: a reachable daemon that rejected the write (or answered unexpectedly) must be
/// reported instead, or the next daemon persist overwrites our entry (U02-008).
pub(crate) fn direct_library_write_allowed(daemon_reachable: bool) -> bool {
    !daemon_reachable
}

/// Interprets one IPC result and, when the daemon is confirmed gone, applies the change
/// directly through `apply_directly`.
pub(crate) fn finish_library_write(
    result: Result<crate::ipc::IpcResponse, String>,
    daemon_reachable: bool,
    log_target: &'static str,
    apply_directly: impl FnOnce() -> std::io::Result<()>,
) -> LibraryWriteOutcome {
    match result {
        Ok(crate::ipc::IpcResponse::Ok) => LibraryWriteOutcome::Persisted,
        Ok(crate::ipc::IpcResponse::Error(reason)) => {
            LibraryWriteOutcome::Failed(format!("the daemon rejected the change: {reason}"))
        }
        Ok(_) => LibraryWriteOutcome::Failed("the daemon returned an unexpected response".to_string()),
        Err(error) => {
            if !direct_library_write_allowed(daemon_reachable) {
                return LibraryWriteOutcome::Failed(format!("the daemon is reachable but the change failed: {error}"));
            }
            match apply_directly() {
                Ok(()) => {
                    crate::logging::info("CONFIG", log_target);
                    LibraryWriteOutcome::Persisted
                }
                Err(write_error) => LibraryWriteOutcome::Failed(format!("direct library write failed: {write_error}")),
            }
        }
    }
}

/// Deletes one preset, keeping the daemon authoritative while it is reachable (U02-008).
pub(crate) fn delete_overlay_preset(id: &str) -> LibraryWriteOutcome {
    let result = crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::DeleteOverlay(id.to_string()));
    let daemon_reachable = result.is_err() && crate::ipc::IpcClient::is_daemon_running();
    let outcome = finish_library_write(result, daemon_reachable, OVERLAY_LIBRARY_LOG, || {
        let mut library = crate::config::Library::read();
        library.overlays.retain(|o| o.id != id);
        library.save()
    });
    if let LibraryWriteOutcome::Failed(reason) = &outcome {
        crate::logging::error("CONFIG", &format!("Overlay preset '{id}' was not deleted: {reason}"));
    }
    outcome
}

/// Saves one preset definition, keeping the daemon authoritative while it is reachable
/// (U02-008). The daemon persists synchronously before answering `Ok`, so no fallback write is
/// attempted while it is alive.
pub(crate) fn save_overlay_preset(def: &OverlayDefinition) -> LibraryWriteOutcome {
    let result = crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::SaveOverlay(def.clone()));
    let daemon_reachable = result.is_err() && crate::ipc::IpcClient::is_daemon_running();
    let outcome = finish_library_write(result, daemon_reachable, OVERLAY_LIBRARY_LOG, || {
        let mut library = crate::config::Library::read();
        if let Some(existing) = library.overlays.iter_mut().find(|o| o.id == def.id) {
            *existing = def.clone();
        } else {
            library.overlays.push(def.clone());
        }
        library.save()
    });
    if let LibraryWriteOutcome::Failed(reason) = &outcome {
        crate::logging::error("CONFIG", &format!("Overlay preset '{}' was not saved: {reason}", def.id));
    }
    outcome
}

/// Deletes one macro preset, keeping the daemon authoritative while it is reachable.
pub(crate) fn delete_macro_preset(id: &str) -> LibraryWriteOutcome {
    let result = crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::DeleteMacroPreset(id.to_string()));
    let daemon_reachable = result.is_err() && crate::ipc::IpcClient::is_daemon_running();
    let outcome = finish_library_write(result, daemon_reachable, MACRO_LIBRARY_LOG, || {
        let mut library = crate::config::Library::read();
        library.macro_presets.retain(|preset| preset.id != id);
        library.save()
    });
    if let LibraryWriteOutcome::Failed(reason) = &outcome {
        crate::logging::error(
            "CONFIG",
            &format!("Macro preset '{id}' was not deleted: {reason}"),
        );
    }
    outcome
}

/// Saves one macro preset, keeping the daemon authoritative while it is reachable. The daemon
/// persists synchronously before answering `Ok`, so no fallback write is attempted while it lives.
pub(crate) fn save_macro_preset(preset: &crate::models::MacroPreset) -> LibraryWriteOutcome {
    let result =
        crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::SaveMacroPreset(preset.clone()));
    let daemon_reachable = result.is_err() && crate::ipc::IpcClient::is_daemon_running();
    let outcome = finish_library_write(result, daemon_reachable, MACRO_LIBRARY_LOG, || {
        let mut library = crate::config::Library::read();
        if let Some(existing) = library.macro_presets.iter_mut().find(|p| p.id == preset.id) {
            *existing = preset.clone();
        } else {
            library.macro_presets.push(preset.clone());
        }
        library.save()
    });
    if let LibraryWriteOutcome::Failed(reason) = &outcome {
        crate::logging::error(
            "CONFIG",
            &format!("Macro preset '{}' was not saved: {reason}", preset.id),
        );
    }
    outcome
}

impl super::GuiState {
    /// Queues a preset save; the result handler updates the snapshot and shows the toast.
    pub fn request_save_overlay_preset(&mut self, definition: OverlayDefinition, name: String) {
        let key = self.next_library_write_key;
        self.next_library_write_key += 1;
        let request = LibraryWriteRequest::SaveOverlay(Box::new(definition));
        self.library_writes.insert(
            key,
            PendingLibraryWrite {
                request: request.clone(),
                name,
            },
        );
        self.enqueue_library_write(key, request);
    }

    /// Queues a preset delete; success removes it from the local snapshot.
    pub fn request_delete_overlay_preset(&mut self, overlay_id: &str) {
        let key = self.next_library_write_key;
        self.next_library_write_key += 1;
        let request = LibraryWriteRequest::DeleteOverlay(overlay_id.to_string());
        self.library_writes.insert(
            key,
            PendingLibraryWrite {
                request: request.clone(),
                name: String::new(),
            },
        );
        self.enqueue_library_write(key, request);
    }

    /// Queues a macro-preset save; the result handler updates the snapshot and shows the toast.
    pub fn request_save_macro_preset(
        &mut self,
        preset: crate::models::MacroPreset,
        name: String,
    ) {
        let key = self.next_library_write_key;
        self.next_library_write_key += 1;
        let request = LibraryWriteRequest::SaveMacroPreset(Box::new(preset));
        self.library_writes.insert(
            key,
            PendingLibraryWrite {
                request: request.clone(),
                name,
            },
        );
        self.enqueue_library_write(key, request);
    }

    /// Queues a macro-preset delete; success removes it from the local snapshot.
    pub fn request_delete_macro_preset(&mut self, preset_id: &str) {
        let key = self.next_library_write_key;
        self.next_library_write_key += 1;
        let request = LibraryWriteRequest::DeleteMacroPreset(preset_id.to_string());
        self.library_writes.insert(
            key,
            PendingLibraryWrite {
                request: request.clone(),
                name: String::new(),
            },
        );
        self.enqueue_library_write(key, request);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The log target is irrelevant to the outcome assertions.
    const LOG: &str = "test log target";

    #[test]
    fn test_library_write_outcome_keeps_a_reachable_daemon_authoritative() {
        use std::cell::Cell;

        let applied = Cell::new(0usize);

        // Daemon applied the change: no direct write.
        assert_eq!(
            finish_library_write(Ok(crate::ipc::IpcResponse::Ok), true, LOG, || {
                applied.set(applied.get() + 1);
                Ok(())
            }),
            LibraryWriteOutcome::Persisted
        );
        assert_eq!(applied.get(), 0);

        // Daemon rejected the change: reported, never written behind its back.
        let outcome = finish_library_write(Ok(crate::ipc::IpcResponse::Error("denied".to_string())), true, LOG, || {
            applied.set(applied.get() + 1);
            Ok(())
        });
        assert!(matches!(outcome, LibraryWriteOutcome::Failed(_)));
        assert_eq!(applied.get(), 0);

        // Unexpected response: same treatment.
        let outcome = finish_library_write(Ok(crate::ipc::IpcResponse::Pong), true, LOG, || {
            applied.set(applied.get() + 1);
            Ok(())
        });
        assert!(matches!(outcome, LibraryWriteOutcome::Failed(_)));
        assert_eq!(applied.get(), 0);

        // Transport error but the daemon is still alive: reported, no second writer.
        let outcome = finish_library_write(Err("pipe closed".to_string()), true, LOG, || {
            applied.set(applied.get() + 1);
            Ok(())
        });
        assert!(matches!(outcome, LibraryWriteOutcome::Failed(_)));
        assert_eq!(applied.get(), 0);

        // Confirmed-offline daemon: the direct write is allowed and counted.
        let outcome = finish_library_write(Err("pipe closed".to_string()), false, LOG, || {
            applied.set(applied.get() + 1);
            Ok(())
        });
        assert_eq!(outcome, LibraryWriteOutcome::Persisted);
        assert_eq!(applied.get(), 1);

        // A failing direct write is surfaced instead of a success toast.
        let outcome = finish_library_write(Err("pipe closed".to_string()), false, LOG, || {
            Err(std::io::Error::other("disk full"))
        });
        assert!(matches!(outcome, LibraryWriteOutcome::Failed(_)));
    }

    #[test]
    fn test_direct_macro_library_write_is_allowed_only_when_the_daemon_is_unreachable() {
        assert!(
            !direct_library_write_allowed(true),
            "a reachable daemon owns library.json"
        );
        assert!(direct_library_write_allowed(false));
    }

    /// The Macro Studio delete/save paths must keep the daemon authoritative: an `Ok` is final, a
    /// rejection or unexpected answer is reported instead of writing `library.json` behind it, and
    /// only a confirmed-offline daemon falls back to a direct write (U02-008, same rule as the
    /// overlay studio).
    #[test]
    fn test_macro_library_write_keeps_a_reachable_daemon_authoritative() {
        let mut direct_writes = 0;

        let outcome = finish_library_write(Ok(crate::ipc::IpcResponse::Ok), true, LOG, || {
            direct_writes += 1;
            Ok(())
        });
        assert_eq!(outcome, LibraryWriteOutcome::Persisted);
        assert_eq!(direct_writes, 0, "a daemon Ok is final");

        let outcome = finish_library_write(
            Ok(crate::ipc::IpcResponse::Error("denied".to_string())),
            true,
            LOG,
            || {
                direct_writes += 1;
                Ok(())
            },
        );
        assert!(matches!(outcome, LibraryWriteOutcome::Failed(_)));
        assert_eq!(direct_writes, 0, "a rejection must not be overwritten directly");

        let outcome = finish_library_write(Ok(crate::ipc::IpcResponse::Pong), true, LOG, || {
            direct_writes += 1;
            Ok(())
        });
        assert!(matches!(outcome, LibraryWriteOutcome::Failed(_)));
        assert_eq!(direct_writes, 0, "an unexpected answer is not a success");

        let outcome = finish_library_write(Err("pipe busy".to_string()), true, LOG, || {
            direct_writes += 1;
            Ok(())
        });
        assert!(matches!(outcome, LibraryWriteOutcome::Failed(_)));
        assert_eq!(direct_writes, 0, "a reachable daemon that failed is reported");

        let outcome = finish_library_write(Err("pipe closed".to_string()), false, LOG, || {
            direct_writes += 1;
            Ok(())
        });
        assert_eq!(outcome, LibraryWriteOutcome::Persisted);
        assert_eq!(direct_writes, 1, "a confirmed-offline daemon allows the fallback");

        let outcome = finish_library_write(Err("pipe closed".to_string()), false, LOG, || {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "locked",
            ))
        });
        match outcome {
            LibraryWriteOutcome::Failed(reason) => {
                assert!(reason.contains("direct library write failed"));
            }
            LibraryWriteOutcome::Persisted => {
                panic!("a failed direct write must not be reported as success")
            }
        }
    }
}
