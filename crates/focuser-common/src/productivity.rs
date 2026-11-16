//! Productivity layer: goals, tasks, focus sessions, notifications and the
//! deterministic coach.
//!
//! Everything here is local and explicit. A goal's progress is derived from
//! measurements that are already stored (activity samples, focus sessions,
//! completed tasks) or entered by the user — never estimated, never inferred
//! from keystrokes, never fetched from a service. The coach is a set of
//! documented heuristics over those same rows; there is no model, no account
//! and no network involved.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::types::{EntityId, new_id};

/// What a goal measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum GoalKind {
    /// Foreground time on the goal's linked plan targets.
    #[default]
    Time,
    /// Completed focus sessions linked to the goal.
    Sessions,
    /// Outcome: the fraction of linked tasks completed. Progress is real
    /// task state, but completion is always confirmed by the user.
    Result,
}

/// The window a goal resets on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum GoalPeriod {
    #[default]
    Day,
    Week,
    Month,
    Once,
}

impl GoalPeriod {
    /// Inclusive start date of the period containing `today`.
    pub fn start(self, today: NaiveDate) -> NaiveDate {
        use chrono::{Datelike, Duration};
        match self {
            Self::Day => today,
            Self::Week => {
                let from_monday = i64::from(today.weekday().num_days_from_monday());
                today - Duration::days(from_monday)
            }
            Self::Month => NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today),
            Self::Once => NaiveDate::MIN,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    Active,
    Completed,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Goal {
    pub id: EntityId,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub kind: GoalKind,
    pub period: GoalPeriod,
    /// Minutes for `Time`, session count for `Sessions`, task count for
    /// `Result`. Always an absolute number, never a percentage.
    pub target_value: u32,
    /// The plan whose targets supply `Time` progress, when set.
    #[serde(default)]
    pub block_list_id: Option<EntityId>,
    pub state: GoalState,
    #[serde(default)]
    pub deadline: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
}

impl Goal {
    pub fn new(title: impl Into<String>, kind: GoalKind, target_value: u32) -> Self {
        let now = Utc::now();
        Self {
            id: new_id(),
            title: title.into(),
            notes: String::new(),
            kind,
            period: GoalPeriod::Week,
            target_value,
            block_list_id: None,
            state: GoalState::Active,
            deadline: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("goal title must not be empty".into());
        }
        if self.target_value == 0 {
            return Err("goal target must be at least 1".into());
        }
        if self.kind == GoalKind::Time && self.target_value > 24 * 60 * 31 {
            return Err("goal target is implausibly large".into());
        }
        Ok(())
    }
}

/// A goal plus its measured progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct GoalProgress {
    pub goal: Goal,
    /// Units depend on `goal.kind`: seconds for Time, sessions for Sessions,
    /// completed tasks for Result.
    #[specta(type = specta_typescript::Number)]
    pub current: u64,
    #[specta(type = specta_typescript::Number)]
    pub target: u64,
    pub percent: u8,
    pub period_start: NaiveDate,
    /// True when the measured value reached the target. Result goals still
    /// need an explicit confirmation before they are marked completed.
    pub reached: bool,
}

impl GoalProgress {
    pub fn from_goal(goal: Goal, current: u64, period_start: NaiveDate) -> Self {
        // `current` is seconds for time goals, so the target must be seconds
        // too. Comparing minutes with seconds made every time goal look met.
        let target = match goal.kind {
            GoalKind::Time => u64::from(goal.target_value.max(1)) * 60,
            GoalKind::Sessions | GoalKind::Result => u64::from(goal.target_value.max(1)),
        };
        let percent = ((current.saturating_mul(100)) / target).min(100) as u8;
        Self {
            reached: current >= target,
            goal,
            current,
            target,
            percent,
            period_start,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Todo,
    Doing,
    Done,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ChecklistItem {
    pub id: EntityId,
    pub text: String,
    pub done: bool,
}

impl ChecklistItem {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            text: text.into(),
            done: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Task {
    pub id: EntityId,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub priority: TaskPriority,
    #[serde(default)]
    pub checklist: Vec<ChecklistItem>,
    #[serde(default)]
    pub goal_id: Option<EntityId>,
    #[serde(default)]
    pub due_date: Option<NaiveDate>,
    /// Minutes the user estimated; shown next to measured focus time.
    #[serde(default)]
    pub estimate_minutes: Option<u32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
}

impl Task {
    pub fn new(title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: new_id(),
            title: title.into(),
            notes: String::new(),
            status: TaskStatus::Todo,
            priority: TaskPriority::Medium,
            checklist: Vec::new(),
            goal_id: None,
            due_date: None,
            estimate_minutes: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("task title must not be empty".into());
        }
        if self.estimate_minutes.is_some_and(|m| m == 0 || m > 24 * 60) {
            return Err("estimate must be between 1 and 1440 minutes".into());
        }
        Ok(())
    }

    pub fn is_done(&self) -> bool {
        self.status == TaskStatus::Done
    }
}

/// A completed stretch of focused work, linked to a task or goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FocusSession {
    pub id: EntityId,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub planned_secs: u32,
    #[specta(type = specta_typescript::Number)]
    pub focused_secs: u64,
    #[serde(default)]
    pub task_id: Option<EntityId>,
    #[serde(default)]
    pub goal_id: Option<EntityId>,
    #[serde(default)]
    pub block_list_id: Option<EntityId>,
    /// `quick` for the timer, `pomodoro` for a Pomodoro work phase, `manual`
    /// for a session started by hand.
    pub source: String,
    /// What the user was focusing on; free text, shown in history.
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    BreakDue,
    QuickTimerEnded,
    GoalReached,
    GoalBehind,
    AllowanceWarning,
    AllowanceExhausted,
    BlockingHealth,
    CoachTip,
    UnlockReady,
}

impl NotificationKind {
    /// Whether this kind has a close-button/ignored-by-default place in the
    /// notification centre. Break reminders are the only kind allowed to be
    /// visually prominent; everything else stays quiet.
    pub fn is_reminder(self) -> bool {
        matches!(self, Self::BreakDue)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum NotificationPriority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Notification {
    #[specta(type = specta_typescript::Number)]
    pub id: i64,
    pub kind: NotificationKind,
    pub priority: NotificationPriority,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub read_at: Option<DateTime<Utc>>,
    /// Stable key used to avoid repeating the same reminder over and over.
    #[serde(default)]
    pub dedupe_key: Option<String>,
}

/// Preferences, stored as settings under one JSON key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct NotificationPrefs {
    /// Break reminders are opt-in at the OS level because they interrupt.
    pub breaks: bool,
    pub goals: bool,
    pub allowances: bool,
    pub coach: bool,
    /// Quiet hours, local time, inclusive start / exclusive end.
    pub quiet_start_hour: u8,
    pub quiet_end_hour: u8,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            breaks: true,
            goals: true,
            allowances: true,
            coach: true,
            quiet_start_hour: 22,
            quiet_end_hour: 7,
        }
    }
}

impl NotificationPrefs {
    pub fn allows(self, kind: NotificationKind) -> bool {
        match kind {
            NotificationKind::BreakDue => self.breaks,
            NotificationKind::GoalReached | NotificationKind::GoalBehind => self.goals,
            NotificationKind::AllowanceWarning | NotificationKind::AllowanceExhausted => {
                self.allowances
            }
            NotificationKind::CoachTip => self.coach,
            _ => true,
        }
    }

