//! Plan runtime: measures a plan's targets and decides whether it enforces.
//!
//! Everything here is derived from activity samples plus a small amount of
//! persisted pause state. There is no shadow counter to drift: a day is
//! re-measured from the same rows the Activity page shows, so the number on the
//! plan card and the number in the chart cannot disagree.
//!
//! Precedence is enforced by the caller (`Engine`): protection and scheduled
//! locks are checked before this module's verdict is applied, and a plan can
//! only ever add blocking, never remove it.

use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use focuser_common::error::{FocuserError, Result};
use focuser_common::plan::{PlanMode, PlanRuntime, PlanStatus, target_in_list};
use focuser_common::types::{BlockList, EntityId};
use serde::{Deserialize, Serialize};

use crate::db::Database;

/// Measured usage of one plan's targets today.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanUsage {
    pub used_secs: u64,
    pub sessions_used: u32,
    /// Length of the current unbroken stretch, ending at the last sample.
    pub current_streak_secs: u64,
    pub streak_started_at: Option<DateTime<Utc>>,
    pub last_sample_at: Option<DateTime<Utc>>,
}

/// Persisted pause bookkeeping for one list and one local day.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PauseState {
    pub used: u32,
    pub last_pause_end: Option<DateTime<Utc>>,
    pub paused_until: Option<DateTime<Utc>>,
}

fn pause_key(list_id: EntityId, day: NaiveDate) -> String {
    format!("plan_pause:{list_id}:{day}")
}

pub fn pause_state(db: &Database, list_id: EntityId, day: NaiveDate) -> Result<PauseState> {
    match db.get_setting(&pause_key(list_id, day))? {
        Some(json) => serde_json::from_str(&json).map_err(FocuserError::from),
        None => Ok(PauseState::default()),
    }
}

fn save_pause_state(
    db: &Database,
    list_id: EntityId,
    day: NaiveDate,
    state: &PauseState,
) -> Result<()> {
    let json = serde_json::to_string(state)?;
    db.set_setting(&pause_key(list_id, day), &json)
}

/// Start a bounded pause. Refuses when the list is protected (a pause is a way
/// to loosen enforcement and must never be available under a lock), when the
/// daily budget is spent, or when the minimum gap has not passed.
pub fn start_pause(db: &Database, list: &BlockList, now: DateTime<Utc>) -> Result<PauseState> {
    if list.has_active_protection() {
        return Err(FocuserError::ProtectionActive(list.id.to_string()));
    }
    let Some(policy) = list.plan.as_ref().and_then(|p| p.pauses) else {
        return Err(FocuserError::InvalidConfig(
            "this plan does not allow pauses".into(),
        ));
    };
    let day = now.with_timezone(&Local).date_naive();
    let mut state = pause_state(db, list.id, day)?;
    if state.paused_until.is_some_and(|until| now < until) {
        return Ok(state);
    }
    if state.used >= policy.allowed_per_day {
        return Err(FocuserError::InvalidConfig(
            "no pauses left today".into(),
        ));
    }
    if let Some(last_end) = state.last_pause_end {
        let gap = Duration::minutes(i64::from(policy.min_gap_minutes));
        if now < last_end + gap {
            return Err(FocuserError::InvalidConfig(
                "the minimum gap between pauses has not passed".into(),
            ));
        }
    }
    state.used += 1;
    state.paused_until = Some(now + Duration::minutes(i64::from(policy.duration_minutes)));
    save_pause_state(db, list.id, day, &state)?;
    Ok(state)
}

/// End a pause early. Allowed: shortening a break, never extending it past the
/// configured duration, so this cannot become an unlimited bypass.
pub fn end_pause(db: &Database, list: &BlockList, now: DateTime<Utc>) -> Result<PauseState> {
    let day = now.with_timezone(&Local).date_naive();
    let mut state = pause_state(db, list.id, day)?;
    if state.paused_until.is_none() {
        return Ok(state);
    }
    state.paused_until = None;
    state.last_pause_end = Some(now);
    save_pause_state(db, list.id, day, &state)?;
    Ok(state)
}

