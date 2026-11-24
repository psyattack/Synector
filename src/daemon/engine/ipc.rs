use std::collections::{HashMap, HashSet};

use crate::daemon::actions::ActionRunner;
use crate::daemon::watcher::ProcessWatcher;
use crate::ipc::{DaemonState, IpcRequest, IpcResponse};
use crate::models::{Action, Rule, ScheduleType, Trigger};

use super::executions::ExecutionOrigin;
use super::projector::StateProjector;
use super::quota::adopt_rule_runtime;
use super::DaemonEngine;

impl DaemonEngine {
    pub async fn handle_request(&mut self, req: IpcRequest) -> IpcResponse {
        self.drain_action_events();
        match req {
            IpcRequest::Ping => IpcResponse::Pong,

            IpcRequest::GetState => {
                let state = self.get_current_state();
                IpcResponse::State(state)
            }

            // Narrow answer for the analytics dialog: the collector's rolling snapshot, without
            // projecting the whole config and log stream on every dashboard refresh.
            IpcRequest::GetAnalytics => IpcResponse::Analytics(self.analytics_snapshot()),

            // Narrow answer for the standalone update-progress overlay: two updater fields, no
            // `DaemonState` projection (config and logs stay where they are).
            IpcRequest::GetUpdateProgress => {
                let (progress, status) = self.update_progress_snapshot();
                IpcResponse::UpdateProgress { progress, status }
            }

            IpcRequest::SaveSettings(settings) => {
                let tray_changed = self.config.tray_icon_path != settings.tray_icon_path;
                let new_tray_path = settings.tray_icon_path.clone();
                let log_to_disk = settings.log_to_disk;
                // Window geometry is owned by the daemon through UpdateWindowState; a
                // settings snapshot from the GUI may be older, so it never overrides it.
                let mut settings = settings;
                settings.window = self.config.window.clone();
                self.config.apply_settings(settings);
                self.persist_settings();
                // Applies the in-memory preference: a failed save still stops this process from
                // writing, and the next daemon start reads whatever reached the file.
                crate::logging::set_disk_logging_enabled(log_to_disk);
                if tray_changed {
                    crate::daemon::tray::update_tray_icon(new_tray_path);
                }
                crate::daemon::tray::set_tray_visible(!self.config.hide_tray_icon);
                self.log("INFO", "CONFIG", "Settings updated and saved");
                IpcResponse::Ok
            }

            IpcRequest::SaveRule(rule) => self.ipc_save_rule(rule),

            IpcRequest::ReorderRules(order) => {
                let mut remaining = std::mem::take(&mut self.config.rules);
                let mut ordered = Vec::with_capacity(remaining.len());
                for id in order {
                    if let Some(position) = remaining.iter().position(|rule| rule.id == id) {
                        ordered.push(remaining.remove(position));
                    }
                }
                ordered.append(&mut remaining);
                self.config.rules = ordered;
                let saved = self.persist_rules();
                self.refresh_live_rules();
                if !saved {
                    return Self::persist_failure("rules");
                }
                IpcResponse::Ok
            }

            IpcRequest::ResetRule(id) => {
                let Some(rule) = self.config.rules.iter_mut().find(|r| r.id == id) else {
                    return IpcResponse::Error(format!("rule \"{id}\" not found"));
                };
                rule.clear_quota_runtime();
                rule.is_enabled = true;
                let saved = self.persist_rules();
                self.refresh_live_rules();
                self.mark_runtime_dirty();
                self.persist_runtime_now();
                if let Some(h) = &self.hotkey_handle {
                    h.reload(&self.config.rules);
                }
                if !saved {
                    return Self::persist_failure("rules");
                }
                self.log("INFO", "RULE", &format!("Reset rule '{id}': counter cleared, rule enabled"));
                IpcResponse::Ok
            }

            IpcRequest::DeleteRule(id) => {
                // The rule is going away: aborting (not pausing) is the only thing left to do.
                self.abort_rule_executions(&id);
                if let Some(rule) = self.config.rules.iter().find(|r| r.id == id) {
                    Self::hide_rule_overlays(rule);
                }
                self.config.rules.retain(|r| r.id != id);
                self.rule_last_triggered.remove(&id);
                self.scheduler.forget_rule(&id);
                let saved = self.persist_rules();
                self.refresh_live_rules();
                self.mark_runtime_dirty();
                self.persist_runtime_now();
                if let Some(h) = &self.hotkey_handle {
                    h.reload(&self.config.rules);
                }
                if !saved {
                    return Self::persist_failure("rules");
                }
                IpcResponse::Ok
            }

            IpcRequest::ToggleRule { id, enabled } => {
                if !enabled {
                    self.abort_rule_executions(&id);
                }
                if let Some(rule) = self.config.rules.iter_mut().find(|r| r.id == id) {
                    rule.is_enabled = enabled;
                }
                let saved = self.persist_rules();
                self.refresh_live_rules();
                if let Some(h) = &self.hotkey_handle {
                    h.reload(&self.config.rules);
                }
                if !saved {
                    return Self::persist_failure("rules");
                }
                IpcResponse::Ok
            }

            IpcRequest::TestRunRule { id, rule } => {
                let target_rule = if let Some(r) = rule {
                    *r
                } else if let Some(r) = self.config.rules.iter().find(|r| r.id == id).cloned() {
                    r
                } else {
                    return IpcResponse::Error("Rule not found".into());
                };
                let _ = self.emulate_rule_trigger(&target_rule).await;
                self.drain_action_events();
                IpcResponse::Ok
            }

            IpcRequest::StopRuleExecution(id) => {
                self.stop_rule_execution(&id);
                IpcResponse::Ok
            }

            IpcRequest::GetProcesses => {
                let list = ProcessWatcher::get_detailed_snapshot();
                IpcResponse::Processes(list)
            }

            IpcRequest::KillProcess { pid, force } => {
                let mut cmd = std::process::Command::new("taskkill");
                if force {
                    cmd.arg("/F");
                }
                cmd.args(["/PID", &pid.to_string()]);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x08000000);
                }
                let _ = cmd.spawn();
                IpcResponse::Ok
            }

