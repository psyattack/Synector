use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

use super::rule::Rule;

/// Unit used by a custom quota window.
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuotaUnit {
    #[default]
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
}

impl QuotaUnit {
    pub const ALL: [QuotaUnit; 5] = [
        QuotaUnit::Minutes,
        QuotaUnit::Hours,
        QuotaUnit::Days,
        QuotaUnit::Weeks,
        QuotaUnit::Months,
    ];

    /// Fixed length of one unit in seconds. Calendar months have no fixed length and are
    /// added as calendar months by [`QuotaPeriod::custom_window_end`].
    pub fn fixed_seconds(self) -> Option<u64> {
        match self {
            QuotaUnit::Minutes => Some(60),
            QuotaUnit::Hours => Some(3_600),
            QuotaUnit::Days => Some(86_400),
            QuotaUnit::Weeks => Some(604_800),
            QuotaUnit::Months => None,
        }
    }
}

/// Reset period of a rule's execution quota.
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum QuotaPeriod {
    /// Reset at the start of every clock minute.
    Minute,
    /// Reset at the start of every clock hour.
    Hour,
    /// Reset at local midnight.
    Day,
    /// Reset at Monday 00:00 of the ISO week.
    Week,
    /// Reset on the 1st day of the month at 00:00.
    Month,
    /// Fixed-length window anchored at the first execution after the previous window lapsed.
    Custom {
        /// Window length in `unit`s.
        value: u32,
        #[serde(default)]
        unit: QuotaUnit,
    },
}

impl QuotaPeriod {
    /// Largest allowed length of a custom window, in `unit`s. Matches the GUI clamp so the
    /// CLI, share import and the editor accept exactly the same range.
    pub const MAX_CUSTOM_WINDOW_UNITS: u32 = 100_000;

    /// Rejects custom window lengths that are zero or absurdly large, which would overflow
    /// the calendar arithmetic and silently break the reset.
    pub fn validation_error(self) -> Option<&'static str> {
        match self {
            QuotaPeriod::Custom { value, .. }
                if !(1..=Self::MAX_CUSTOM_WINDOW_UNITS).contains(&value) =>
            {
                Some("a custom quota window length must be between 1 and 100000")
            }
            _ => None,
        }
    }

    /// End of a custom window that started at `start`; calendar months are added as
    /// calendar months (Jan 31 + 1 month = Feb 28/29), other units as fixed time.
    ///
    /// Lengths are clamped into the accepted range first, and the arithmetic is checked:
    /// `None` means the end cannot be represented and the window must be treated as lapsed.
    pub fn custom_window_end(self, start: DateTime<Local>) -> Option<DateTime<Local>> {
        match self {
            QuotaPeriod::Custom { value, unit } => {
                let value = value.clamp(1, Self::MAX_CUSTOM_WINDOW_UNITS);
                match unit.fixed_seconds() {
                    Some(seconds) => {
                        let total = i64::try_from(seconds).ok()?.checked_mul(i64::from(value))?;
                        start.checked_add_signed(chrono::Duration::seconds(total))
                    }
                    None => start.checked_add_months(chrono::Months::new(value)),
                }
            }
            _ => None,
        }
    }
}

impl Rule {
    /// Clears the daemon-owned quota counters and the custom-window anchor.
    pub fn clear_quota_runtime(&mut self) {
        self.execution_count = 0;
        self.last_executed_at = None;
        self.quota_window_start = None;
    }

    /// True when `self` redefines the quota relative to `previous`: the reset period changed,
    /// or a limit was enabled where there was none. Changing only the numeric limit keeps
    /// the running counter, which is the documented update semantics.
    pub fn quota_redefined_from(&self, previous: &Rule) -> bool {
        self.quota_period != previous.quota_period
            || (previous.max_executions.is_none() && self.max_executions.is_some())
    }

    /// Clears the quota counter and the custom-window anchor but keeps `last_executed_at`,
    /// which schedule triggers use for deduplication.
    pub fn reset_quota_progress(&mut self) {
        self.execution_count = 0;
        self.quota_window_start = None;
    }

    fn parsed_timestamp(stamp: Option<&str>) -> Option<DateTime<Local>> {
        let naive = chrono::NaiveDateTime::parse_from_str(stamp?, "%Y-%m-%d %H:%M:%S").ok()?;
        Local.from_local_datetime(&naive).single()
    }

    fn last_executed_local(&self) -> Option<DateTime<Local>> {
        Self::parsed_timestamp(self.last_executed_at.as_deref())
    }

    fn quota_window_start_local(&self) -> Option<DateTime<Local>> {
        Self::parsed_timestamp(self.quota_window_start.as_deref())
    }

