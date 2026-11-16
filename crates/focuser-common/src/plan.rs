//! Plans: a block list's intent, beyond "which rules are armed".
//!
//! A [`BlockList`](crate::types::BlockList) answers *what* is blocked. A plan
//! answers *why* and *until when*: a plain block, a daily time allowance, a
//! session cap, a break schedule, or a time goal that never blocks anything.
//! Keeping the plan on the list (rather than as a second, parallel entity)
//! means every existing rule, exception, schedule and protection primitive
//! keeps working; a plan only changes how enforcement is decided.
//!
//! The modes are not cosmetic labels. Each one has explicit precedence:
//!
//! 1. Protection and scheduled locks always win — a plan cannot loosen a lock.
//! 2. A pause postpones enforcement for a bounded time, and only while the
//!    list's own pause budget allows it.
//! 3. A quick timer extends the armed window: while it runs the plan is
//!    enforced even outside its schedule.
//! 4. The mode decides whether armed means *blocking* (Block), *counting
//!    down* (TimeLimit, SessionLimit, TimeGoal) or *reminding* (ScreenBreak).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::activity::ActivitySource;
use crate::types::{BlockList, EntityId};

/// What kind of plan a list is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    /// The classic list: armed means its rules block.
    #[default]
    Block,
    /// Up to `daily_minutes` of the plan's targets per local day, then block.
    TimeLimit,
    /// Up to `max_sessions` separate visits per local day, then block.
    SessionLimit,
    /// Remind the user to step away; never blocks.
    ScreenBreak,
    /// Target amount of foreground time per day; never blocks.
    TimeGoal,
}

impl PlanMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::TimeLimit => "time_limit",
            Self::SessionLimit => "session_limit",
            Self::ScreenBreak => "screen_break",
            Self::TimeGoal => "time_goal",
        }
    }

    /// Whether this mode is allowed to block anything at all.
    pub fn can_block(self) -> bool {
        matches!(self, Self::Block | Self::TimeLimit | Self::SessionLimit)
    }
}

/// A bounded pause: the pressure valve every commitment system needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PausePolicy {
    /// Pauses allowed per local day. Zero disables pausing entirely.
    pub allowed_per_day: u32,
    /// How long one pause lasts.
    pub duration_minutes: u32,
    /// Minimum time between the end of one pause and the start of the next.
    pub min_gap_minutes: u32,
}

impl Default for PausePolicy {
    fn default() -> Self {
        Self {
            allowed_per_day: 2,
            duration_minutes: 5,
            min_gap_minutes: 30,
        }
    }
}

