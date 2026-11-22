use std::time::Instant;

use crate::models::Rule;

use super::state::config_write_guard;
use super::DaemonEngine;

/// What a quota check means for one counted execution.
pub(super) enum QuotaDecision {
    /// The run fits in the current window, or the rule has no limit.
    Allowed,
    /// A periodic quota is spent: the rule stays enabled and resumes with the next window.
    PeriodSpent { message: String },
    /// A lifetime quota is spent: the rule must be auto-paused.
    LimitReached { message: String },
}

impl QuotaDecision {
    /// Classifies `counted` executions against the rule's limit. `counted` includes the
    /// execution being decided: the pre-run gate passes the stored period count and the
    /// post-run path passes the already incremented counter, so the pause/rollover
    /// semantics and their message live here once.
    pub(super) fn classify(rule: &Rule, counted: u64, now: chrono::DateTime<chrono::Local>) -> Self {
        let Some(limit) = rule.max_executions else {
            return Self::Allowed;
        };
        if counted < limit {
            return Self::Allowed;
        }
        if rule.quota_period.is_some() {
            Self::PeriodSpent {
                message: format!(
                    "Rule '{}' reached its quota ({}/{}); the counter resets {}.",
                    rule.name,
                    counted,
                    limit,
                    QuotaLedger::reset_text(rule.quota_next_reset(now))
                ),
            }
        } else {
            Self::LimitReached {
                message: format!(
                    "Rule '{}' completed quota ({}/{}) and has been auto-paused.",
                    rule.name, counted, limit
                ),
            }
        }
    }

    pub(super) fn message(&self) -> Option<&str> {
        match self {
            Self::Allowed => None,
            Self::PeriodSpent { message } | Self::LimitReached { message } => Some(message),
        }
    }
}

/// Daemon-owned quota counters: the coalesced `runtime.json` flush clock and the single
/// place that turns a spent quota into its message and pause decision.
pub(super) struct QuotaLedger {
    /// Set when rule counters changed; `runtime.json` is flushed coalesced so a fast-firing
    /// rule cannot rewrite the file on every execution.
    pub(super) dirty: bool,
    pub(super) last_save: Instant,
}