            IpcRequest::SetAutorun(enable) => {
                match crate::autorun::set_autorun(crate::autorun::APP_NAME, enable) {
                    Ok(()) => {
                        self.log("INFO", "CONFIG", &format!("Autorun set to: {enable}"));
                        IpcResponse::Ok
                    }
                    Err(err) => {
                        self.log("ERROR", "CONFIG", &format!("Failed to set autorun: {err}"));
                        IpcResponse::Error(err)
                    }
                }
            }

            IpcRequest::UpdateWindowState(ws) => {
                self.config.window = Some(ws);
                self.persist_settings();
                IpcResponse::Ok
            }

            IpcRequest::ImportRules(imported) => self.ipc_import_rules(imported),

            IpcRequest::ImportShare { payload, enable } => self.ipc_import_share(*payload, enable),

            IpcRequest::SyncRules(incoming) => self.ipc_sync_rules(incoming),

            // The only production path for `TestStep` is `daemon::ipc_server::dispatch`, which
            // runs the action *before* taking the engine lock: a wait or a script executed here
            // would stall the scheduler and every other IPC request for the action's duration.
            // The arm stays only to keep the match exhaustive; reaching it means a caller
            // bypassed the dispatcher, so it must never run the action.
            IpcRequest::TestStep { .. } => IpcResponse::Error(
                "TestStep must be dispatched before the engine lock is taken".to_string(),
            ),

            IpcRequest::SaveOverlay(overlay) => {
                if let Some(existing) = self.config.overlays.iter_mut().find(|o| o.id == overlay.id) {
                    *existing = overlay;
                } else {
                    self.config.overlays.push(overlay);
                }
                self.persist_library();
                IpcResponse::Ok
            }

            IpcRequest::DeleteOverlay(id) => {
                crate::daemon::overlay_manager::hide_overlay(&id);
                self.config.overlays.retain(|o| o.id != id && !o.name.eq_ignore_ascii_case(&id));
                self.persist_library();
                IpcResponse::Ok
            }

            IpcRequest::SaveMacroPreset(preset) => {
                if let Some(existing) = self.config.macro_presets.iter_mut().find(|p| p.id == preset.id) {
                    *existing = preset;
                } else {
                    self.config.macro_presets.push(preset);
                }
                self.persist_library();
                IpcResponse::Ok
            }

            IpcRequest::DeleteMacroPreset(id) => {
                self.config.macro_presets.retain(|p| p.id != id && !p.name.eq_ignore_ascii_case(&id));
                self.persist_library();
                IpcResponse::Ok
            }

            IpcRequest::ShowOverlayPreview { overlay, calibration_mode } => {
                crate::daemon::overlay_manager::show_overlay(&overlay, calibration_mode, None);
                IpcResponse::Ok
            }

