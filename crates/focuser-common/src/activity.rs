//! Activity measurement: what counts as human time, and where it comes from.
//!
//! Three different quantities are kept apart on purpose, because collapsing
//! them is how "screen time" products end up lying:
//!
//! * **Foreground** — the target was the thing in front of the user. A browser
//!   tab that is active in a focused window, or the application that owns the
//!   foreground window. It does not require input: reading a page for ten
//!   minutes is foreground time.
//! * **Interaction** — a subset of foreground where a human actually produced
//!   input inside the interval (click, key, scroll, pointer). Automation,
//!   background tabs, uptime and open windows never produce this.
//! * **Idle** — the target stayed in front but no input arrived for at least
//!   [`IDLE_THRESHOLD_SECS`]. It is an overlay on foreground time, not a third
//!   additive bucket: it explains how much of the day was unattended, and is
//!   *never* counted as interaction.
//!
//! Sources report **increments**, not totals, and every increment is placed in
//! a fixed [`ACTIVITY_BUCKET_SECS`] bucket. That makes ingestion idempotent
//! enough to survive a retried request (a duplicate can never push one target
//! past the wall-clock length of a bucket) and makes a day's total a plain
//! `SUM` over buckets. Cross-source double counting is resolved at read time
//! by taking the per-bucket maximum instead of the sum: a browser tab and the
//! browser application cannot both add a full minute to the same minute.
//!
//! Nothing here stores URLs — only the executable name or hostname the caller
//! sends, plus the local date the bucket falls on.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;

/// Length of one activity bucket. The extension and the foreground watcher
/// both report every five seconds; a minute is coarse enough that retries and
/// clock jitter cannot inflate a total meaningfully, and fine enough to draw a
/// day without a huge table.
pub const ACTIVITY_BUCKET_SECS: i64 = 60;

/// No input for this long means the interval is idle, not interaction.
pub const IDLE_THRESHOLD_SECS: u32 = 60;

/// Reported increments are clamped to this, matching the allowance tick path.
pub const MAX_INCREMENT_SECS: u32 = 120;

/// Where a sample came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ActivitySource {
    /// The foreground-window watcher: `target` is the executable name.
    ForegroundApp,
    /// The browser extension: `target` is the hostname of the active tab.
    BrowserTab,
}

impl ActivitySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ForegroundApp => "app",
            Self::BrowserTab => "web",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "app" => Some(Self::ForegroundApp),
            "web" => Some(Self::BrowserTab),
            _ => None,
        }
    }

    pub fn is_web(self) -> bool {
        matches!(self, Self::BrowserTab)
    }
}

/// One measured interval, as sent by a collector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ActivityTick {
    pub source: ActivitySource,
    /// Executable name for apps, canonical hostname for web.
    pub target: String,
    /// Seconds this interval represents. Clamped server-side.
    #[serde(default)]
    pub increment_secs: Option<u32>,
    /// A human produced input during the interval.
    #[serde(default)]
    pub interaction: bool,
    /// The interval ended with the user idle past the threshold.
    #[serde(default)]
    pub idle: bool,
    /// When the collector observed it. Absent means "now"; present makes a
    /// retried request land in the same bucket as the original.
    #[serde(default)]
    pub observed_at: Option<DateTime<Utc>>,
    /// Identifies the reporting installation, so two machines can share a
    /// database later without their minutes being merged.
    #[serde(default)]
    pub device_id: Option<String>,
}

impl ActivityTick {
    /// Trim and validate in place. Returns `false` when the tick cannot name a
    /// target and should be dropped rather than counted.
    pub fn normalize(&mut self) {
        self.target = match self.source {
            ActivitySource::BrowserTab => crate::host::canonical_host(&self.target),
            ActivitySource::ForegroundApp => crate::process::running_name(&self.target)
                .trim()
                .to_lowercase(),
        };
        self.increment_secs = Some(
            self.increment_secs
                .unwrap_or(5)
                .clamp(1, MAX_INCREMENT_SECS),
        );
        self.device_id = Some(
            self.device_id
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .unwrap_or("local")
                .to_string(),
        );
    }

