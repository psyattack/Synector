//! Daemon-side analytics collector.
//!
//! The daemon is the only tier that sees every pipeline start and finish, including the runs that
//! happen while the GUI is closed. This collector keeps a rolling 24-hour window of hourly
//! buckets (runs, outcomes, per-source and per-rule counts) plus the last two minutes of system
//! CPU/RAM samples for the live chart. The GUI reads one [`AnalyticsSnapshot`] on demand and
//! never counts or samples anything itself.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Timelike, TimeZone};

use crate::models::{
    AnalyticsHour, AnalyticsSnapshot, DaemonUsage, RuleRuns, RunOutcomes, RunSource, SourceRuns,
    Trigger,
};

/// Hours of lookback the rolling window covers.
const WINDOW_HOURS: i64 = 24;
/// Hourly slots in a snapshot: the current local hour plus every hour of the lookback.
const WINDOW_SLOTS: usize = WINDOW_HOURS as usize + 1;
/// Live CPU/RAM samples kept for the chart (about two minutes at the one-second sample cadence).
const LIVE_SAMPLES: usize = 120;
/// Minimum distance between two live samples. The flush loop ticks every second; the guard makes
/// a delayed or doubled tick unable to compress the timeline.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(900);
/// Rules listed by the dashboard ranking.
const TOP_RULES: usize = 8;

/// The source a run is attributed to: a manual run or a rule-to-rule call overrides the rule's
/// configured trigger, everything else follows the trigger kind.
pub fn source_for_run(trigger: Option<&Trigger>, origin: Option<crate::daemon::engine::ExecutionOrigin>) -> RunSource {
    use crate::daemon::engine::ExecutionOrigin;
    match origin {
        Some(ExecutionOrigin::Manual | ExecutionOrigin::Test) => return RunSource::Manual,
        Some(ExecutionOrigin::RuleInvocation) => return RunSource::RuleCall,
        _ => {}
    }
    source_for_trigger(trigger)
}

/// The configured trigger kind, used for recurring cycles whose start is reported without an
/// origin.
pub fn source_for_trigger(trigger: Option<&Trigger>) -> RunSource {
    match trigger {
        Some(Trigger::Process { .. }) => RunSource::Process,
        Some(Trigger::WindowFocus { .. }) => RunSource::Window,
        Some(Trigger::ResourceThreshold { .. }) => RunSource::Resource,
        Some(Trigger::GlobalHotkey { .. }) => RunSource::Hotkey,
        Some(Trigger::Schedule { .. }) => RunSource::Schedule,
        Some(Trigger::SystemEvent { .. }) => RunSource::System,
        Some(Trigger::FileWatcher { .. }) => RunSource::File,
        Some(Trigger::Webhook { .. }) => RunSource::Webhook,
        Some(Trigger::Manual) | None => RunSource::Manual,
    }
}

/// One hour of raw counts. The per-source and per-rule maps live in the bucket so an aggregate
/// leaves the window exactly when its hour does.
#[derive(Default)]
struct HourBucket {
    hour_start_unix: i64,
    executions: u32,
    succeeded: u32,
    failures: u32,
    aborted: u32,
    sources: HashMap<RunSource, u32>,
    rules: HashMap<String, u32>,
}

/// Rolling analytics state owned by the daemon engine.
#[derive(Default)]
pub struct AnalyticsCollector {
    session_executions: u64,
    hours: VecDeque<HourBucket>,
    cpu: VecDeque<f32>,
    ram: VecDeque<f32>,
    last_sample: Option<Instant>,
    /// The daemon's own CPU/memory/disk use, refreshed by the one-second flush loop.
    daemon: DaemonUsage,
}