    /// Identifier of the calendar bucket of `period` that contains `at`.
    fn calendar_bucket(period: QuotaPeriod, at: DateTime<Local>) -> Option<String> {
        match period {
            QuotaPeriod::Minute => Some(at.format("%Y-%m-%d %H:%M").to_string()),
            QuotaPeriod::Hour => Some(at.format("%Y-%m-%d %H").to_string()),
            QuotaPeriod::Day => Some(at.format("%Y-%m-%d").to_string()),
            QuotaPeriod::Week => Some(at.format("%G-W%V").to_string()),
            QuotaPeriod::Month => Some(at.format("%Y-%m").to_string()),
            QuotaPeriod::Custom { .. } => None,
        }
    }

    /// True when the stored counter still belongs to the quota window containing `now`.
    fn quota_window_is_current(&self, now: DateTime<Local>) -> bool {
        match self.quota_period {
            None => true,
            Some(period @ QuotaPeriod::Custom { .. }) => {
                let Some(start) = self.quota_window_start_local() else {
                    return false;
                };
                let Some(end) = period.custom_window_end(start) else {
                    // An unrepresentable window end (calendar overflow or a corrupt value)
                    // is treated as lapsed so the quota cannot get stuck forever.
                    return false;
                };
                now < end
            }
            Some(period) => match self.last_executed_local() {
                Some(last) => Self::calendar_bucket(period, last) == Self::calendar_bucket(period, now),
                None => false,
            },
        }
    }

    /// Executions counted against the quota in the period containing `now`. A lapsed
    /// window reads as zero even before the stored counter is reset.
    pub fn quota_used(&self, now: DateTime<Local>) -> u64 {
        if self.quota_window_is_current(now) {
            self.execution_count
        } else {
            0
        }
    }

    /// Remaining executions in the current period; `None` when the rule is unlimited.
    pub fn quota_remaining(&self, now: DateTime<Local>) -> Option<u64> {
        let limit = self.max_executions?;
        Some(limit.saturating_sub(self.quota_used(now)))
    }

    pub fn quota_exhausted(&self, now: DateTime<Local>) -> bool {
        self.max_executions
            .is_some_and(|limit| self.quota_used(now) >= limit)
    }

    /// Drops the counter of a lapsed period. Returns `true` when the counters changed,
    /// so the daemon can persist the rollover.
    pub fn normalize_quota(&mut self, now: DateTime<Local>) -> bool {
        if self.quota_period.is_none() || self.quota_window_is_current(now) {
            return false;
        }
        let changed = self.execution_count != 0 || self.quota_window_start.is_some();
        self.execution_count = 0;
        self.quota_window_start = None;
        changed
    }

    /// Anchors a custom window at the moment its first execution starts.
    pub fn begin_quota_window(&mut self, now: DateTime<Local>) -> bool {
        if !matches!(self.quota_period, Some(QuotaPeriod::Custom { .. })) || self.quota_window_start.is_some() {
            return false;
        }
        self.quota_window_start = Some(now.format("%Y-%m-%d %H:%M:%S").to_string());
        true
    }

    /// The next moment the quota counter resets, when that moment is known.
    pub fn quota_next_reset(&self, now: DateTime<Local>) -> Option<DateTime<Local>> {
        match self.quota_period? {
            period @ QuotaPeriod::Custom { .. } => {
                let start = self.quota_window_start_local()?;
                let end = period.custom_window_end(start)?;
                (end > now).then_some(end)
            }
            period => {
                let midnight = now.date_naive().and_hms_opt(0, 0, 0)?;
                let start_of_day = Local.from_local_datetime(&midnight).single()?;
                let next = match period {
                    QuotaPeriod::Minute => {
                        let start_of_minute = Local
                            .from_local_datetime(&now.date_naive().and_hms_opt(now.hour(), now.minute(), 0)?)
                            .single()?;
                        start_of_minute + chrono::Duration::minutes(1)
                    }
                    QuotaPeriod::Hour => start_of_day + chrono::Duration::hours(now.hour() as i64 + 1),
                    QuotaPeriod::Day => start_of_day + chrono::Duration::days(1),
                    QuotaPeriod::Week => {
                        let monday =
                            start_of_day - chrono::Duration::days(now.weekday().num_days_from_monday() as i64);
                        monday + chrono::Duration::weeks(1)
                    }
                    QuotaPeriod::Month => {
                        let (year, month) = if now.month() == 12 {
                            (now.year() + 1, 1)
                        } else {
                            (now.year(), now.month() + 1)
                        };
                        let naive = chrono::NaiveDate::from_ymd_opt(year, month, 1)?.and_hms_opt(0, 0, 0)?;
                        Local.from_local_datetime(&naive).single()?
                    }
                    QuotaPeriod::Custom { .. } => return None,
                };
                Some(next)
            }
        }
    }
}