/// Group bucket timestamps into sessions: a gap of at least `gap_minutes`
/// starts a new visit. Buckets inside one visit are 60 s apart, so the rule is
/// deliberately much larger than the bucket size.
pub fn count_sessions(times: &[DateTime<Utc>], gap_minutes: u32) -> u32 {
    let gap = Duration::minutes(i64::from(gap_minutes.max(1)));
    let mut count = 0;
    let mut previous: Option<DateTime<Utc>> = None;
    for at in times {
        if previous.is_none_or(|prev| *at - prev >= gap) {
            count += 1;
        }
        previous = Some(*at);
    }
    count
}

/// Continuous stretch ending at the last sample. A gap of more than one bucket
/// ends the stretch: the user stepped away, so a break reminder restarts.
pub fn current_streak(times: &[DateTime<Utc>]) -> (u64, Option<DateTime<Utc>>) {
    let Some(last) = times.last() else {
        return (0, None);
    };
    let mut start = *last;
    for at in times.iter().rev().skip(1) {
        if *last - *at > Duration::seconds(120) {
            break;
        }
        start = *at;
    }
    let secs = (*last - start).num_seconds().max(0) as u64 + 60;
    (secs, Some(start))
}

pub fn measure_plan_usage(db: &Database, list: &BlockList, now: DateTime<Utc>) -> Result<PlanUsage> {
    let today = now.with_timezone(&Local).date_naive();
    measure_plan_usage_between(db, list, today, today, now)
}

/// Measure a plan's targets across an inclusive local-date range. Daily plans
/// pass one day; goals pass their whole period so a weekly target is not
/// compared against a single day's usage.
pub fn measure_plan_usage_between(
    db: &Database,
    list: &BlockList,
    from: NaiveDate,
    to: NaiveDate,
    now: DateTime<Utc>,
) -> Result<PlanUsage> {
    let (start, end) = crate::db::activity::local_range(from, to)?;
    let targets = db.activity_targets(start, end)?;
    // A browser tab and its foreground process can describe the same minute.
    // Per bucket, keep at most one matching sample: the web target wins because
    // it is more specific; otherwise the app sample is the only match.
    let mut per_bucket: std::collections::BTreeMap<DateTime<Utc>, (bool, u64)> =
        std::collections::BTreeMap::new();
    for (source, target) in targets {
        if !target_in_list(list, source, &target) {
            continue;
        }
        let is_web = source.is_web();
        for (at, secs) in db.activity_buckets_for_target(source, &target, start, end)? {
            if secs == 0 {
                continue;
            }
            per_bucket
                .entry(at)
                .and_modify(|slot| {
                    if (is_web && !slot.0) || (is_web == slot.0 && secs > slot.1) {
                        *slot = (is_web, secs);
                    }
                })
                .or_insert((is_web, secs));
        }
    }
    let used_secs = per_bucket.values().map(|(_, secs)| secs).sum();
    let times: Vec<DateTime<Utc>> = per_bucket.into_keys().collect();

    let (mut streak_secs, mut streak_start) = current_streak(&times);
    let last_sample_at = times.last().copied();
    // A streak that ended more than one bucket ago is not "current".
    if last_sample_at.is_some_and(|last| now - last > Duration::seconds(120)) {
        streak_secs = 0;
        streak_start = None;
    }

    let sessions_used = list
        .plan
        .as_ref()
        .and_then(|p| p.session_limit)
        .map(|limit| count_sessions(&times, limit.min_gap_minutes))
        .unwrap_or(0);

    Ok(PlanUsage {
        used_secs,
        sessions_used,
        current_streak_secs: streak_secs,
        streak_started_at: streak_start,
        last_sample_at: times.last().copied(),
    })
}