impl AnalyticsCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one pipeline start.
    pub fn record_run(&mut self, source: RunSource, rule_id: &str, now: DateTime<Local>) {
        self.session_executions = self.session_executions.saturating_add(1);
        let bucket = self.hour_bucket(now);
        bucket.executions = bucket.executions.saturating_add(1);
        let source_count = bucket.sources.entry(source).or_insert(0);
        *source_count = source_count.saturating_add(1);
        let rule_count = bucket.rules.entry(rule_id.to_string()).or_insert(0);
        *rule_count = rule_count.saturating_add(1);
    }

    /// Records a pipeline that finished with every step succeeded.
    pub fn record_success(&mut self, now: DateTime<Local>) {
        let bucket = self.hour_bucket(now);
        bucket.succeeded = bucket.succeeded.saturating_add(1);
    }

    /// Records a pipeline that finished with at least one failed step.
    pub fn record_failure(&mut self, now: DateTime<Local>) {
        let bucket = self.hour_bucket(now);
        bucket.failures = bucket.failures.saturating_add(1);
    }

    /// Records pipelines stopped before finishing.
    pub fn record_aborts(&mut self, count: u32, now: DateTime<Local>) {
        if count == 0 {
            return;
        }
        let bucket = self.hour_bucket(now);
        bucket.aborted = bucket.aborted.saturating_add(count);
    }

    /// Stores the latest sample of the daemon's own resource use. The collector only keeps the
    /// newest one: the dashboard shows it as a single line, not a history.
    pub fn record_daemon_usage(&mut self, usage: DaemonUsage) {
        self.daemon = usage;
    }

    /// Pushes one CPU/RAM sample, throttled to the sample interval and bounded to the live window.
    pub fn sample_system(&mut self, cpu_pct: f32, ram_pct: f32, now: Instant) {
        if self
            .last_sample
            .is_some_and(|last| now.duration_since(last) < SAMPLE_INTERVAL)
        {
            return;
        }
        self.last_sample = Some(now);
        push_sample(&mut self.cpu, cpu_pct);
        push_sample(&mut self.ram, ram_pct);
    }

    /// Builds the snapshot for one dashboard read. Hour slots without a recorded bucket are
    /// returned as zeros so the chart always has a full timeline.
    pub fn snapshot(&self, now: DateTime<Local>) -> AnalyticsSnapshot {
        let mut hours = Vec::with_capacity(WINDOW_SLOTS);
        let mut outcomes = RunOutcomes::default();
        let mut window_executions = 0u32;
        let mut sources: HashMap<RunSource, u32> = HashMap::new();
        let mut rules: HashMap<String, u32> = HashMap::new();

        for start in window_starts(now) {
            let bucket = self.hours.iter().find(|bucket| bucket.hour_start_unix == start);
            let (executions, failures, aborted) = bucket
                .map(|bucket| (bucket.executions, bucket.failures, bucket.aborted))
                .unwrap_or_default();
            hours.push(AnalyticsHour {
                hour_start_unix: start,
                executions,
                failures,
                aborted,
            });
            if let Some(bucket) = bucket {
                window_executions = window_executions.saturating_add(bucket.executions);
                outcomes.succeeded = outcomes.succeeded.saturating_add(bucket.succeeded);
                outcomes.failed = outcomes.failed.saturating_add(bucket.failures);
                outcomes.aborted = outcomes.aborted.saturating_add(bucket.aborted);
                for (source, count) in &bucket.sources {
                    let total = sources.entry(*source).or_insert(0);
                    *total = total.saturating_add(*count);
                }
                for (rule_id, count) in &bucket.rules {
                    let total = rules.entry(rule_id.clone()).or_insert(0);
                    *total = total.saturating_add(*count);
                }
            }
        }

        let mut sources: Vec<SourceRuns> = sources
            .into_iter()
            .map(|(source, executions)| SourceRuns { source, executions })
            .collect();
        sources.sort_by(|a, b| {
            b.executions
                .cmp(&a.executions)
                .then_with(|| a.source.cmp(&b.source))
        });

        let mut top_rules: Vec<RuleRuns> = rules
            .into_iter()
            .map(|(rule_id, executions)| RuleRuns { rule_id, executions })
            .collect();
        top_rules.sort_by(|a, b| {
            b.executions
                .cmp(&a.executions)
                .then_with(|| a.rule_id.cmp(&b.rule_id))
        });
        top_rules.truncate(TOP_RULES);

        AnalyticsSnapshot {
            session_executions: self.session_executions,
            window_executions,
            outcomes,
            sources,
            hours,
            top_rules,
            cpu: self.cpu.iter().copied().collect(),
            ram: self.ram.iter().copied().collect(),
            daemon: self.daemon,
        }
    }

    /// The current hour bucket, rolled forward when the clock crosses an hour. At most
    /// `WINDOW_SLOTS` buckets are kept.
    fn hour_bucket(&mut self, now: DateTime<Local>) -> &mut HourBucket {
        let start = hour_start_unix(now);
        // A clock that steps backwards (an NTP correction or a manual change) can ask for an hour
        // that already has a bucket. Reusing that bucket keeps its counts in one place; pushing a
        // second one for the same hour would leave it unread, because `snapshot` finds buckets by
        // their start.
        if let Some(index) = self
            .hours
            .iter()
            .position(|bucket| bucket.hour_start_unix == start)
        {
            return &mut self.hours[index];
        }
        self.hours.push_back(HourBucket {
            hour_start_unix: start,
            ..Default::default()
        });
        while self.hours.len() > WINDOW_SLOTS {
            self.hours.pop_front();
        }
        self.hours
            .back_mut()
            .expect("a bucket was just ensured for the current hour")
    }
}

