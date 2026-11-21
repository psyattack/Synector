//! Process-wide snapshot of rule pipeline activity.
//!
//! Conditions that observe another rule (`RuleCondition::RuleRunning` and the
//! `WaitCondition::RuleStarted` / `RuleStopped` wait modes) are evaluated inside action tasks and
//! inside the engine, where the daemon's `Arc<Mutex<DaemonEngine>>` is not available. The engine
//! therefore publishes a pipeline's start and finish here, and readers only take this short lock:
//! a condition never has to wait for the engine, and a long pipeline never keeps the daemon busy.
//!
//! `active` counts the pipelines of a rule that are running right now. `starts` is a monotonic
//! per-rule counter that lets a wait observe a short pipeline that started and finished between two
//! polls.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// Upper bound on remembered rule ids. Real ids come from rule definitions; the cap only protects
/// against target strings interpolated at runtime.
const MAX_TRACKED_RULES: usize = 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuleActivity {
    /// Pipelines of this rule that are running right now.
    pub active: usize,
    /// Number of pipelines of this rule that have started in this daemon process.
    pub starts: u64,
}

static ACTIVITY: LazyLock<RwLock<HashMap<String, RuleActivity>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Records that a pipeline of `rule_id` has started.
pub fn mark_started(rule_id: &str) {
    let Ok(mut activity) = ACTIVITY.write() else {
        return;
    };
    if activity.len() >= MAX_TRACKED_RULES && !activity.contains_key(rule_id) {
        // Forgetting idle rules is safe: a rule that is not running cannot be observed as running
        // any more. The only loss is the `starts` baseline of a wait that already ended.
        activity.retain(|_, entry| entry.active > 0);
    }
    let entry = activity.entry(rule_id.to_string()).or_default();
    entry.active += 1;
    entry.starts = entry.starts.saturating_add(1);
}

/// Records that a pipeline of `rule_id` has ended, however it ended (finish, abort or stop).
pub fn mark_finished(rule_id: &str) {
    let Ok(mut activity) = ACTIVITY.write() else {
        return;
    };
    if let Some(entry) = activity.get_mut(rule_id) {
        entry.active = entry.active.saturating_sub(1);
    }
}

/// Current activity of `rule_id`; a rule that has never run reports the default snapshot.
pub fn snapshot(rule_id: &str) -> RuleActivity {
    ACTIVITY
        .read()
        .ok()
        .and_then(|activity| activity.get(rule_id).copied())
        .unwrap_or_default()
}

/// Whether `rule_id` has a running pipeline right now.
pub fn is_running(rule_id: &str) -> bool {
    snapshot(rule_id).active > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_lifecycle() {
        let id = "rule_activity_test_lifecycle";
        assert!(!is_running(id));
        assert_eq!(snapshot(id), RuleActivity::default());

        mark_started(id);
        assert!(is_running(id));
        assert_eq!(
            snapshot(id),
            RuleActivity {
                active: 1,
                starts: 1,
            }
        );

        // Parallel executions of the same rule are counted together.
        mark_started(id);
        assert_eq!(snapshot(id), RuleActivity { active: 2, starts: 2 });

        mark_finished(id);
        assert!(is_running(id));
        assert_eq!(snapshot(id), RuleActivity { active: 1, starts: 2 });

        mark_finished(id);
        assert!(!is_running(id));
        assert_eq!(snapshot(id), RuleActivity { active: 0, starts: 2 });

        // A stray finish must not underflow the counter.
        mark_finished(id);
        assert!(!is_running(id));
        assert_eq!(snapshot(id).starts, 2);
    }

    #[test]
    fn test_unknown_rule_is_idle() {
        let snapshot = snapshot("rule_activity_test_unknown");
        assert_eq!(snapshot.active, 0);
        assert_eq!(snapshot.starts, 0);
        assert!(!is_running("rule_activity_test_unknown"));
    }
}
