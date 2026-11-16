//! Persistence for activity samples and visits.
//!
//! Buckets live in `activity_samples`, keyed by
//! `(bucket_start, source, target, device_id)`. Two invariants are enforced by
//! the schema and the upsert itself, not by the caller:
//!
//! * No column can exceed the bucket length, so a duplicate or malicious tick
//!   can never inflate a minute.
//! * Reads take `MAX` per bucket across targets, so the same minute reported by
//!   the browser and by the browser's process counts once. Interaction and
//!   idle are clamped to that foreground minute instead of being summed.

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use focuser_common::activity::{
    ACTIVITY_BUCKET_SECS, ActivityDay, ActivityOverview, ActivitySource, ActivityTarget, ActivityTick,
    ActivityVisit,
};
use focuser_common::error::{FocuserError, Result};
use rusqlite::{Connection, params};
use tracing::debug;

use super::Database;

impl Database {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))
    }

    /// Record one interval. Safe to call again with the same tick: the second
    /// write lands in the same bucket and is capped at the bucket length.
    pub fn record_activity_tick(&self, tick: &ActivityTick) -> Result<()> {
        let conn = self.lock()?;
        record_activity_tick(&conn, tick)
    }

    pub fn record_activity_visit(&self, visit: &ActivityVisit) -> Result<bool> {
        let conn = self.lock()?;
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO activity_visits (at, source, target, device_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    visit.at().timestamp(),
                    visit.source.as_str(),
                    visit.target,
                    visit.device_id.as_deref().unwrap_or("local"),
                ],
            )
            .map_err(db_err)?;
        Ok(inserted > 0)
    }

    /// All-time sample count. Used to tell "not collecting" from "nothing in
    /// this range".
    pub fn activity_sample_count(&self) -> Result<u64> {
        let conn = self.lock()?;
        conn.query_row("SELECT COUNT(*) FROM activity_samples", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|n| n.max(0) as u64)
        .map_err(db_err)
    }

    pub fn last_activity_sample_at(&self) -> Result<Option<DateTime<Utc>>> {
        let conn = self.lock()?;
        let value: Option<i64> = conn
            .query_row("SELECT MAX(bucket_start) FROM activity_samples", [], |row| {
                row.get(0)
            })
            .map_err(db_err)?;
        Ok(value.and_then(|s| Utc.timestamp_opt(s, 0).single()))
    }

    /// Totals and per-target rows for an inclusive local-date range.
    ///
    /// `from`/`to` are local dates, because "today" is a local concept:
    /// turning them into UTC bounds here keeps the daylight-saving edge in one
    /// place instead of leaking `localtime` arithmetic into queries.
    pub fn activity_overview(&self, from: NaiveDate, to: NaiveDate) -> Result<ActivityOverview> {
        let conn = self.lock()?;
        let (start, end) = local_range(from, to)?;
        let days = query_days(&conn, start, end)?;
        let targets = query_targets(&conn, start, end)?;
        let total_rows: u64 = conn
            .query_row("SELECT COUNT(*) FROM activity_samples", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|n| n.max(0) as u64)
            .map_err(db_err)?;
        let last = conn
            .query_row("SELECT MAX(bucket_start) FROM activity_samples", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .map_err(db_err)?
            .and_then(|s| Utc.timestamp_opt(s, 0).single());

        let foreground_secs = days.iter().map(|d| d.foreground_secs).sum();
        let interaction_secs = days.iter().map(|d| d.interaction_secs).sum();
        let idle_secs = days.iter().map(|d| d.idle_secs).sum();

        Ok(ActivityOverview {
            from,
            to,
            measurement: focuser_common::activity::ActivityMeasurement::AwaitingData,
            foreground_secs,
            interaction_secs,
            idle_secs,
            active_targets: targets.len() as u32,
            total_sample_rows: total_rows,
            last_sample_at: last,
            days,
            targets,
            extension_connected: false,
            app_usage_measurable: false,
            collection_enabled: true,
        })
    }

    /// Delete activity older than the retention window. Returns rows removed.
    pub fn cleanup_old_activity(&self, keep_days: u32) -> Result<u64> {
        let conn = self.lock()?;
        let cutoff = local_day_start_utc(Local::now().date_naive() - chrono::Duration::days(keep_days as i64))?;
        let samples = conn
            .execute(
                "DELETE FROM activity_samples WHERE bucket_start < ?1",
                params![cutoff.timestamp()],
            )
            .map_err(db_err)?;
        let visits = conn
            .execute("DELETE FROM activity_visits WHERE at < ?1", params![cutoff.timestamp()])
            .map_err(db_err)?;
        debug!(samples, visits, "activity retention applied");
        Ok((samples + visits) as u64)
    }

    /// Clear all activity, used by `DeleteAllData` and the Activity page's
    /// explicit "clear" action.
    pub fn clear_activity(&self) -> Result<()> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM activity_samples", [])
            .map_err(db_err)?;
        conn.execute("DELETE FROM activity_visits", [])
            .map_err(db_err)?;
        Ok(())
    }

    /// Per-day, per-target totals for the statistics page.
    ///
    /// Foreground time is collapsed per bucket first, then summed per local
    /// day, so a browser tab and its process cannot double-count one minute.
    /// Blocked attempts come from `blocked_events`, the only table that
    /// records them with a timestamp.
    pub fn usage_stats(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<focuser_common::types::UsageStat>> {
        use std::collections::BTreeMap;

        let conn = self.lock()?;
        let (start, end) = local_range(from, to)?;
        let mut totals: BTreeMap<(NaiveDate, String), (u64, u64)> = BTreeMap::new();

        let mut stmt = conn
            .prepare(
                "SELECT day, target, SUM(fg) FROM (
                    SELECT date(bucket_start, 'unixepoch', 'localtime') AS day,
                           target, foreground_secs AS fg,
                           ROW_NUMBER() OVER (
                               PARTITION BY bucket_start
                               ORDER BY CASE source WHEN 'web' THEN 0 ELSE 1 END,
                                        foreground_secs DESC, target
                           ) AS rank
                    FROM activity_samples
                    WHERE bucket_start >= ?1 AND bucket_start < ?2
                 )
                 WHERE rank = 1
                 GROUP BY day, target",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![start.timestamp(), end.timestamp()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(db_err)?;
        for row in rows.flatten() {
            let Ok(day) = NaiveDate::parse_from_str(&row.0, "%Y-%m-%d") else {
                continue;
            };
            let entry = totals.entry((day, row.1)).or_default();
            entry.0 += row.2.max(0) as u64;
        }

        let mut stmt = conn
            .prepare(
                "SELECT date(timestamp, 'localtime') AS day, domain_or_app, COUNT(*)
                 FROM blocked_events
                 WHERE timestamp >= ?1 AND timestamp < ?2
                 GROUP BY day, domain_or_app",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(
                params![start.to_rfc3339(), end.to_rfc3339()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .map_err(db_err)?;
        for row in rows.flatten() {
            let Ok(day) = NaiveDate::parse_from_str(&row.0, "%Y-%m-%d") else {
                continue;
            };
            totals.entry((day, row.1)).or_default().1 += row.2.max(0) as u64;
        }

        Ok(totals
            .into_iter()
            .map(|((date, domain_or_app), (duration_seconds, blocked_attempts))| {
                focuser_common::types::UsageStat {
                    domain_or_app,
                    duration_seconds,
                    blocked_attempts,
                    date,
                }
            })
            .collect())
    }

    /// Per-bucket foreground seconds for one target between two UTC instants.
    /// Returned sorted so session counting can walk it.
    pub fn activity_buckets_for_target(
        &self,
        source: ActivitySource,
        target: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<(DateTime<Utc>, u64)>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT bucket_start, foreground_secs FROM activity_samples
                 WHERE source = ?1 AND target = ?2 AND bucket_start >= ?3 AND bucket_start < ?4
                 ORDER BY bucket_start ASC",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(
                params![source.as_str(), target, start.timestamp(), end.timestamp()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(db_err)?
            .filter_map(std::result::Result::ok)
            .filter_map(|(secs, value)| {
                Utc.timestamp_opt(secs, 0)
                    .single()
                    .map(|dt| (dt, value.max(0) as u64))
            })
            .collect();
        Ok(rows)
    }

    /// Every distinct target seen in a range, as `(source, target)`.
    pub fn activity_targets(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<(ActivitySource, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT source, target FROM activity_samples
                 WHERE bucket_start >= ?1 AND bucket_start < ?2
                 GROUP BY source, target
                 ORDER BY SUM(foreground_secs) DESC",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![start.timestamp(), end.timestamp()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_err)?
            .filter_map(std::result::Result::ok)
            .filter_map(|(source, target)| {
                ActivitySource::parse(&source).map(|source| (source, target))
            })
            .collect();
        Ok(rows)
    }

    /// Distinct visits of a target in a range, most recent last. Session
    /// boundaries are decided by the caller with a gap rule.
    pub fn activity_visits_since(
        &self,
        source: ActivitySource,
        target: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<DateTime<Utc>>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT at FROM activity_visits
                 WHERE source = ?1 AND target = ?2 AND at >= ?3 AND at < ?4
                 ORDER BY at ASC",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(
                params![source.as_str(), target, start.timestamp(), end.timestamp()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(db_err)?
            .filter_map(std::result::Result::ok)
            .filter_map(|secs| Utc.timestamp_opt(secs, 0).single())
            .collect();
        Ok(rows)
    }
}

/// Start of a local day, as UTC. Picks the earliest instant when the local
/// midnight happens twice (fall-back), so a range never starts mid-dawn.
pub fn local_day_start_utc(date: NaiveDate) -> Result<DateTime<Utc>> {
    let naive = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| FocuserError::Database("invalid date".into()))?;
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => Ok(dt.with_timezone(&Utc)),
        chrono::LocalResult::Ambiguous(a, b) => Ok(a.min(b).with_timezone(&Utc)),
        chrono::LocalResult::None => Ok((naive + chrono::Duration::hours(1))
            .and_local_timezone(Local)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now)),
    }
}

/// `[start, end)` UTC bounds for an inclusive local-date range.
pub fn local_range(from: NaiveDate, to: NaiveDate) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    if to < from {
        return Err(FocuserError::Database("range end is before start".into()));
    }
    let end_date = to
        .checked_add_days(chrono::Days::new(1))
        .ok_or_else(|| FocuserError::Database("range end out of bounds".into()))?;
    Ok((local_day_start_utc(from)?, local_day_start_utc(end_date)?))
}

fn record_activity_tick(conn: &Connection, tick: &ActivityTick) -> Result<()> {
    if !tick.is_usable() {
        return Ok(());
    }
    let bucket = tick.bucket_start();
    let (foreground, interaction, idle) = tick.credit();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO activity_samples
            (bucket_start, source, target, device_id, foreground_secs, interaction_secs, idle_secs, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(bucket_start, source, target, device_id) DO UPDATE SET
            foreground_secs = MIN(foreground_secs + excluded.foreground_secs, ?9),
            interaction_secs = MIN(interaction_secs + excluded.interaction_secs, ?9),
            idle_secs = MIN(idle_secs + excluded.idle_secs, ?9),
            updated_at = excluded.updated_at",
        params![
            bucket.timestamp(),
            tick.source.as_str(),
            tick.target,
            tick.device_id.as_deref().unwrap_or("local"),
            foreground,
            interaction,
            idle,
            now,
            ACTIVITY_BUCKET_SECS,
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

fn query_days(conn: &Connection, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<ActivityDay>> {
    // Inner query collapses each bucket to one value across sources; the outer
    // one groups those values into local days.
    let mut stmt = conn
        .prepare(
            "SELECT day, SUM(fg), SUM(MIN(inter, fg)), SUM(MIN(idle, fg)) FROM (
                SELECT date(bucket_start, 'unixepoch', 'localtime') AS day,
                       MAX(foreground_secs) AS fg,
                       MAX(interaction_secs) AS inter,
                       MAX(idle_secs) AS idle
                FROM activity_samples
                WHERE bucket_start >= ?1 AND bucket_start < ?2
                GROUP BY bucket_start
             )
             GROUP BY day ORDER BY day ASC",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![start.timestamp(), end.timestamp()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(db_err)?
        .filter_map(std::result::Result::ok)
        .filter_map(|(day, fg, inter, idle)| {
            NaiveDate::parse_from_str(&day, "%Y-%m-%d")
                .ok()
                .map(|date| ActivityDay {
                    date,
                    foreground_secs: fg.max(0) as u64,
                    interaction_secs: inter.max(0) as u64,
                    idle_secs: idle.max(0) as u64,
                })
        })
        .collect();
    Ok(rows)
}

fn query_targets(conn: &Connection, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<ActivityTarget>> {
    let mut stmt = conn
        .prepare(
            "SELECT source, target, SUM(foreground_secs), SUM(interaction_secs),
                    SUM(idle_secs), MAX(bucket_start)
             FROM (
                 SELECT source, target, foreground_secs, interaction_secs, idle_secs,
                        bucket_start,
                        ROW_NUMBER() OVER (
                            PARTITION BY bucket_start
                            ORDER BY CASE source WHEN 'web' THEN 0 ELSE 1 END,
                                     foreground_secs DESC, target
                        ) AS rank
                 FROM activity_samples
                 WHERE bucket_start >= ?1 AND bucket_start < ?2
             )
             WHERE rank = 1
             GROUP BY source, target
             HAVING SUM(foreground_secs) > 0
             ORDER BY SUM(foreground_secs) DESC",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![start.timestamp(), end.timestamp()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(db_err)?
        .filter_map(std::result::Result::ok)
        .filter_map(|(source, target, fg, inter, idle, last)| {
            let source = ActivitySource::parse(&source)?;
            let last_seen = Utc.timestamp_opt(last, 0).single()?;
            Some(ActivityTarget {
                target,
                source,
                foreground_secs: fg.max(0) as u64,
                interaction_secs: inter.max(0) as u64,
                idle_secs: idle.max(0) as u64,
                last_seen,
            })
        })
        .collect();
    Ok(rows)
}

fn db_err(e: rusqlite::Error) -> FocuserError {
    FocuserError::Database(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use focuser_common::activity::IDLE_THRESHOLD_SECS;

    fn tick(source: ActivitySource, target: &str, secs: u32, interaction: bool) -> ActivityTick {
        ActivityTick {
            source,
            target: target.into(),
            increment_secs: Some(secs),
            interaction,
            idle: false,
            observed_at: None,
            device_id: Some("test".into()),
        }
    }

    #[test]
    fn a_retried_tick_cannot_exceed_its_bucket() {
        let db = Database::open_in_memory().unwrap();
        let mut t = tick(ActivitySource::BrowserTab, "youtube.com", 30, true);
        t.normalize();
        for _ in 0..5 {
            db.record_activity_tick(&t).unwrap();
        }
        let today = Local::now().date_naive();
        let overview = db.activity_overview(today, today).unwrap();
        assert!(
            overview.foreground_secs <= ACTIVITY_BUCKET_SECS as u64,
            "got {}",
            overview.foreground_secs
        );
    }

    #[test]
    fn browser_and_app_minutes_are_not_added_twice() {
        let db = Database::open_in_memory().unwrap();
        let at = Utc::now();
        let mut web = tick(ActivitySource::BrowserTab, "youtube.com", 30, true);
        web.observed_at = Some(at);
        web.normalize();
        let mut app = tick(ActivitySource::ForegroundApp, "firefox", 30, true);
        app.observed_at = Some(at);
        app.normalize();
        db.record_activity_tick(&web).unwrap();
        db.record_activity_tick(&app).unwrap();

        let today = Local::now().date_naive();
        let overview = db.activity_overview(today, today).unwrap();
        assert_eq!(overview.foreground_secs, 30, "same minute counted once");
        assert_eq!(overview.targets.len(), 1, "one target owns the minute");
        assert_eq!(overview.targets[0].target, "youtube.com");
        assert_eq!(overview.targets[0].source, ActivitySource::BrowserTab);
    }

    #[test]
    fn idle_time_is_foreground_without_interaction() {
        let db = Database::open_in_memory().unwrap();
        let mut t = tick(ActivitySource::ForegroundApp, "steam", 60, false);
        t.idle = true;
        t.normalize();
        db.record_activity_tick(&t).unwrap();
        let today = Local::now().date_naive();
        let overview = db.activity_overview(today, today).unwrap();
        assert_eq!(overview.foreground_secs, 60);
        assert_eq!(overview.idle_secs, 60);
        assert_eq!(overview.interaction_secs, 0);
    }

    #[test]
    fn a_visit_is_recorded_once_per_second() {
        let db = Database::open_in_memory().unwrap();
        let at = Utc::now();
        let mut visit = ActivityVisit {
            source: ActivitySource::BrowserTab,
            target: "reddit.com".into(),
            observed_at: Some(at),
            device_id: None,
        };
        visit.normalize();
        assert!(db.record_activity_visit(&visit).unwrap());
        assert!(!db.record_activity_visit(&visit).unwrap());
        let today = Local::now().date_naive();
        let (start, _) = local_range(today, today).unwrap();
        assert_eq!(
            db.activity_visits_since(
                ActivitySource::BrowserTab,
                "reddit.com",
                start,
                Utc::now() + chrono::Duration::hours(1)
            )
            .unwrap()
            .len(),
            1
        );
    }

    #[test]
    fn overview_distinguishes_nothing_ever_from_nothing_in_range() {
        let db = Database::open_in_memory().unwrap();
        let today = Local::now().date_naive();
        let empty = db.activity_overview(today, today).unwrap();
        assert_eq!(empty.total_sample_rows, 0);
        assert_eq!(empty.foreground_secs, 0);

        let mut t = tick(ActivitySource::BrowserTab, "example.com", 10, false);
        t.normalize();
        db.record_activity_tick(&t).unwrap();
        let old = today - chrono::Duration::days(30);
        let earlier = db.activity_overview(old, old).unwrap();
        assert_eq!(earlier.total_sample_rows, 1);
        assert_eq!(earlier.foreground_secs, 0, "data exists, just not then");
    }

    #[test]
    fn cleaning_up_keeps_todays_rows() {
        let db = Database::open_in_memory().unwrap();
        let mut t = tick(ActivitySource::BrowserTab, "example.com", 10, false);
        t.normalize();
        db.record_activity_tick(&t).unwrap();
        db.cleanup_old_activity(7).unwrap();
        assert_eq!(db.activity_sample_count().unwrap(), 1);
    }

    #[test]
    fn an_unusable_target_is_ignored() {
        let db = Database::open_in_memory().unwrap();
        let mut t = tick(ActivitySource::BrowserTab, "  ", 10, false);
        t.normalize();
        assert!(!t.is_usable());
        db.record_activity_tick(&t).unwrap();
        assert_eq!(db.activity_sample_count().unwrap(), 0);
    }

    #[test]
    fn the_idle_threshold_is_at_least_the_tick_interval() {
        assert!(IDLE_THRESHOLD_SECS >= 5);
        assert_eq!(focuser_common::activity::MAX_INCREMENT_SECS, 120);
    }

    #[test]
    fn usage_stats_sum_minutes_once_per_target_and_day() {
        let db = Database::open_in_memory().unwrap();
        let today = Local::now().date_naive();
        let mut browser = tick(ActivitySource::BrowserTab, "example.com", 30, false);
        browser.normalize();
        let mut app = tick(ActivitySource::ForegroundApp, "example.com", 30, false);
        app.normalize();
        // Same bucket, same target, two sources: still 30 seconds, not 60.
        app.observed_at = browser.observed_at;
        db.record_activity_tick(&browser).unwrap();
        db.record_activity_tick(&app).unwrap();

        let stats = db.usage_stats(today, today).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].domain_or_app, "example.com");
        assert_eq!(stats[0].duration_seconds, 30);
        assert_eq!(stats[0].blocked_attempts, 0);
        assert_eq!(stats[0].date, today);
    }

    #[test]
    fn usage_stats_credit_one_target_per_minute_across_sources() {
        let db = Database::open_in_memory().unwrap();
        let today = Local::now().date_naive();
        let mut browser = tick(ActivitySource::BrowserTab, "example.com", 30, false);
        browser.normalize();
        let mut app = tick(ActivitySource::ForegroundApp, "firefox", 30, false);
        app.observed_at = browser.observed_at;
        app.normalize();
        db.record_activity_tick(&browser).unwrap();
        db.record_activity_tick(&app).unwrap();

        let stats = db.usage_stats(today, today).unwrap();
        assert_eq!(stats.len(), 1, "the same minute cannot belong to two targets");
        assert_eq!(stats[0].domain_or_app, "example.com");
        assert_eq!(stats[0].duration_seconds, 30);

        let overview = db.activity_overview(today, today).unwrap();
        assert_eq!(overview.foreground_secs, 30);
        assert_eq!(overview.targets.len(), 1);
        assert_eq!(overview.targets[0].target, "example.com");
    }

    #[test]
    fn a_bucket_keeps_foreground_interaction_and_idle_consistent_across_sources() {
        let db = Database::open_in_memory().unwrap();
        let mut browser = tick(ActivitySource::BrowserTab, "example.com", 40, true);
        browser.observed_at = Some(Utc::now());
        browser.normalize();
        db.record_activity_tick(&browser).unwrap();
        let mut idle = tick(ActivitySource::ForegroundApp, "reader", 30, false);
        idle.idle = true;
        idle.observed_at = browser.observed_at;
        idle.normalize();
        db.record_activity_tick(&idle).unwrap();

        let today = Local::now().date_naive();
        let overview = db.activity_overview(today, today).unwrap();
        assert_eq!(overview.foreground_secs, 40);
        assert_eq!(overview.interaction_secs, 40, "interaction cannot exceed foreground");
        assert_eq!(
            overview.idle_secs, 30,
            "idle stays inside the foreground minute after merging sources"
        );
        assert!(overview.idle_secs <= overview.foreground_secs);
    }
}