/// Bucket starts of the rolling window, oldest first. The walk goes back one *wall-clock* hour at
/// a time instead of subtracting 3600-second steps from the current start, so a local-time offset
/// that is not a whole hour still labels a bucket with its real local hour, and a clock change
/// cannot produce two slots with the same label.
fn window_starts(now: DateTime<Local>) -> Vec<i64> {
    let mut starts = Vec::with_capacity(WINDOW_SLOTS);
    let mut instant = now.timestamp();
    while starts.len() < WINDOW_SLOTS {
        let local = Local
            .timestamp_opt(instant, 0)
            .single()
            .unwrap_or(now);
        let start = hour_start_unix(local);
        starts.push(start);
        // One second before the bucket start is inside the previous existing wall-clock hour, so
        // the next iteration cannot land on the same slot.
        instant = start - 1;
    }
    starts.reverse();
    starts
}

/// Local-hour start of `now` as a Unix timestamp: the wall-clock hour, not the UTC hour.
fn hour_start_unix(now: DateTime<Local>) -> i64 {
    let truncated = now
        .naive_local()
        .with_minute(0)
        .and_then(|time| time.with_second(0))
        .and_then(|time| time.with_nanosecond(0));
    let Some(truncated) = truncated else {
        return now.timestamp();
    };
    match Local.from_local_datetime(&truncated) {
        chrono::LocalResult::Single(value) => value.timestamp(),
        // A repeated wall hour (a DST fall-back) keeps one bucket: the earliest occurrence.
        chrono::LocalResult::Ambiguous(earliest, _) => earliest.timestamp(),
        chrono::LocalResult::None => now.timestamp(),
    }
}