/// The verdict for one list, at one instant.
pub fn evaluate(db: &Database, list: &BlockList, now: DateTime<Utc>) -> Result<PlanRuntime> {
    let plan = list.plan.clone().unwrap_or_default();
    let usage = measure_plan_usage(db, list, now)?;
    let day = now.with_timezone(&Local).date_naive();
    let pause = pause_state(db, list.id, day)?;
    let paused_until = pause.paused_until.filter(|until| now < *until);

    let armed = list.enabled && list.is_effectively_active();
    let limit_secs = plan.daily_limit_secs();
    let limit_reached = limit_secs.is_some_and(|limit| usage.used_secs >= limit);
    let sessions_allowed = plan.session_limit.map(|s| s.max_sessions);
    let session_reached = sessions_allowed.is_some_and(|max| usage.sessions_used >= max);

    let break_due_at = plan.screen_break.and_then(|b| {
        usage.streak_started_at.map(|started| {
            started + Duration::minutes(i64::from(b.work_minutes))
        })
    });
    let break_due = break_due_at.is_some_and(|due| now >= due);
    let break_rest_over = plan.screen_break
        .and_then(|b| usage.last_sample_at.map(|last| (b, last)))
        .is_some_and(|(b, last)| {
            now - last >= Duration::minutes(i64::from(b.break_minutes)) && break_due
        });

    let status = if !list.enabled {
        PlanStatus::Inactive
    } else if paused_until.is_some() {
        PlanStatus::Paused
    } else if matches!(plan.mode, PlanMode::TimeLimit | PlanMode::SessionLimit)
        && (limit_reached || session_reached)
    {
        PlanStatus::LimitReached
    } else if plan.mode == PlanMode::TimeGoal
        && plan.time_goal.is_some_and(|g| usage.used_secs >= u64::from(g.daily_minutes) * 60)
    {
        PlanStatus::GoalReached
    } else if plan.mode == PlanMode::ScreenBreak {
        PlanStatus::Monitoring
    } else if armed {
        PlanStatus::Active
    } else {
        PlanStatus::Inactive
    };

    let blocking = armed
        && paused_until.is_none()
        && match plan.mode {
            PlanMode::Block => true,
            PlanMode::TimeLimit => limit_reached,
            PlanMode::SessionLimit => session_reached,
            PlanMode::ScreenBreak | PlanMode::TimeGoal => false,
        };

    Ok(PlanRuntime {
        block_list_id: list.id,
        mode: plan.mode,
        armed,
        blocking,
        paused_until,
        quick_timer_ends_at: plan.quick_timer_ends_at,
        used_secs: usage.used_secs,
        limit_secs,
        sessions_used: usage.sessions_used,
        sessions_allowed,
        break_due_at: if break_due && !break_rest_over {
            break_due_at
        } else {
            None
        },
        status,
    })
}

/// Helper used by the engine and by tests: whether the plan blocks now.
pub fn blocks_now(db: &Database, list: &BlockList, now: DateTime<Utc>) -> Result<bool> {
    Ok(evaluate(db, list, now)?.blocking)
}

#[cfg(test)]
mod tests {
    use super::*;
    use focuser_common::activity::{ActivitySource, ActivityTick};
    use focuser_common::plan::{PausePolicy, SessionLimit, TimeLimit};
    use focuser_common::types::BlockList;

    fn list_with_plan(mode: PlanMode) -> BlockList {
        let mut list = BlockList::new("Focus");
        list.enabled = true;
        list.websites = vec![focuser_common::types::WebsiteRule::domain("youtube.com")];
        list.plan = Some(Plan::with_mode(mode));
        list
    }

    use focuser_common::plan::Plan;

    fn tick_at(target: &str, at: DateTime<Utc>, secs: u32) -> ActivityTick {
        let mut t = ActivityTick {
            source: ActivitySource::BrowserTab,
            target: target.into(),
            increment_secs: Some(secs),
            interaction: true,
            idle: false,
            observed_at: Some(at),
            device_id: Some("test".into()),
        };
        t.normalize();
        t
    }

    #[test]
    fn a_plain_block_is_armed_and_blocking() {
        let db = Database::open_in_memory().unwrap();
        let list = list_with_plan(PlanMode::Block);
        let rt = evaluate(&db, &list, Utc::now()).unwrap();
        assert!(rt.blocking);
        assert_eq!(rt.status, PlanStatus::Active);
    }

    #[test]
    fn a_time_limit_only_blocks_after_the_budget_is_spent() {
        let db = Database::open_in_memory().unwrap();
        let mut list = list_with_plan(PlanMode::TimeLimit);
        list.plan.as_mut().unwrap().time_limit = Some(TimeLimit { daily_minutes: 1 });
        let now = Utc::now();
        db.record_activity_tick(&tick_at("youtube.com", now, 60))
            .unwrap();
        let rt = evaluate(&db, &list, now + Duration::seconds(1)).unwrap();
        assert_eq!(rt.used_secs, 60);
        assert!(rt.blocking);
        assert_eq!(rt.status, PlanStatus::LimitReached);
    }