    pub fn is_usable(&self) -> bool {
        !self.target.is_empty()
    }

    pub fn increment(&self) -> u32 {
        self.increment_secs.unwrap_or(5).clamp(1, MAX_INCREMENT_SECS)
    }

    /// The bucket this tick belongs to. Local time matters: a "day" on the
    /// Activity page is the user's day, so buckets are floored in local time
    /// and stored as an epoch second.
    pub fn bucket_start(&self) -> DateTime<Utc> {
        let at = self.observed_at.unwrap_or_else(Utc::now);
        let local = at.with_timezone(&chrono::Local);
        let secs = local.timestamp().rem_euclid(ACTIVITY_BUCKET_SECS);
        let floored = local - chrono::Duration::seconds(secs);
        floored.with_timezone(&Utc)
    }

    /// How much of this interval may be credited to each column.
    ///
    /// Foreground is credited whenever the target was in front, even if the
    /// user had stopped typing: reading is still screen time. `interaction`
    /// remains the stricter subset with input, and `idle` runs alongside
    /// foreground so unattended time is visible without being added to
    /// interaction.
    pub fn credit(&self) -> (u32, u32, u32) {
        let seconds = self.increment();
        if self.idle {
            (seconds, 0, seconds)
        } else if self.interaction {
            (seconds, seconds, 0)
        } else {
            (seconds, 0, 0)
        }
    }
}

/// A visit to a target, used to count sessions for a session-limit plan.
///
/// Deliberately minimal: a target and a timestamp. Whether two visits belong
/// to the same session is a read-time decision based on the gap between them,
/// so a late report cannot rewrite history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ActivityVisit {
    pub source: ActivitySource,
    pub target: String,
    #[serde(default)]
    pub observed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub device_id: Option<String>,
}

impl ActivityVisit {
    pub fn normalize(&mut self) {
        self.target = match self.source {
            ActivitySource::BrowserTab => crate::host::canonical_host(&self.target),
            ActivitySource::ForegroundApp => crate::process::running_name(&self.target)
                .trim()
                .to_lowercase(),
        };
        self.device_id = Some(
            self.device_id
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .unwrap_or("local")
                .to_string(),
        );
    }

    pub fn is_usable(&self) -> bool {
        !self.target.is_empty()
    }

    pub fn at(&self) -> DateTime<Utc> {
        self.observed_at.unwrap_or_else(Utc::now)
    }
}

/// Time inside one local day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ActivityDay {
    pub date: NaiveDate,
    #[specta(type = specta_typescript::Number)]
    pub foreground_secs: u64,
    #[specta(type = specta_typescript::Number)]
    pub interaction_secs: u64,
    #[specta(type = specta_typescript::Number)]
    pub idle_secs: u64,
}

/// Per-target totals for the range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ActivityTarget {
    pub target: String,
    pub source: ActivitySource,
    #[specta(type = specta_typescript::Number)]
    pub foreground_secs: u64,
    #[specta(type = specta_typescript::Number)]
    pub interaction_secs: u64,
    #[specta(type = specta_typescript::Number)]
    pub idle_secs: u64,
    pub last_seen: DateTime<Utc>,
}

/// Why the page has, or has not, got numbers to show.
///
/// Four distinct situations that used to look identical ("0s"):
/// the platform cannot measure, the user disabled the collector, the collector
/// is running but has not written anything yet, and there is data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ActivityMeasurement {
    /// No source capable of reporting is available on this machine.
    Unavailable,
    /// The user turned activity collection off; zero is a choice, not a gap.
    Disabled,
    /// Sources are available, but nothing has ever been recorded.
    AwaitingData,
    /// Data exists in the database, just not in the requested range.
    NothingInRange,
    /// Data exists for the requested range.
    Recorded,
}

