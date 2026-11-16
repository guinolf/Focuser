//! Activity service: turns samples into the answer the UI is allowed to show.
//!
//! The distinction this module exists for is the one the old statistics page
//! got wrong: zero seconds is not the same as "not measured". Four states are
//! kept apart, and every one of them is derived from evidence:
//!
//! * `Unavailable` — neither the extension nor the foreground watcher can
//!   report anything on this machine.
//! * `Disabled` — the user turned collection off.
//! * `AwaitingData` — collection is possible but no sample has ever arrived.
//! * `NothingInRange` — samples exist, but none in the selected range.
//! * `Recorded` — there is measured time in the range.

use chrono::{NaiveDate, Utc};
use focuser_common::activity::{
    ActivityMeasurement, ActivityOverview, ActivitySource, ActivityTick, IDLE_THRESHOLD_SECS,
};
use focuser_common::allowance::AllowanceTick;
use focuser_common::error::Result;

use crate::db::Database;

pub const SETTING_RETENTION: &str = "activity_retention_days";
pub const DEFAULT_RETENTION_DAYS: u32 = 90;
pub const MAX_RETENTION_DAYS: u32 = 3650;

/// The user's retention window, clamped to the range the command layer allows.
pub fn retention_days(db: &Database) -> Result<u32> {
    Ok(db
        .get_setting(SETTING_RETENTION)?
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS)
        .clamp(1, MAX_RETENTION_DAYS))
}

/// Apply the retention window now. Called at startup and periodically by the
/// foreground watcher so old samples do not accumulate on a machine that runs
/// for weeks.
pub fn apply_retention(db: &Database) -> Result<u64> {
    db.cleanup_old_activity(retention_days(db)?)
}

/// Build the overview with the measurement state filled in.
///
/// `extension_connected` and `app_usage_measurable` come from the blocking
/// health check; `collection_enabled` from the user setting. Passing them in
/// rather than reading them here keeps the service testable without a live
/// browser.
pub fn overview(
    db: &Database,
    from: NaiveDate,
    to: NaiveDate,
    extension_connected: bool,
    app_usage_measurable: bool,
    collection_enabled: bool,
) -> Result<ActivityOverview> {
    let mut overview = db.activity_overview(from, to)?;
    overview.extension_connected = extension_connected;
    overview.app_usage_measurable = app_usage_measurable;
    overview.collection_enabled = collection_enabled;
    overview.measurement = measurement_for(&overview, extension_connected, app_usage_measurable, collection_enabled);
    Ok(overview)
}

/// Record the time a source just reported, as an activity sample.
///
/// Both feeders (the browser extension through the REST tick, and the local
/// foreground watcher) go through here, so the semantics cannot drift between
/// them: interaction is human input within the idle threshold, idle is the
/// rest of the foreground time, and a tick with no measurable increment writes
/// nothing at all.
pub fn record_tick(db: &Database, tick: &AllowanceTick) -> Result<bool> {
    // The Activity page switch stops local recording at the write path, not
    // only in the UI. Allowance accounting still runs in its own path.
    if db
        .get_setting("activity_collection")?
        .is_some_and(|value| value == "off")
    {
        return Ok(false);
    }
    // A browser tick only describes foreground time while its window is
    // actually focused. Older extensions omit the field, so the coarse
    // `active` fallback is kept for them instead of inventing a precise claim.
    if tick.hostname.as_deref().is_some_and(|host| !host.trim().is_empty())
        && tick.window_focused == Some(false)
    {
        return Ok(false);
    }
    let (source, target) = match (&tick.hostname, &tick.app_exe) {
        (Some(host), _) if !host.trim().is_empty() => (ActivitySource::BrowserTab, host.as_str()),
        (None, Some(app)) if !app.trim().is_empty() => {
            (ActivitySource::ForegroundApp, app.as_str())
        }
        _ => return Ok(false),
    };
    let secs = tick.increment();
    if secs == 0 {
        return Ok(false);
    }
    let human = tick.is_human_use(IDLE_THRESHOLD_SECS);
    let mut sample = ActivityTick {
        source,
        target: target.into(),
        increment_secs: Some(secs),
        interaction: human,
        idle: !human,
        observed_at: Some(Utc::now()),
        device_id: None,
    };
    sample.normalize();
    if sample.target.is_empty() {
        return Ok(false);
    }
    db.record_activity_tick(&sample)?;
    Ok(true)
}