            IpcRequest::HideOverlayPreview(id) => {
                crate::daemon::overlay_manager::hide_overlay(&id);
                IpcResponse::Ok
            }

            IpcRequest::GetOverlayPosition(id) => {
                let pos = crate::daemon::overlay_manager::get_calibrated_position(&id);
                IpcResponse::OverlayPosition(pos)
            }

            IpcRequest::UpdateOverlayPosition { id, x, y } => {
                crate::daemon::overlay_manager::save_calibrated_position(&id, x, y);
                let monitors = crate::daemon::overlay_manager::get_system_monitors();
                let mut library_changed = false;
                let mut rules_changed = false;
                if let Some(ov) = self.config.overlays.iter_mut().find(|o| o.id == id) {
                    let mon = ov.monitor_index
                        .and_then(|idx| monitors.iter().find(|m| m.index == idx))
                        .or_else(|| monitors.iter().find(|m| m.is_primary));
                    let (mon_x, mon_y) = mon.map(|m| (m.x as f32, m.y as f32)).unwrap_or((0.0, 0.0));
                    ov.rect.x = x - mon_x;
                    ov.rect.y = y - mon_y;
                    library_changed = true;
                }
                for rule in &mut self.config.rules {
                    for action in &mut rule.actions {
                        if let Action::Overlay { overlay_id, custom_overlay: Some(ov), .. } = action {
                            if overlay_id == &id || ov.id == id {
                                let mon = ov.monitor_index
                                    .and_then(|idx| monitors.iter().find(|m| m.index == idx))
                                    .or_else(|| monitors.iter().find(|m| m.is_primary));
                                let (mon_x, mon_y) = mon.map(|m| (m.x as f32, m.y as f32)).unwrap_or((0.0, 0.0));
                                ov.rect.x = x - mon_x;
                                ov.rect.y = y - mon_y;
                                rules_changed = true;
                            }
                        }
                    }
                }
                self.mark_overlay_position_dirty(library_changed, rules_changed);
                IpcResponse::Ok
            }

            IpcRequest::SetAvailableUpdate(info) => {
                self.update.set_available(info.clone());
                crate::daemon::tray::set_available_update(info);
                IpcResponse::Ok
            }

            IpcRequest::AcknowledgeOpenUpdateModal => {
                self.update.clear_pending_open_modal();
                IpcResponse::Ok
            }

            IpcRequest::TriggerUpdate => {
                crate::daemon::tray::trigger_update_flow(false);
                IpcResponse::Ok
            }

            IpcRequest::StartUpdate => {
                crate::daemon::tray::start_update_download(false);
                IpcResponse::Ok
            }

            IpcRequest::AcknowledgeAutoStartUpdate => {
                self.update.clear_pending_auto_start();
                IpcResponse::Ok
            }