fn push_sample(series: &mut VecDeque<f32>, value: f32) {
    let value = if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        series.back().copied().unwrap_or(0.0)
    };
    series.push_back(value);
    while series.len() > LIVE_SAMPLES {
        series.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 11, 24, hour, minute, 0)
            .single()
            .expect("a valid local timestamp")
    }

    #[test]
    fn test_sources_follow_the_trigger_and_override_manual_runs() {
        use crate::daemon::engine::ExecutionOrigin;
        use crate::models::ScheduleType;

        let schedule = Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec: 60,
            daily_time: None,
            days_of_week: vec![],
            day_of_month: None,
            target_datetime: None,
        };
        assert_eq!(source_for_run(Some(&schedule), Some(ExecutionOrigin::Schedule)), RunSource::Schedule);
        assert_eq!(source_for_run(Some(&schedule), Some(ExecutionOrigin::Test)), RunSource::Manual);
        assert_eq!(source_for_run(None, Some(ExecutionOrigin::Manual)), RunSource::Manual);
        assert_eq!(
            source_for_run(Some(&schedule), Some(ExecutionOrigin::RuleInvocation)),
            RunSource::RuleCall
        );
        assert_eq!(source_for_trigger(None), RunSource::Manual);
    }

    #[test]
    fn test_runs_and_outcomes_accumulate_in_the_current_hour() {
        let mut collector = AnalyticsCollector::new();
        let now = at(10, 15);
        collector.record_run(RunSource::Process, "rule-a", now);
        collector.record_run(RunSource::Process, "rule-a", now);
        collector.record_run(RunSource::Schedule, "rule-b", now);
        collector.record_success(now);
        collector.record_failure(now);
        collector.record_aborts(2, now);

        let snapshot = collector.snapshot(now);
        assert_eq!(snapshot.session_executions, 3);
        assert_eq!(snapshot.window_executions, 3);
        assert_eq!(snapshot.outcomes, RunOutcomes { succeeded: 1, failed: 1, aborted: 2 });
        assert_eq!(snapshot.outcomes.success_rate(), Some(0.25));

        assert_eq!(snapshot.hours.len(), WINDOW_SLOTS);
        let current = snapshot.hours.last().copied().expect("the current hour is present");
        assert_eq!(current.executions, 3);
        assert_eq!(current.failures, 1);
        assert_eq!(current.aborted, 2);
        assert!(
            snapshot.hours.iter().take(WINDOW_SLOTS - 1).all(|hour| hour.executions == 0),
            "hours before the daemon started stay empty"
        );

        let sources: Vec<_> = snapshot.sources.iter().map(|entry| (entry.source, entry.executions)).collect();
        assert_eq!(sources, vec![(RunSource::Process, 2), (RunSource::Schedule, 1)]);
        assert_eq!(snapshot.top_rules[0], RuleRuns { rule_id: "rule-a".into(), executions: 2 });
        assert_eq!(snapshot.top_rules[1], RuleRuns { rule_id: "rule-b".into(), executions: 1 });
    }

    #[test]
    fn test_hourly_window_rolls_off_after_a_day() {
        let mut collector = AnalyticsCollector::new();
        let first = at(0, 5);
        collector.record_run(RunSource::Hotkey, "rule-a", first);
        let same_hour_next_day = at(0, 5) + chrono::Duration::hours(24);
        collector.record_run(RunSource::Hotkey, "rule-a", same_hour_next_day);
        assert_eq!(collector.hours.len(), 2);

        // A run exactly 24 hours ago is on the window boundary and still counted.
        let boundary = collector.snapshot(same_hour_next_day);
        assert_eq!(boundary.window_executions, 2);
        assert_eq!(boundary.session_executions, 2);

        // One hour later that bucket has left the window and only the newer run remains.
        let an_hour_later = same_hour_next_day + chrono::Duration::hours(1);
        let later = collector.snapshot(an_hour_later);
        assert_eq!(later.window_executions, 1);
        assert_eq!(later.session_executions, 2);
        assert_eq!(later.sources, vec![SourceRuns { source: RunSource::Hotkey, executions: 1 }]);
        assert_eq!(later.top_rules, vec![RuleRuns { rule_id: "rule-a".into(), executions: 1 }]);
        assert_eq!(
            later.hours.last().map(|hour| hour.hour_start_unix),
            Some(hour_start_unix(an_hour_later)),
            "the newest slot is the current hour"
        );
        assert_eq!(
            later
                .hours
                .iter()
                .find(|hour| hour.hour_start_unix == hour_start_unix(same_hour_next_day))
                .map(|hour| hour.executions),
            Some(1),
            "the run stays in the slot of the hour that recorded it"
        );
    }

    #[test]
    fn test_a_clock_that_steps_back_reuses_the_bucket_of_that_hour() {
        let mut collector = AnalyticsCollector::new();
        collector.record_run(RunSource::Process, "rule-a", at(10, 5));
        // A correction moves the clock back into the previous hour: the 09:00 slot must receive
        // the run, not a duplicate bucket that `snapshot` would never read.
        collector.record_run(RunSource::Schedule, "rule-a", at(9, 30));
        collector.record_run(RunSource::Process, "rule-a", at(10, 20));
        assert_eq!(collector.hours.len(), 2, "one bucket per hour");

        let snapshot = collector.snapshot(at(10, 21));
        assert_eq!(snapshot.window_executions, 3);
        let nine = snapshot
            .hours
            .iter()
            .find(|hour| hour.hour_start_unix == hour_start_unix(at(9, 0)))
            .copied()
            .expect("the 09:00 slot is part of the window");
        assert_eq!(nine.executions, 1);
        let ten = snapshot
            .hours
            .iter()
            .find(|hour| hour.hour_start_unix == hour_start_unix(at(10, 0)))
            .copied()
            .expect("the 10:00 slot is part of the window");
        assert_eq!(ten.executions, 2);
        assert_eq!(snapshot.sources.len(), 2);
    }

    #[test]
    fn test_empty_buckets_keep_the_snapshot_contiguous() {
        let mut collector = AnalyticsCollector::new();
        let now = at(12, 0);
        collector.record_run(RunSource::Manual, "rule-a", at(9, 0));
        let snapshot = collector.snapshot(now);
        assert_eq!(snapshot.hours.len(), WINDOW_SLOTS);
        assert!(snapshot.hours.windows(2).all(|pair| pair[1].hour_start_unix - pair[0].hour_start_unix
            == chrono::Duration::hours(1).num_seconds()));
        // The 09:00 bucket is still inside the 24-hour window and shows up exactly once.
        let nine = snapshot
            .hours
            .iter()
            .filter(|hour| hour.hour_start_unix == hour_start_unix(at(9, 0)))
            .count();
        assert_eq!(nine, 1);
        assert_eq!(snapshot.window_executions, 1);
    }

    #[test]
    fn test_live_samples_are_throttled_and_bounded() {
        let mut collector = AnalyticsCollector::new();
        let start = Instant::now();
        collector.sample_system(10.0, 40.0, start);
        collector.sample_system(90.0, 90.0, start + Duration::from_millis(200));
        assert_eq!(collector.cpu.len(), 1, "a doubled tick must not compress the timeline");

        for index in 0..LIVE_SAMPLES + 20 {
            collector.sample_system(index as f32, 50.0, start + Duration::from_secs(index as u64 + 1));
        }
        assert_eq!(collector.cpu.len(), LIVE_SAMPLES);
        assert_eq!(collector.ram.len(), LIVE_SAMPLES);
        assert!(collector.cpu.iter().all(|value| (0.0..=100.0).contains(value)));

        collector.sample_system(f32::NAN, f32::INFINITY, start + Duration::from_secs(1000));
        let last = collector.cpu.back().copied().unwrap();
        assert!(last.is_finite(), "a non-finite sample must not reach the chart");
    }

    #[test]
    fn test_outcome_helpers() {
        assert_eq!(RunOutcomes::default().success_rate(), None);
        assert_eq!(
            RunOutcomes { succeeded: 3, failed: 1, aborted: 0 }.success_rate(),
            Some(0.75)
        );
        assert_eq!(
            RunOutcomes { succeeded: 1, failed: 2, aborted: 1 }.total(),
            4
        );
    }

    #[test]
    fn test_daemon_usage_rides_on_the_snapshot() {
        let mut collector = AnalyticsCollector::new();
        assert_eq!(
            collector.snapshot(at(10, 0)).daemon,
            DaemonUsage::default(),
            "no sample yet"
        );

        let usage = DaemonUsage {
            cpu_pct: Some(1.5),
            ram_bytes: 24 * 1024 * 1024,
            disk_read_bps: 2048,
            disk_write_bps: 512,
        };
        collector.record_daemon_usage(usage);

        assert_eq!(collector.snapshot(at(10, 0)).daemon, usage);
        assert_eq!(usage.disk_bps(), 2560, "the dashboard reads one combined disk rate");
    }
}