impl PausePolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.duration_minutes == 0 || self.duration_minutes > 120 {
            return Err("pause duration must be between 1 and 120 minutes".into());
        }
        if self.allowed_per_day > 12 {
            return Err("at most 12 pauses per day".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TimeLimit {
    pub daily_minutes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SessionLimit {
    pub max_sessions: u32,
    /// Visits closer together than this belong to the same session.
    pub min_gap_minutes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ScreenBreak {
    pub work_minutes: u32,
    pub break_minutes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TimeGoal {
    pub daily_minutes: u32,
}

/// A plan attached to a block list.
///
/// Every field is optional except the mode, so an old database (which has no
/// plan) deserialises into "no plan" and behaves exactly as before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Plan {
    #[serde(default)]
    pub mode: PlanMode,
    /// A one-off focus window. While it runs the plan is armed even if its
    /// schedule says otherwise; it never overrides protection.
    #[serde(default)]
    pub quick_timer_ends_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub pauses: Option<PausePolicy>,
    #[serde(default)]
    pub time_limit: Option<TimeLimit>,
    #[serde(default)]
    pub session_limit: Option<SessionLimit>,
    #[serde(default)]
    pub screen_break: Option<ScreenBreak>,
    #[serde(default)]
    pub time_goal: Option<TimeGoal>,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            mode: PlanMode::Block,
            quick_timer_ends_at: None,
            pauses: None,
            time_limit: None,
            session_limit: None,
            screen_break: None,
            time_goal: None,
        }
    }
}

impl Plan {
    pub fn block() -> Self {
        Self::default()
    }

    pub fn with_mode(mode: PlanMode) -> Self {
        let mut plan = Self {
            mode,
            ..Self::default()
        };
        match mode {
            PlanMode::Block => {}
            PlanMode::TimeLimit => plan.time_limit = Some(TimeLimit { daily_minutes: 60 }),
            PlanMode::SessionLimit => {
                plan.session_limit = Some(SessionLimit {
                    max_sessions: 5,
                    min_gap_minutes: 30,
                });
            }
            PlanMode::ScreenBreak => {
                plan.screen_break = Some(ScreenBreak {
                    work_minutes: 50,
                    break_minutes: 10,
                });
                plan.pauses = None;
            }
            PlanMode::TimeGoal => {
                plan.time_goal = Some(TimeGoal { daily_minutes: 120 });
                plan.pauses = None;
            }
        }
        plan
    }

    /// The configured daily limit for a counting mode, in seconds.
    pub fn daily_limit_secs(&self) -> Option<u64> {
        match self.mode {
            PlanMode::TimeLimit => self
                .time_limit
                .map(|l| u64::from(l.daily_minutes) * 60)
                .filter(|s| *s > 0),
            PlanMode::SessionLimit => None,
            _ => None,
        }
    }

    pub fn quick_timer_active(&self, now: DateTime<Utc>) -> bool {
        self.quick_timer_ends_at.is_some_and(|end| now < end)
    }

    /// Reject combinations that would leave a mode with nothing to enforce.
    pub fn validate(&self) -> Result<(), String> {
        match self.mode {
            PlanMode::Block => {}
            PlanMode::TimeLimit => match self.time_limit {
                Some(l) if (1..=24 * 60).contains(&l.daily_minutes) => {}
                _ => return Err("a time limit needs between 1 and 1440 minutes".into()),
            },
            PlanMode::SessionLimit => match self.session_limit {
                Some(s) if (1..=100).contains(&s.max_sessions) && s.min_gap_minutes <= 24 * 60 => {}
                _ => return Err("a session limit needs 1-100 sessions and a valid gap".into()),
            },
            PlanMode::ScreenBreak => match self.screen_break {
                Some(b) if (1..=240).contains(&b.work_minutes) && (1..=60).contains(&b.break_minutes) => {}
                _ => return Err("a screen break needs 1-240 work and 1-60 break minutes".into()),
            },
            PlanMode::TimeGoal => match self.time_goal {
                Some(g) if (1..=24 * 60).contains(&g.daily_minutes) => {}
                _ => return Err("a time goal needs between 1 and 1440 minutes".into()),
            },
        }
        if let Some(p) = self.pauses {
            p.validate()?;
        }
        Ok(())
    }

    /// Normalise: drop configuration that does not belong to the mode so a
    /// later mode switch cannot resurrect stale numbers.
    pub fn normalized(mut self) -> Self {
        match self.mode {
            PlanMode::Block => {
                self.time_limit = None;
                self.session_limit = None;
                self.screen_break = None;
                self.time_goal = None;
            }
            PlanMode::TimeLimit => {
                self.session_limit = None;
                self.screen_break = None;
                self.time_goal = None;
            }
            PlanMode::SessionLimit => {
                self.time_limit = None;
                self.screen_break = None;
                self.time_goal = None;
            }
            PlanMode::ScreenBreak => {
                self.time_limit = None;
                self.session_limit = None;
                self.time_goal = None;
            }
            PlanMode::TimeGoal => {
                self.time_limit = None;
                self.session_limit = None;
                self.screen_break = None;
            }
        }
        if self.quick_timer_ends_at.is_some_and(|end| end <= Utc::now()) {
            self.quick_timer_ends_at = None;
        }
        self
    }
}

/// The computed state of one plan, ready to render or enforce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PlanRuntime {
    pub block_list_id: EntityId,
    pub mode: PlanMode,
    /// The list's rules + schedule + quick timer say it is in scope.
    pub armed: bool,
    /// Enforcement is in force right now.
    pub blocking: bool,
    /// A pause is currently postponing enforcement.
    pub paused_until: Option<DateTime<Utc>>,
    pub quick_timer_ends_at: Option<DateTime<Utc>>,
    /// Foreground time counted against the plan today.
    #[specta(type = specta_typescript::Number)]
    pub used_secs: u64,
    #[specta(type = specta_typescript::Number)]
    pub limit_secs: Option<u64>,
    pub sessions_used: u32,
    pub sessions_allowed: Option<u32>,
    /// When a screen-break reminder is next due.
    pub break_due_at: Option<DateTime<Utc>>,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Enabled but outside its schedule, with no quick timer.
    Inactive,
    /// Armed and (for blocking modes) enforcing.
    Active,
    /// Temporarily postponed by a pause.
    Paused,
    /// Allowance or session cap reached; the plan is blocking.
    LimitReached,
    /// A goal-only plan whose target was reached.
    GoalReached,
    /// A break-only plan, idle between reminders.
    Monitoring,
}

/// Whether a target belongs to a list's rules, ignoring schedule and
/// protection. Used to attribute activity to a plan's own targets; an
/// excepted domain is not the plan's business and does not spend its budget.
pub fn target_in_list(list: &BlockList, source: ActivitySource, target: &str) -> bool {
    match source {
        ActivitySource::BrowserTab => {
            let domain = crate::host::canonical_host(target);
            !list.is_excepted_domain(&domain)
                && list
                    .websites
                    .iter()
                    .any(|r| r.enabled && r.matches_domain(&domain))
        }
        ActivitySource::ForegroundApp => list.applications.iter().any(|r| {
            r.enabled
                && r.matches_process(target, None, None)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BlockList, WebsiteMatchType, WebsiteRule};

    #[test]
    fn default_plan_is_a_plain_block() {
        let p = Plan::default();
        assert_eq!(p.mode, PlanMode::Block);
        assert!(!p.quick_timer_active(Utc::now()));
        assert_eq!(p.daily_limit_secs(), None);
    }

    #[test]
    fn each_counting_mode_has_a_usable_default_and_validates() {
        for mode in [
            PlanMode::Block,
            PlanMode::TimeLimit,
            PlanMode::SessionLimit,
            PlanMode::ScreenBreak,
            PlanMode::TimeGoal,
        ] {
            let plan = Plan::with_mode(mode);
            assert!(plan.validate().is_ok(), "{mode:?} default should validate");
            assert_eq!(plan.normalized().mode, mode);
        }
    }

    #[test]
    fn a_time_limit_without_a_minute_count_is_rejected() {
        let plan = Plan {
            mode: PlanMode::TimeLimit,
            time_limit: None,
            ..Plan::default()
        };
        assert!(plan.validate().is_err());
    }

    #[test]
    fn switching_mode_drops_other_modes_configuration() {
        let plan = Plan {
            mode: PlanMode::SessionLimit,
            time_limit: Some(TimeLimit { daily_minutes: 30 }),
            ..Plan::with_mode(PlanMode::SessionLimit)
        }
        .normalized();
        assert!(plan.session_limit.is_some());
        assert!(plan.time_limit.is_none());
    }

    #[test]
    fn an_expired_quick_timer_is_dropped_on_normalization() {
        let plan = Plan {
            quick_timer_ends_at: Some(Utc::now() - chrono::Duration::minutes(1)),
            ..Plan::default()
        }
        .normalized();
        assert!(plan.quick_timer_ends_at.is_none());
    }

    #[test]
    fn target_matching_is_scoped_to_the_lists_own_rules() {
        let mut list = BlockList::new("work");
        list.websites.push(WebsiteRule {
            id: crate::types::new_id(),
            match_type: WebsiteMatchType::Domain("youtube.com".into()),
            enabled: true,
        });
        assert!(target_in_list(
            &list,
            ActivitySource::BrowserTab,
            "www.youtube.com"
        ));
        assert!(!target_in_list(
            &list,
            ActivitySource::BrowserTab,
            "example.com"
        ));
    }
}