impl ActivityMeasurement {
    /// Stable machine-readable name; the UI localises its own labels from this.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Disabled => "disabled",
            Self::AwaitingData => "awaiting_data",
            Self::NothingInRange => "nothing_in_range",
            Self::Recorded => "recorded",
        }
    }
}

/// Everything the Activity page needs for one range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ActivityOverview {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub measurement: ActivityMeasurement,
    /// Foreground seconds, de-duplicated across sources per bucket: one minute
    /// cannot be counted twice just because a browser and its process both
    /// reported it.
    #[specta(type = specta_typescript::Number)]
    pub foreground_secs: u64,
    #[specta(type = specta_typescript::Number)]
    pub interaction_secs: u64,
    #[specta(type = specta_typescript::Number)]
    pub idle_secs: u64,
    /// Targets with any foreground time in the range.
    pub active_targets: u32,
    /// Rows in the whole database, used to tell "not collecting" apart from
    /// "nothing in this range".
    #[specta(type = specta_typescript::Number)]
    pub total_sample_rows: u64,
    pub last_sample_at: Option<DateTime<Utc>>,
    pub days: Vec<ActivityDay>,
    pub targets: Vec<ActivityTarget>,
    /// Whether the browser extension checked in recently.
    pub extension_connected: bool,
    /// Whether this platform exposes the foreground window.
    pub app_usage_measurable: bool,
    /// Whether the foreground watcher has been configured off.
    pub collection_enabled: bool,
}

impl ActivityOverview {
    /// True when the page can show a real number.
    pub fn has_data(&self) -> bool {
        self.measurement == ActivityMeasurement::Recorded
    }
}

/// Health of the collectors, separate from the numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ActivityHealth {
    pub app_usage_measurable: bool,
    pub extension_connected: bool,
    pub collection_enabled: bool,
    #[specta(type = specta_typescript::Number)]
    pub total_sample_rows: u64,
    pub last_sample_at: Option<DateTime<Utc>>,
    /// Minimum foreground time that would be recorded for a day, used by the
    /// UI to explain when a range is too old for the retention window.
    pub retention_days: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn tick(increment: u32, interaction: bool, idle: bool) -> ActivityTick {
        ActivityTick {
            source: ActivitySource::BrowserTab,
            target: "WWW.Example.COM".into(),
            increment_secs: Some(increment),
            interaction,
            idle,
            observed_at: None,
            device_id: None,
        }
    }

    #[test]
    fn normalization_canonicalizes_targets_and_clamps_increments() {
        let mut t = tick(0, false, false);
        t.normalize();
        assert_eq!(t.target, "example.com");
        assert_eq!(t.increment(), 1, "zero is raised to the minimum");

        let mut t = tick(9999, false, false);
        t.normalize();
        assert_eq!(t.increment(), MAX_INCREMENT_SECS);
    }

    #[test]
    fn an_empty_target_is_not_usable() {
        let mut t = tick(5, false, false);
        t.target = "   ".into();
        t.normalize();
        assert!(!t.is_usable());
    }

    #[test]
    fn interaction_is_a_subset_of_foreground_and_idle_runs_alongside_it() {
        assert_eq!(tick(5, true, false).credit(), (5, 5, 0));
        assert_eq!(tick(5, false, false).credit(), (5, 0, 0));
        assert_eq!(tick(5, false, true).credit(), (5, 0, 5));
    }

    #[test]
    fn a_bucket_is_whole_local_minutes() {
        let at = Utc.with_ymd_and_hms(2026, 11, 15, 12, 34, 56).unwrap();
        let mut t = tick(5, false, false);
        t.observed_at = Some(at);
        let bucket = t.bucket_start();
        assert_eq!(bucket.timestamp() % ACTIVITY_BUCKET_SECS, 0);
        let local_offset = bucket.with_timezone(&chrono::Local).offset().local_minus_utc();
        assert_eq!(
            (bucket.timestamp() + i64::from(local_offset)) % ACTIVITY_BUCKET_SECS,
            0
        );
    }
}
