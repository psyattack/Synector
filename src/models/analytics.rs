//! Analytics snapshot exchanged between the daemon and the GUI.
//!
//! The daemon owns the collector: it sees every pipeline start and its outcome even while the
//! GUI is closed. The GUI only renders this snapshot and never counts runs or samples system
//! metrics itself.

use serde::{Deserialize, Serialize};

/// The source that started a run. A manual run and a rule-to-rule call are their own sources
/// regardless of the rule's configured trigger, so the distribution answers "what drives
/// activity" instead of "what is configured".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunSource {
    Process,
    Window,
    Resource,
    Hotkey,
    Schedule,
    System,
    File,
    Webhook,
    Manual,
    RuleCall,
}

impl RunSource {
    /// Every source in one stable order. The index is the series-colour index (`Tokens::series`),
    /// so a source keeps its colour in the donut, the legend and the top-rules dots; the dashboard
    /// itself lists only the sources that started a run.
    pub const ALL: [Self; 10] = [
        Self::Process,
        Self::Window,
        Self::Resource,
        Self::Hotkey,
        Self::Schedule,
        Self::System,
        Self::File,
        Self::Webhook,
        Self::Manual,
        Self::RuleCall,
    ];
}

/// Runs one source started in the tracked window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRuns {
    pub source: RunSource,
    pub executions: u32,
}

/// Runs one rule started in the tracked window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleRuns {
    pub rule_id: String,
    pub executions: u32,
}

/// How the finished runs of the tracked window ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOutcomes {
    /// Finished with every step succeeded.
    pub succeeded: u32,
    /// Finished, but at least one step failed; non-stopping errors count as a failure.
    pub failed: u32,
    /// Stopped before finishing: a user stop, a rule stop or a replaced run.
    pub aborted: u32,
}

impl RunOutcomes {
    pub fn total(self) -> u32 {
        self.succeeded
            .saturating_add(self.failed)
            .saturating_add(self.aborted)
    }

    /// Error-free share of the finished runs, `None` before the first outcome is recorded.
    pub fn success_rate(self) -> Option<f32> {
        let total = self.total();
        (total > 0).then(|| self.succeeded as f32 / total as f32)
    }
}

/// One hour of the activity chart.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyticsHour {
    /// Start of the bucket as a Unix timestamp. The daemon buckets by local wall-clock hour, so
    /// the GUI formats it as the local `HH:00` of that bucket.
    pub hour_start_unix: i64,
    pub executions: u32,
    pub failures: u32,
    pub aborted: u32,
}

impl AnalyticsHour {
    pub fn failures_total(self) -> u32 {
        self.failures.saturating_add(self.aborted)
    }
}

/// Resource use of the daemon process itself, sampled by the daemon once per second.
///
/// The dashboard shows these next to the system-wide load, so a busy daemon cannot hide in the
/// machine average. Network and GPU are deliberately absent: Windows exposes no cheap
/// per-process counter for either, and a daemon's share of them is effectively zero anyway.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct DaemonUsage {
    /// Share of the whole machine's CPU, matching the Task Manager figure. `None` until two
    /// samples exist, because a CPU percentage needs an interval.
    pub cpu_pct: Option<f32>,
    /// Private working set, the memory figure Task Manager reports.
    pub ram_bytes: u64,
    /// File and device bytes the daemon read and wrote per second since the previous sample.
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
}

impl DaemonUsage {
    /// Combined read and write throughput in bytes per second.
    pub fn disk_bps(self) -> u64 {
        self.disk_read_bps.saturating_add(self.disk_write_bps)
    }
}

/// One analytics read for the dashboard. Produced on demand by the daemon, so the ordinary
/// state poll stays lean; polled only while the dialog is open.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsSnapshot {
    /// Pipelines started since the daemon started, all sources and test runs included.
    pub session_executions: u64,
    /// Runs started in the tracked window (at most the last 24 hours).
    pub window_executions: u32,
    pub outcomes: RunOutcomes,
    /// Sources that started at least one run in the window.
    pub sources: Vec<SourceRuns>,
    /// The last 24 hours of activity, oldest first: the current local hour plus the 24 hours
    /// before it.
    pub hours: Vec<AnalyticsHour>,
    /// Top rules by runs in the window, highest first.
    pub top_rules: Vec<RuleRuns>,
    /// System CPU samples in percent, oldest first.
    pub cpu: Vec<f32>,
    /// System RAM usage samples in percent, oldest first.
    pub ram: Vec<f32>,
    /// The daemon's own CPU, memory and disk use.
    pub daemon: DaemonUsage,
}