fn measurement_for(
    overview: &ActivityOverview,
    extension_connected: bool,
    app_usage_measurable: bool,
    collection_enabled: bool,
) -> ActivityMeasurement {
    if !collection_enabled {
        return ActivityMeasurement::Disabled;
    }
    if overview.total_sample_rows == 0 {
        // No samples ever: decide between "cannot measure" and "not yet".
        if !extension_connected && !app_usage_measurable {
            return ActivityMeasurement::Unavailable;
        }
        return ActivityMeasurement::AwaitingData;
    }
    if overview.foreground_secs == 0
        && overview.interaction_secs == 0
        && overview.idle_secs == 0
    {
        return ActivityMeasurement::NothingInRange;
    }
    ActivityMeasurement::Recorded
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, Utc};
    use focuser_common::activity::{ActivitySource, ActivityTick};

    fn tick(target: &str, secs: u32) -> ActivityTick {
        let mut t = ActivityTick {
            source: ActivitySource::BrowserTab,
            target: target.into(),
            increment_secs: Some(secs),
            interaction: true,
            idle: false,
            observed_at: Some(Utc::now()),
            device_id: None,
        };
        t.normalize();
        t
    }

    #[test]
    fn no_extension_and_no_watcher_means_unavailable() {
        let db = Database::open_in_memory().unwrap();
        let today = Local::now().date_naive();
        let o = overview(&db, today, today, false, false, true).unwrap();
        assert_eq!(o.measurement, ActivityMeasurement::Unavailable);
    }

    #[test]
    fn a_connected_browser_without_samples_is_awaiting_data() {
        let db = Database::open_in_memory().unwrap();
        let today = Local::now().date_naive();
        let o = overview(&db, today, today, true, true, true).unwrap();
        assert_eq!(o.measurement, ActivityMeasurement::AwaitingData);
    }

    #[test]
    fn collection_can_be_turned_off() {
        let db = Database::open_in_memory().unwrap();
        let today = Local::now().date_naive();
        let o = overview(&db, today, today, true, true, false).unwrap();
        assert_eq!(o.measurement, ActivityMeasurement::Disabled);
    }

    #[test]
    fn an_explicit_off_setting_stops_new_samples() {
        let db = Database::open_in_memory().unwrap();
        db.set_setting("activity_collection", "off").unwrap();
        let tick = focuser_common::allowance::AllowanceTick {
            url: None,
            shared_active: false,
            shared_only: false,
            hostname: Some("example.com".into()),
            app_exe: None,
            active: true,
            source: "test".into(),
            increment_secs: Some(30),
            window_focused: Some(true),
            idle_secs: Some(0),
        };
        assert!(!record_tick(&db, &tick).unwrap());
        let o = overview(
            &db,
            Local::now().date_naive(),
            Local::now().date_naive(),
            true,
            true,
            true,
        )
        .unwrap();
        assert_eq!(o.total_sample_rows, 0);
    }

    #[test]
    fn samples_elsewhere_are_not_reported_as_zero_without_a_reason() {
        let db = Database::open_in_memory().unwrap();
        db.record_activity_tick(&tick("example.com", 30)).unwrap();
        let today = Local::now().date_naive();
        let old = today - chrono::Duration::days(10);
        let o = overview(&db, old, old, true, true, true).unwrap();
        assert_eq!(o.measurement, ActivityMeasurement::NothingInRange);
        assert_eq!(o.total_sample_rows, 1);
    }

    #[test]
    fn measured_time_is_recorded() {
        let db = Database::open_in_memory().unwrap();
        db.record_activity_tick(&tick("example.com", 30)).unwrap();
        let today = Local::now().date_naive();
        let o = overview(&db, today, today, true, true, true).unwrap();
        assert_eq!(o.measurement, ActivityMeasurement::Recorded);
        assert_eq!(o.foreground_secs, 30);
    }

    #[test]
    fn an_unfocused_browser_tick_is_not_foreground_time() {
        let db = Database::open_in_memory().unwrap();
        let tick = focuser_common::allowance::AllowanceTick {
            url: None,
            shared_active: false,
            shared_only: false,
            hostname: Some("example.com".into()),
            app_exe: None,
            active: true,
            source: "test".into(),
            increment_secs: Some(30),
            window_focused: Some(false),
            idle_secs: Some(0),
        };
        assert!(!record_tick(&db, &tick).unwrap());
        let today = Local::now().date_naive();
        let o = overview(&db, today, today, true, true, true).unwrap();
        assert_eq!(o.foreground_secs, 0);
        assert_eq!(o.total_sample_rows, 0);
    }
}

