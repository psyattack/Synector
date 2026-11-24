pub mod activity;
mod executions;
mod ipc;
mod offline;
mod pipeline;
mod projector;
mod quota;
pub mod rule_call;
mod scheduler;
mod state;
mod triggers;
mod update;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::daemon::analytics::AnalyticsCollector;
use crate::daemon::watcher::ProcessWatcher;
use crate::models::{AnalyticsSnapshot, LogItem, Rule};

use executions::ExecutionTracker;
use quota::QuotaLedger;
use scheduler::Scheduler;
use update::UpdateStatus;

pub use executions::*;
pub use offline::*;
pub(crate) use pipeline::{run_step_loop, PipelineObserver};

pub struct DaemonEngine {
    pub config: AppConfig,
    start_time: Instant,
    logs: Vec<LogItem>,
    watcher: ProcessWatcher,
    /// Active pipelines, abort handles, recurring test loops, bound processes and bound overlays.
    executions: ExecutionTracker,
    rule_last_triggered: HashMap<String, Instant>,
    /// Latest rule definitions plus their runtime counters, shared with recurring executions so a
    /// running loop re-reads its rule instead of executing a stale captured pipeline.
    live_rules: std::sync::Arc<std::sync::RwLock<Vec<Rule>>>,
    active_rule_pids: HashMap<String, HashSet<u32>>,
    /// Rolling analytics: run sources, outcomes, hourly buckets and live CPU/RAM samples.
    analytics: AnalyticsCollector,
    last_foreground_pid: Option<u32>,
    hotkey_handle: Option<crate::daemon::hotkeys::HotkeyHandle>,
    /// Updater display state shared by the tray download task and the state projection.
    update: UpdateStatus,
    /// Scheduler admission clocks (attempt throttle and resource-threshold dwell).
    scheduler: Scheduler,
    /// Quota counters and their coalesced `runtime.json` flush clock.
    quota: QuotaLedger,
    /// Overlay drags stream position updates; persisting presets on every frame would
    /// hammer `library.json`, so those saves are coalesced separately.
    overlay_position_library_dirty: bool,
    overlay_position_rules_dirty: bool,
    last_overlay_position_save: Instant,
    action_tx: mpsc::UnboundedSender<ActionCompletionEvent>,
    action_rx: mpsc::UnboundedReceiver<ActionCompletionEvent>,
    /// Test-only switch that makes rule writes fail so the IPC error contract can be exercised
    /// without corrupting the shared test config directory.
    #[cfg(test)]
    forced_persist_failure: bool,
}

impl DaemonEngine {
    pub const MAX_LOGS: usize = 500;
    /// Coalescing window for `runtime.json`; counters may lag the daemon by this much
    /// after a hard kill, which is acceptable for execution quotas.
    const RUNTIME_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
    /// Coalescing window for overlay positions streamed while dragging an overlay.
    const OVERLAY_POSITION_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

    pub fn new(config: AppConfig) -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let live_rules = std::sync::Arc::new(std::sync::RwLock::new(config.rules.clone()));
        Self {
            config,
            start_time: Instant::now(),
            logs: Vec::new(),
            watcher: ProcessWatcher::new(),
            executions: ExecutionTracker::new(),
            rule_last_triggered: HashMap::new(),
            live_rules,
            active_rule_pids: HashMap::new(),
            analytics: AnalyticsCollector::new(),
            last_foreground_pid: None,
            hotkey_handle: None,
            update: UpdateStatus::new(),
            scheduler: Scheduler::new(),
            quota: QuotaLedger::new(),
            overlay_position_library_dirty: false,
            overlay_position_rules_dirty: false,
            last_overlay_position_save: Instant::now(),
            action_tx,
            action_rx,
            #[cfg(test)]
            forced_persist_failure: false,
        }
    }

    pub(crate) fn watcher(&self) -> &ProcessWatcher {
        &self.watcher
    }

    /// One analytics read for the dashboard. The ordinary state poll stays lean; the GUI asks
    /// for this only while its analytics dialog is open.
    pub fn analytics_snapshot(&self) -> AnalyticsSnapshot {
        self.analytics.snapshot(chrono::Local::now())
    }

    /// Pushes one system sample into the live chart series. Called from the one-second runtime
    /// flush loop with metrics sampled outside the engine lock.
    pub(crate) fn record_analytics_sample(&mut self, cpu_pct: f32, ram_pct: f32, now: Instant) {
        self.analytics.sample_system(cpu_pct, ram_pct, now);
    }

    /// Stores the daemon's own CPU/memory/disk sample for the dashboard line.
    pub(crate) fn record_daemon_usage(&mut self, usage: crate::models::DaemonUsage) {
        self.analytics.record_daemon_usage(usage);
    }

    pub(crate) fn set_hotkey_handle(&mut self, handle: crate::daemon::hotkeys::HotkeyHandle) {
        self.hotkey_handle = Some(handle);
    }

    pub(crate) fn mark_rule_triggered(&mut self, rule_id: &str) {
        self.rule_last_triggered.insert(rule_id.to_string(), Instant::now());
    }

    #[cfg(test)]
    pub(crate) fn triggered_rule_ids(&self) -> Vec<String> {
        self.rule_last_triggered.keys().cloned().collect()
    }
}