            IpcRequest::ShutdownDaemon => {
                // Hand the exit to the daemon main thread instead of ending the process here: the
                // coordinated shutdown runs the On App Close rules and flushes the coalesced
                // runtime counters and overlay positions before the process ends.
                crate::daemon::actions::release_all_simulated_inputs();
                crate::daemon::overlay_manager::hide_all();
                crate::daemon::request_shutdown();
                IpcResponse::Ok
            }
        }
    }

    /// Executes one `TestStep` action for the GUI. It deliberately takes no `&self`: the IPC
    /// server runs it before taking the engine lock, so a wait or a script cannot stall the
    /// scheduler and every other IPC request.
    pub async fn execute_test_step(
        action: &Action,
        context: &HashMap<String, String>,
    ) -> Result<crate::daemon::actions::ActionOutcome, String> {
        ActionRunner::execute(action, context).await
    }

    /// Records a step-test outcome. Script errors embed stdout/stderr, so they stay in the in-app
    /// log while the daily file log gets a redacted line.
    pub fn record_test_step(
        &mut self,
        outcome: Result<crate::daemon::actions::ActionOutcome, String>,
    ) -> IpcResponse {
        let result = match outcome {
            Ok(crate::daemon::actions::ActionOutcome::Continue(_)) => {
                self.log("INFO", "TEST", "Step test executed successfully");
                Ok(())
            }
            Ok(crate::daemon::actions::ActionOutcome::AbortPipeline { reason }) => {
                self.log("INFO", "TEST", &format!("Pipeline check halted: {reason}"));
                Ok(())
            }
            Err(err) => {
                self.log_with_file_message(
                    "ERROR",
                    "TEST",
                    &format!("Step test failed: {err}"),
                    Some("Step test failed (details omitted from the file log)"),
                );
                Err(err)
            }
        };
        IpcResponse::TestStepResult(result)
    }

    /// Applies a GUI draft of one rule. Runtime counters stay with the daemon, and a replaced
    /// definition may not keep an old recurring test loop running. The shared validator runs
    /// before anything is applied, so a rejected draft leaves the stored set untouched.
    fn ipc_save_rule(&mut self, mut rule: Rule) -> IpcResponse {
        let errors = crate::validation::validate_rules(std::slice::from_ref(&rule));
        if !errors.is_empty() {
            return IpcResponse::Error(format!(
                "rejected invalid rule: {}",
                errors.join("; ")
            ));
        }
        let rule_id = rule.id.clone();
        let mut runtime_reset = false;
        if let Some(existing) = self.config.rules.iter_mut().find(|r| r.id == rule.id) {
            // Counters belong to the daemon: the GUI edits drafts, it does not
            // own execution history.
            runtime_reset = adopt_rule_runtime(&mut rule, existing);
            *existing = rule;
        } else {
            // The GUI draft may carry counters copied from another rule; a newly
            // created rule starts with a clean runtime window.
            rule.clear_quota_runtime();
            self.config.rules.push(rule);
        }
        self.abort_recurring_test(&rule_id);
        let saved = self.persist_rules();
        self.refresh_live_rules();
        self.flush_quota_reset(runtime_reset);
        if let Some(h) = &self.hotkey_handle {
            h.reload(&self.config.rules);
        }
        if !saved {
            return Self::persist_failure("rules");
        }
        IpcResponse::Ok
    }

    /// Merges a validated import file into the rule set.
    fn ipc_import_rules(&mut self, imported: Vec<Rule>) -> IpcResponse {
        let errors = crate::validation::validate_rules(&imported);
        if !errors.is_empty() {
            return IpcResponse::Error(format!(
                "rejected invalid rules: {}",
                errors.join("; ")
            ));
        }
        let mut count = 0;
        let mut runtime_reset = false;
        for mut rule in imported {
            if let Some(existing) = self.config.rules.iter_mut().find(|r| r.id == rule.id) {
                // Imported definitions never carry execution history: counters stay
                // with the daemon and the rule keeps running where it left off.
                runtime_reset |= adopt_rule_runtime(&mut rule, existing);
                *existing = rule;
            } else {
                // A new rule starts with a clean quota window even if the file
                // carried counter fields; runtime.json owns them.
                rule.clear_quota_runtime();
                self.config.rules.push(rule);
            }
            count += 1;
        }
        let saved = self.persist_rules();
        self.refresh_live_rules();
        self.flush_quota_reset(runtime_reset);
        if let Some(h) = &self.hotkey_handle {
            h.reload(&self.config.rules);
        }
        if !saved {
            return Self::persist_failure("rules");
        }
        self.log("INFO", "CONFIG", &format!("Imported {count} automations"));
        IpcResponse::Ok
    }

    /// Adds a decoded share code together with the library presets and overlays it references.
    fn ipc_import_share(
        &mut self,
        payload: crate::sharing::SharePayload,
        enable: bool,
    ) -> IpcResponse {
        let prepared = match crate::sharing::prepare_import(
            payload,
            &self.config.rules,
            &self.config.overlays,
            &self.config.macro_presets,
            enable,
        ) {
            Ok(prepared) => prepared,
            Err(err) => return IpcResponse::Error(crate::sharing::error_text(&err).to_string()),
        };
        let errors = crate::validation::validate_rules(&prepared.rules);
        if !errors.is_empty() {
            return IpcResponse::Error(format!(
                "share import rejected: {}",
                errors.join("; ")
            ));
        }
        let summary = crate::sharing::ShareImportSummary {
            imported: prepared.rules.len(),
            skipped_duplicates: prepared.skipped_duplicates,
            overlays: prepared.overlays.len(),
            macro_presets: prepared.macro_presets.len(),
            enabled: enable,
        };
        if !prepared.has_new_content {
            return IpcResponse::ShareImportResult(summary);
        }
        for overlay in prepared.overlays {
            self.config.overlays.push(overlay);
        }
        for preset in prepared.macro_presets {
            self.config.macro_presets.push(preset);
        }
        for rule in prepared.rules {
            self.config.rules.push(rule);
        }
        let library_saved = if summary.overlays > 0 || summary.macro_presets > 0 {
            self.persist_library()
        } else {
            true
        };
        let rules_saved = self.persist_rules();
        self.refresh_live_rules();
        self.mark_runtime_dirty();
        if let Some(h) = &self.hotkey_handle {
            h.reload(&self.config.rules);
        }
        if !rules_saved || !library_saved {
            return Self::persist_failure("the imported rules and presets");
        }
        self.log(
            "INFO",
            "CONFIG",
            &format!(
                "Imported share code: {} automation(s), {} duplicate(s) skipped, {} overlay(s), {} preset(s)",
                summary.imported,
                summary.skipped_duplicates,
                summary.overlays,
                summary.macro_presets
            ),
        );
        IpcResponse::ShareImportResult(summary)
    }

    /// Declarative replace: the file is the full desired rule set. Counters of surviving rules
    /// stay with the daemon; removed rules lose theirs on the runtime flush below. One persist +
    /// one hotkey reload for the whole sync.
    fn ipc_sync_rules(&mut self, incoming: Vec<Rule>) -> IpcResponse {
        // The file is the whole desired set, so every rule and the duplicate-id check are
        // validated together; a rejected sync leaves the stored rules untouched.
        let errors = crate::validation::validate_rules(&incoming);
        if !errors.is_empty() {
            return IpcResponse::Error(format!(
                "rejected invalid rule set: {}",
                errors.join("; ")
            ));
        }
        let mut synced = Vec::with_capacity(incoming.len());
        let mut runtime_reset = false;
        for mut rule in incoming {
            if let Some(existing) = self.config.rules.iter().find(|r| r.id == rule.id) {
                runtime_reset |= adopt_rule_runtime(&mut rule, existing);
            } else {
                // Sync files define rules, not execution history.
                rule.clear_quota_runtime();
            }
            synced.push(rule);
        }
        // Stop what the replaced set still has running. A surviving rule may get a new pipeline
        // under its running executions, and a vanished rule has nothing left to own its pipelines
        // or bound companions.
        let incoming_ids: HashSet<&str> = synced.iter().map(|r| r.id.as_str()).collect();
        let removed: Vec<String> = self.config.rules.iter()
            .filter(|r| !incoming_ids.contains(r.id.as_str()))
            .map(|r| r.id.clone())
            .collect();
        let replaced: Vec<String> = self.config.rules.iter()
            .filter(|r| incoming_ids.contains(r.id.as_str()))
            .map(|r| r.id.clone())
            .collect();
        for id in replaced.iter().chain(removed.iter()) {
            self.abort_recurring_test(id);
        }
        for id in &removed {
            self.abort_rule_executions(id);
            self.rule_last_triggered.remove(id);
            self.scheduler.forget_rule(id);
        }

        let count = synced.len();
        self.config.rules = synced;
        let saved = self.persist_rules();
        self.refresh_live_rules();
        self.flush_quota_reset(runtime_reset);
        self.mark_runtime_dirty();
        self.persist_runtime_now();
        if let Some(h) = &self.hotkey_handle {
            h.reload(&self.config.rules);
        }
        if !saved {
            return Self::persist_failure("rules");
        }
        self.log("INFO", "CONFIG", &format!("Synchronized {count} rules"));
        IpcResponse::Ok
    }

    pub async fn execute_shutdown_rules(&mut self) {
        let rules = self.config.rules.clone();
        let mut handles = Vec::new();
        for rule in &rules {
            if rule.is_enabled {
                if let Some(Trigger::Schedule {
                    schedule_type: ScheduleType::AppShutdown,
                    ..
                }) = &rule.trigger
                {
                    if let Some(handle) = self.trigger(rule, ExecutionOrigin::Schedule, "App Shutdown", 0, HashMap::new()).await {
                        handles.push(handle);
                    }
                }
            }
        }
        for handle in handles {
            let _ = handle.await;
        }
        self.drain_action_events();
    }

    /// Builds the IPC snapshot the GUI renders. The two state mutations — draining completion
    /// events and rolling lapsed quota periods over — stay here; the aggregation itself lives
    /// in [`StateProjector`].
    pub fn get_current_state(&mut self) -> DaemonState {
        self.drain_action_events();

        // Roll lapsed quota periods over before the snapshot is built, so the GUI never
        // shows a spent counter that has already reset with the calendar.
        let now = chrono::Local::now();
        let mut quota_rolled = false;
        for rule in &mut self.config.rules {
            quota_rolled |= rule.normalize_quota(now);
        }
        if quota_rolled {
            self.mark_runtime_dirty();
        }

        StateProjector::new(self).project()
    }
}