    pub fn in_quiet_hours(self, local_hour: u8) -> bool {
        if self.quiet_start_hour == self.quiet_end_hour {
            return false;
        }
        if self.quiet_start_hour < self.quiet_end_hour {
            local_hour >= self.quiet_start_hour && local_hour < self.quiet_end_hour
        } else {
            local_hour >= self.quiet_start_hour || local_hour < self.quiet_end_hour
        }
    }
}

/// A deterministic observation. `evidence` is shown verbatim so the user can
/// judge the claim instead of trusting a black box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CoachInsight {
    pub id: String,
    pub title: String,
    pub body: String,
    pub evidence: Vec<String>,
    pub severity: NotificationPriority,
    /// A concrete command the UI can offer, when one applies.
    #[serde(default)]
    pub action: Option<CoachAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoachAction {
    CreateAllowance { hostname: String, minutes: u32 },
    AddWebsiteRule { block_list_id: EntityId, hostname: String },
    OpenPlans,
    AdjustScreenBreak { work_minutes: u32, break_minutes: u32 },
    ArchiveGoal { goal_id: EntityId },
}

/// A focus session in progress, before it is written to history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ActiveFocusSession {
    pub session: FocusSession,
    #[specta(type = specta_typescript::Number)]
    pub remaining_secs: u64,
    #[specta(type = specta_typescript::Number)]
    pub focused_secs: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_progress_is_clamped_and_reported_exactly() {
        let goal = Goal::new("Read", GoalKind::Time, 60);
        // A 60-minute goal is measured in seconds, matching activity data.
        let p = GoalProgress::from_goal(goal.clone(), 1_800, NaiveDate::from_ymd_opt(2026, 11, 15).unwrap());
        assert_eq!(p.percent, 50);
        assert!(!p.reached);

        let p = GoalProgress::from_goal(goal, 3_600, NaiveDate::from_ymd_opt(2026, 11, 15).unwrap());
        assert_eq!(p.percent, 100);
        assert!(p.reached);
    }

    #[test]
    fn time_goal_targets_are_converted_to_the_measured_seconds() {
        let mut goal = Goal::new("deep work", GoalKind::Time, 30);
        goal.period = GoalPeriod::Day;
        let progress = GoalProgress::from_goal(goal, 600, NaiveDate::from_ymd_opt(2026, 11, 15).unwrap());
        assert_eq!(progress.target, 1_800);
        assert_eq!(progress.percent, 33);
        assert!(!progress.reached);
    }

    #[test]
    fn week_periods_start_on_monday() {
        // 2026-11-15 is a Sunday.
        let sunday = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
        assert_eq!(
            GoalPeriod::Week.start(sunday),
            NaiveDate::from_ymd_opt(2026, 11, 9).unwrap()
        );
    }

    #[test]
    fn quiet_hours_handle_windows_that_cross_midnight() {
        let prefs = NotificationPrefs {
            quiet_start_hour: 22,
            quiet_end_hour: 7,
            ..NotificationPrefs::default()
        };
        assert!(prefs.in_quiet_hours(23));
        assert!(prefs.in_quiet_hours(3));
        assert!(!prefs.in_quiet_hours(12));
    }

    #[test]
    fn task_and_goal_validation_reject_empty_values() {
        assert!(Task::new("  ").validate().is_err());
        assert!(Task::new("Write report").validate().is_ok());
        let mut goal = Goal::new("Focus", GoalKind::Sessions, 5);
        assert!(goal.validate().is_ok());
        goal.target_value = 0;
        assert!(goal.validate().is_err());
    }

    #[test]
    fn only_break_reminders_are_marked_as_reminders() {
        assert!(NotificationKind::BreakDue.is_reminder());
        assert!(!NotificationKind::GoalReached.is_reminder());
    }
}