impl QuotaLedger {
    pub(super) fn new() -> Self {
        Self {
            dirty: false,
            last_save: Instant::now(),
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// True when counters changed and the coalescing window has elapsed.
    fn flush_due(&self, interval: std::time::Duration) -> bool {
        self.dirty && self.last_save.elapsed() >= interval
    }

    fn mark_flushed(&mut self) {
        self.dirty = false;
        self.last_save = Instant::now();
    }

    /// Human-readable "when" part of a periodic-quota reset message.
    fn reset_text(reset: Option<chrono::DateTime<chrono::Local>>) -> String {
        match reset {
            Some(at) => format!("at {}", at.format("%Y-%m-%d %H:%M")),
            None => "when the period ends".to_string(),
        }
    }
}

/// One counted execution, as reported by [`DaemonEngine::record_quota_increment`].
pub(super) struct QuotaIncrement {
    pub(super) decision: QuotaDecision,
    /// A lifetime limit is spent; the caller persists the pause (and may stop the rule).
    pub(super) paused: bool,
}

/// Merges the daemon-owned runtime counters into a freshly saved rule definition.
/// Redefining the quota (changing its period, or enabling one) starts a fresh counting
/// window, because the old counter measured a different thing; every other edit keeps
/// the counters so a rule continues where it left off. Returns `true` when the counters
/// were reset and `runtime.json` therefore has to be flushed. `last_executed_at` is
/// daemon state too and is always preserved, since schedule triggers use it for
/// deduplication.
pub(super) fn adopt_rule_runtime(rule: &mut Rule, existing: &Rule) -> bool {
    let quota_redefined = rule.quota_redefined_from(existing);
    if quota_redefined {
        rule.reset_quota_progress();
    } else {
        rule.execution_count = existing.execution_count;
        rule.quota_window_start = existing.quota_window_start.clone();
    }
    rule.last_executed_at = existing.last_executed_at.clone();
    quota_redefined
}

impl DaemonEngine {
    /// Persists the hot runtime counters at most once per window instead of on every
    /// execution. Quota pauses and shutdown force an immediate flush.
    pub fn persist_runtime_if_due(&mut self) {
        if !self.quota.flush_due(Self::RUNTIME_SAVE_INTERVAL) {
            return;
        }
        self.persist_runtime_now();
    }

    /// Forces `runtime.json` to disk now.
    pub fn persist_runtime_now(&mut self) {
        if !self.quota.is_dirty() {
            return;
        }
        let _write = config_write_guard();
        match self.config.runtime_state().save() {
            Ok(()) => {
                self.quota.mark_flushed();
            }
            Err(err) => {
                self.log(
                    "ERROR",
                    "CONFIG",
                    &format!("Failed to save runtime counters: {err}"),
                );
            }
        }
    }

    pub(super) fn mark_runtime_dirty(&mut self) {
        self.quota.mark_dirty();
    }
}

impl DaemonEngine {
    /// A rule edit that redefined its quota resets the daemon-owned counter, so the
    /// stale `runtime.json` entry must be replaced immediately: the regular flush is
    /// coalesced, and a restart before it would resurrect the old counter.
    pub(super) fn flush_quota_reset(&mut self, reset: bool) {
        if !reset {
            return;
        }
        self.mark_runtime_dirty();
        self.persist_runtime_now();
    }
}

impl DaemonEngine {
    /// Counts one started execution against the quota of `rule_id`: rolls a lapsed window
    /// over, anchors a custom window, advances the counter and stamps `last_executed_at`.
    /// Returns the shared quota decision; a spent lifetime limit is applied to `is_enabled`
    /// here, while persisting and stopping the rule stay with the caller.
    pub(super) fn record_quota_increment(
        &mut self,
        rule_id: &str,
        now: chrono::DateTime<chrono::Local>,
    ) -> QuotaIncrement {
        let mut paused = false;
        let mut decision = QuotaDecision::Allowed;
        if let Some(rule) = self.config.rules.iter_mut().find(|r| r.id == rule_id) {
            let _ = rule.normalize_quota(now);
            rule.begin_quota_window(now);
            rule.execution_count += 1;
            rule.last_executed_at = Some(now.format("%Y-%m-%d %H:%M:%S").to_string());
            decision = QuotaDecision::classify(rule, rule.execution_count, now);
            if matches!(decision, QuotaDecision::LimitReached { .. }) {
                rule.is_enabled = false;
                paused = true;
            }
        }
        self.mark_runtime_dirty();
        QuotaIncrement { decision, paused }
    }

    /// Rolls a lapsed quota period over and reports whether the rule may execute now.
    /// An exhausted lifetime quota pauses the rule (historical behavior); a periodic
    /// quota only waits for the next period, so the rule stays enabled. `announce`
    /// logs the exhausted state for user-invoked runs, which should not fail silently.
    pub(super) fn quota_allows_execution(&mut self, rule_id: &str, announce: bool) -> bool {
        let now = chrono::Local::now();
        let rolled;
        let decision;
        {
            let Some(rule) = self.config.rules.iter_mut().find(|r| r.id == rule_id) else {
                return true;
            };
            rolled = rule.normalize_quota(now);
            decision = QuotaDecision::classify(rule, rule.quota_used(now), now);
            if matches!(decision, QuotaDecision::LimitReached { .. }) {
                rule.is_enabled = false;
            }
        }
        if rolled {
            self.mark_runtime_dirty();
        }
        if matches!(decision, QuotaDecision::LimitReached { .. }) {
            self.persist_rules();
            self.refresh_live_rules();
        }
        match &decision {
            QuotaDecision::Allowed => {}
            QuotaDecision::PeriodSpent { message } => {
                if announce {
                    self.log("INFO", "RULE", message);
                }
            }
            QuotaDecision::LimitReached { message } => self.log("WARN", "RULE", message),
        }
        matches!(decision, QuotaDecision::Allowed)
    }
}