    #[test]
    fn a_quick_timer_arms_a_plan_outside_its_schedule() {
        use chrono::Datelike;
        use focuser_common::types::{Schedule, TimeSlot};
        let mut list = list_with_plan(PlanMode::Block);
        let now = Utc::now();
        let other_day = Local::now().weekday().succ().succ();
        list.schedule = Some(Schedule {
            id: focuser_common::types::new_id(),
            name: "Weekdays".into(),
            enabled: true,
            time_slots: vec![TimeSlot {
                day: other_day,
                start: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                end: chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            }],
        });
        assert!(!list.is_effectively_active(), "outside the slot");
        list.plan.as_mut().unwrap().quick_timer_ends_at = Some(now + Duration::minutes(25));
        assert!(list.is_effectively_active(), "the timer is an explicit now");
        // The user's off switch still wins: a quick timer is not a bypass.
        list.enabled = false;
        assert!(!list.is_effectively_active());
    }

    #[test]
    fn sessions_are_split_by_the_configured_gap() {
        let a = Utc::now();
        let times = vec![
            a,
            a + Duration::minutes(1),
            a + Duration::minutes(2),
            a + Duration::minutes(40),
            a + Duration::minutes(41),
        ];
        assert_eq!(count_sessions(&times, 30), 2);
        assert_eq!(count_sessions(&times, 120), 1);
    }

    #[test]
    fn a_session_limit_blocks_once_the_cap_is_reached() {
        let db = Database::open_in_memory().unwrap();
        let mut list = list_with_plan(PlanMode::SessionLimit);
        list.plan.as_mut().unwrap().session_limit = Some(SessionLimit {
            max_sessions: 1,
            min_gap_minutes: 30,
        });
        let now = Utc::now();
        db.record_activity_tick(&tick_at("youtube.com", now, 60))
            .unwrap();
        let rt = evaluate(&db, &list, now).unwrap();
        assert_eq!(rt.sessions_used, 1);
        assert!(rt.blocking);
    }

    #[test]
    fn pauses_spend_a_daily_budget_and_expire() {
        let db = Database::open_in_memory().unwrap();
        let mut list = list_with_plan(PlanMode::Block);
        list.plan.as_mut().unwrap().pauses = Some(PausePolicy {
            allowed_per_day: 1,
            duration_minutes: 5,
            min_gap_minutes: 0,
        });
        let now = Utc::now();
        let state = start_pause(&db, &list, now).unwrap();
        assert!(state.paused_until.is_some());
        let rt = evaluate(&db, &list, now + Duration::seconds(1)).unwrap();
        assert_eq!(rt.status, PlanStatus::Paused);
        assert!(!rt.blocking);
        // Asking again while the pause runs is idempotent, not a second pause.
        let again = start_pause(&db, &list, now + Duration::seconds(2)).unwrap();
        assert_eq!(again.used, 1);
        end_pause(&db, &list, now + Duration::seconds(3)).unwrap();
        let err = start_pause(&db, &list, now + Duration::seconds(4)).unwrap_err();
        assert!(matches!(err, FocuserError::InvalidConfig(_)));
        // Once it expires, blocking returns.
        let rt = evaluate(&db, &list, now + Duration::minutes(6)).unwrap();
        assert!(rt.blocking);
    }

    #[test]
    fn a_pause_is_refused_under_protection() {
        let db = Database::open_in_memory().unwrap();
        let mut list = list_with_plan(PlanMode::Block);
        list.protection = Some(focuser_common::types::Protection::until_unlocked());
        list.lock = Some(focuser_common::types::Lock::RandomText { length: 20 });
        list.plan.as_mut().unwrap().pauses = Some(PausePolicy::default());
        assert!(start_pause(&db, &list, Utc::now()).is_err());
    }

    #[test]
    fn a_screen_break_never_blocks_and_reports_a_due_reminder() {
        let db = Database::open_in_memory().unwrap();
        let mut list = list_with_plan(PlanMode::ScreenBreak);
        list.plan.as_mut().unwrap().screen_break =
            Some(focuser_common::plan::ScreenBreak {
                work_minutes: 1,
                break_minutes: 5,
            });
        let now = Utc::now();
        db.record_activity_tick(&tick_at("youtube.com", now, 60))
            .unwrap();
        // The bucket is the local minute of the tick, so a one-minute work
        // block is due once that minute has fully passed.
        let rt = evaluate(&db, &list, now + Duration::seconds(61)).unwrap();
        assert!(!rt.blocking);
        assert_eq!(rt.status, PlanStatus::Monitoring);
        assert!(rt.break_due_at.is_some());
    }
}
