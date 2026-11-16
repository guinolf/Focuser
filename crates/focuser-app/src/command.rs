//! The command surface: every action the application can perform.
//!
//! Adding a variant here is the *only* way to add an operation. Both the Tauri
//! shim layer and the CLI are generated from / dispatch on this enum, so an
//! operation cannot exist in one frontend and not the other.
//!
//! Ported incrementally from `focuser-ui/src/commands.rs` — see
//! `internal-docs/tasks/2026-07-26-ui-revamp/02-command-core.md` for the order.

use chrono::NaiveDate;
use focuser_common::allowance::{Allowance, AllowanceMatch, AllowanceStatus};
use focuser_common::pomodoro::{PomodoroConfig, PomodoroPreset, PomodoroSession, PomodoroStatus};
use focuser_common::types::{
    AppMatchType, AppRule, BlockList, BlockedEvent, EntityId, ExceptionRule, ExceptionType,
    TimeSlot, UsageStat, WebsiteMatchType, WebsiteRule,
};
use serde::{Deserialize, Serialize};
use specta::Type;

/// How to lock a protection window, as supplied by a caller.
///
/// Distinct from [`focuser_common::types::Lock`]: that type stores what
/// protection actually persists (a password *hash*, never the plaintext),
/// while this is the one-shot wire input `EnableProtection` hashes on the
/// way in.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LockSetup {
    /// Require this password, typed back, to end the window early.
    Password { password: String },
    /// Require a freshly generated random string of this length, typed
    /// back exactly, to end the window early.
    RandomText { length: u32 },
}

/// A website rule kind *without* its value.
///
/// Needed by bulk import, which supplies one kind and many values. Distinct from
/// [`WebsiteMatchType`], which always carries its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WebsiteRuleKind {
    Domain,
    Keyword,
    Wildcard,
    UrlPath,
}

impl WebsiteRuleKind {
    /// Build a rule of this kind from a value.
    pub fn rule(self, value: &str) -> WebsiteRule {
        match self {
            Self::Domain => WebsiteRule::domain(value),
            Self::Keyword => WebsiteRule::keyword(value),
            Self::Wildcard => WebsiteRule::wildcard(value),
            Self::UrlPath => WebsiteRule::url_path(value),
        }
    }
}

/// One application action.
///
/// Adjacently tagged so the wire form is `{"cmd": "...", "args": {...}}` — stable,
/// self-describing, and a clean discriminated union in TypeScript.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "cmd", content = "args", rename_all = "snake_case")]
pub enum Command {
    // ─── Block lists ──────────────────────────────────────────────
    /// All block lists, with their rules.
    ListBlockLists,
    /// Create an empty block list. Returns the created list.
    CreateBlockList {
        name: String,
    },
    /// Replace a block list wholesale.
    ///
    /// Takes a real [`BlockList`], not the JSON string the old
    /// `update_block_list(list_json: String)` accepted.
    UpdateBlockList {
        list: Box<BlockList>,
    },
    /// Delete a block list and re-sync the hosts file.
    DeleteBlockList {
        id: EntityId,
    },
    /// Enable or disable a block list.
    ToggleBlockList {
        id: EntityId,
        enabled: bool,
    },

    // ─── Website rules ────────────────────────────────────────────
    /// Add a website rule. The match type carries its own value, so there is no
    /// separate stringly-typed `rule_type` to get wrong.
    AddWebsiteRule {
        list_id: EntityId,
        rule: WebsiteMatchType,
    },
    /// Remove a website rule. Errors if the rule is not in the list.
    RemoveWebsiteRule {
        list_id: EntityId,
        rule_id: EntityId,
    },
    /// Add many website rules of one kind at once, skipping blanks, comment
    /// lines, and values already present. Returns how many were added.
    BulkImportWebsites {
        list_id: EntityId,
        values: Vec<String>,
        kind: WebsiteRuleKind,
    },
    /// Remove every website rule from every unprotected list.
    ClearAllWebsites,

    // ─── Application rules ────────────────────────────────────────
    AddAppRule {
        list_id: EntityId,
        rule: AppMatchType,
    },
    /// Remove an app rule. Errors if the rule is not in the list.
    RemoveAppRule {
        list_id: EntityId,
        rule_id: EntityId,
    },
    /// Remove every app rule from every unprotected list.
    ClearAllApps,

    // ─── Exceptions ───────────────────────────────────────────────
    AddException {
        list_id: EntityId,
        exception: ExceptionType,
    },
    /// Remove an exception. Errors if it is not in the list.
    RemoveException {
        list_id: EntityId,
        exception_id: EntityId,
    },

    // ─── Schedule ─────────────────────────────────────────────────
    /// Replace a list's schedule.
    ///
    /// `always_active` clears the schedule entirely, meaning the list blocks at
    /// all times. Slots are real [`TimeSlot`]s — the old command took
    /// `Vec<serde_json::Value>` and hand-parsed `"Mon"`-style day strings,
    /// silently dropping any slot it failed to recognise.
    UpdateSchedule {
        list_id: EntityId,
        slots: Vec<TimeSlot>,
        always_active: bool,
    },

    // ─── Statistics ───────────────────────────────────────────────
    GetStats {
        from: NaiveDate,
        to: NaiveDate,
    },
    GetBlockedEvents {
        from: NaiveDate,
        to: NaiveDate,
    },
    /// Delete all statistics and blocked events. Block lists are preserved.
    ClearStatistics,
    GetStatsRetention,
    /// Set the retention window and immediately prune anything older.
    /// Returns the number of rows deleted.
    SetStatsRetention {
        days: u32,
    },

    // ─── Activity ─────────────────────────────────────────────────
    /// Measured foreground/interaction/idle time, with an explicit reason when
    /// there is nothing to show instead of a misleading zero.
    GetActivityOverview {
        from: NaiveDate,
        to: NaiveDate,
    },
    GetActivityHealth,
    /// Delete every activity sample and visit. Block lists are preserved.
    ClearActivity,
    GetActivityRetention,
    SetActivityRetention {
        days: u32,
    },

    // ─── Plans ────────────────────────────────────────────────────
    /// Attach (or replace) a plan on a list. Refused while the list's lock
    /// would be loosened by the change.
    SetPlan {
        list_id: EntityId,
        plan: Option<focuser_common::plan::Plan>,
    },
    GetPlanRuntimes,
    StartPlanPause {
        list_id: EntityId,
    },
    EndPlanPause {
        list_id: EntityId,
    },
    /// Arm a list outside its schedule for a bounded window.
    StartQuickTimer {
        list_id: EntityId,
        minutes: u32,
    },
    CancelQuickTimer {
        list_id: EntityId,
    },

    // ─── Protection ───────────────────────────────────────────────
    GetProtectionPolicy {
        list_id: EntityId,
    },
    /// Change a list's protection policy. While the list is protected the
    /// change must be a tightening; loosening needs the list unlocked.
    SetProtectionPolicy {
        list_id: EntityId,
        policy: focuser_common::protection::ProtectionPolicy,
    },
    /// Current wait/attempt state for a protected list.
    GetUnlockStatus {
        list_id: EntityId,
    },
    EnableProtection {
        list_id: EntityId,
        /// `None` locks until unlocked, with no timer. Refused without a
        /// `lock`, since that would leave no way out at all.
        duration_minutes: Option<u32>,
        prevent_uninstall: bool,
        prevent_service_stop: bool,
        prevent_modification: bool,
        /// How to end the window early. `None` means "wait it out" — the
        /// pure-commitment mode where nothing can end it before it expires.
        lock: Option<LockSetup>,
        /// Named profile whose wait and guards seed the policy. Omitted by
        /// older clients, which get the default (Firm).
        #[serde(default)]
        preset: Option<focuser_common::protection::ProtectionPreset>,
    },
    ConfigureScheduledProtection {
        list_id: EntityId,
        enabled: bool,
        lock: Option<LockSetup>,
    },
    RelockScheduledProtection {
        list_id: EntityId,
    },
    GetScheduledProtectionStatus,
    ConfigureSharedAllowance {
        list_id: EntityId,
        minutes: Option<u32>,
    },
    GetSharedAllowanceStatus,
    GetProtectionStatus,
    /// Issue a fresh random-text challenge for a protected list. Only valid
    /// on a list whose lock is [`focuser_common::types::Lock::RandomText`].
    /// Returns the string to display and retype.
    RequestUnlockChallenge {
        list_id: EntityId,
    },
    /// Answer a protected list's lock and, if correct, end its protection
    /// window immediately. `response` is the password, or the most recent
    /// random-text challenge typed back.
    UnlockProtection {
        list_id: EntityId,
        response: String,
    },

    // ─── Goals, tasks, notifications ──────────────────────────────
    GetGoals {
        include_archived: bool,
    },
    /// Active goals with the value measured for the current period.
    GetGoalProgress,
    /// Create or update a goal by id.
    SaveGoal {
        goal: focuser_common::productivity::Goal,
    },
    DeleteGoal {
        id: EntityId,
    },
    GetTasks {
        goal_id: Option<EntityId>,
        include_done: bool,
    },
    /// Create or update a task by id.
    SaveTask {
        task: focuser_common::productivity::Task,
    },
    DeleteTask {
        id: EntityId,
    },
    GetNotifications {
        limit: u32,
        unread_only: bool,
    },
    MarkNotificationRead {
        #[specta(type = specta_typescript::Number)]
        id: i64,
    },
    MarkAllNotificationsRead,
    ClearNotifications,
    /// Deterministic local insights derived from measured activity.
    GetCoachInsights,
    StartFocusSession {
        label: String,
        goal_id: Option<EntityId>,
        task_id: Option<EntityId>,
    },
    FinishFocusSession {
        id: EntityId,
    },
    GetFocusSessions {
        from: NaiveDate,
        to: NaiveDate,
    },

    // ─── Settings ─────────────────────────────────────────────────
    GetSetting {
        key: String,
        default: Option<String>,
    },
    SetSetting {
        key: String,
        value: String,
    },
    /// Reset settings to defaults. Block lists and statistics are preserved.
    ResetSettings,

    // ─── Enforcement ──────────────────────────────────────────────
    /// Whether blocking is actually in force right now, and why not if it isn't.
    GetBlockingHealth,
    /// Push the current blocked-domain set to the hosts file now.
    ApplyBlocks,
    /// Remove Focuser's hosts-file entries.
    RemoveBlocks,

    // ─── Pomodoro ─────────────────────────────────────────────────
    /// Current session, or `None` when nothing is running.
    PomodoroStatus,
    /// The built-in duration presets, in display order.
    PomodoroPresets,
    PomodoroStart {
        block_list_id: EntityId,
        config: PomodoroConfig,
        /// Lock the session for this many work rounds: until they are done
        /// it cannot be stopped, skipped or paused (#47). None is no lock.
        lock_rounds: Option<u32>,
    },
    /// Returns whether a session was actually paused.
    PomodoroPause,
    PomodoroResume,
    /// Returns whether a phase actually advanced.
    PomodoroSkip,
    /// Returns whether a session was actually stopped.
    PomodoroStop,
    /// Take and clear buffered phase-change / tamper events.
    PomodoroDrainEvents,
    PomodoroHistory {
        days: u32,
    },

    // ─── Allowances ───────────────────────────────────────────────
    AllowanceList,
    /// The target carries its own kind, so there is no separate `kind: String`.
    AllowanceCreate {
        target: AllowanceMatch,
        daily_limit_secs: u32,
        strict_mode: bool,
    },
    AllowanceUpdate {
        id: EntityId,
        daily_limit_secs: u32,
        strict_mode: bool,
        enabled: bool,
    },
    AllowanceDelete {
        id: EntityId,
    },
    /// Zero today's usage for one allowance.
    AllowanceResetToday {
        id: EntityId,
    },
    AllowanceDrainNotifications,
    AllowanceHistory {
        id: EntityId,
        days: u32,
    },

    // ─── Whole-configuration and diagnostics ──────────────────
    /// Serialise every block list to a portable JSON document.
    ///
    /// Returns the text rather than writing a file — where it goes is the
    /// frontend's business (a save dialog in the GUI, a path in the CLI).
    ExportConfiguration,
    /// Replace every block list with the contents of an exported document.
    /// Statistics and settings are untouched.
    ImportConfiguration {
        json: String,
    },
    /// Delete block lists, rules, schedules, statistics and settings.
    DeleteAllData,
    /// Would this domain be blocked right now?
    CheckDomain {
        domain: String,
    },
    /// Which known browsers are running, and which have the extension.
    GetBrowserStatus,
    /// Icons for application rules, read from the executables on disk.
    ///
    /// Batched because the caller wants a whole list at once and these are
    /// carried inline; one request per row would be one round trip per row.
    GetAppIcons {
        targets: Vec<String>,
    },
    AppVersion,
}

/// A buffered Pomodoro event, in wire form.
///
/// `focuser_app::PomodoroEvent` is the internal type and has no `specta::Type`;
/// this is its serialisable counterpart.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PomodoroEventDto {
    PhaseAdvanced { to: String, cycle: u32 },
    TamperDetected,
}

/// One completed Pomodoro session, for the statistics page.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PomodoroHistoryEntry {
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_cycles: u32,
    pub total_work_secs: u32,
}

/// One day's usage against an allowance.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AllowanceUsageEntry {
    pub date: String,
    pub used_secs: u32,
}

/// An allowance crossing a threshold. Mirrors `focuser_core`'s internal type,
/// which is not `specta::Type`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AllowanceNotificationDto {
    pub allowance_id: String,
    pub target: String,
    pub kind: String,
    pub used_secs: u32,
    pub limit_secs: u32,
}

/// Whether blocking is actually taking effect, for the UI to warn about.
///
/// Focuser blocks websites two ways: the browser extension, and the OS hosts
/// file. The hosts path needs administrator or root, and when the write fails
/// there is nothing on screen to say so — the user sees rules that look armed
/// and sites that still load. This is what makes that visible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BlockingHealth {
    /// Block lists that should be blocking at this moment.
    pub active_lists: u32,
    /// An extension has checked in recently, so it is enforcing rules.
    pub extension_connected: bool,
    /// The hosts file can be written — i.e. we have the privileges for it.
    pub hosts_writable: bool,
    /// A keyword, wildcard or URL-path rule is active. Only the extension can
    /// enforce those; a hosts file has no way to express them.
    pub extension_only_rules: bool,
    /// This session can report which app is in front, so app allowances can
    /// count down. False on Wayland, which has no such protocol.
    pub app_usage_measurable: bool,
}

impl BlockingHealth {
    /// True when the user expects blocking but neither mechanism can deliver,
    /// or when extension-only rules (keywords, wildcards, URL paths) are active
    /// without a connected extension.
    pub fn is_failing(&self) -> bool {
        self.active_lists > 0
            && ((!self.extension_connected && !self.hosts_writable)
                || (self.extension_only_rules && !self.extension_connected))
    }

    /// True when neither mechanism can deliver basic domain blocking.
    pub fn is_total_failure(&self) -> bool {
        self.active_lists > 0 && !self.extension_connected && !self.hosts_writable
    }

    /// True when rules that require the browser extension cannot be enforced.
    pub fn is_partial_failure(&self) -> bool {
        self.active_lists > 0 && self.extension_only_rules && !self.extension_connected
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ScheduledProtectionStatus {
    pub block_list_id: EntityId,
    pub state: focuser_common::types::ScheduledLockState,
}

/// An active protection window on a block list.
///
/// Replaces the ad-hoc `serde_json::json!` object the old command built.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProtectionInfo {
    pub block_list_id: EntityId,
    pub block_list_name: String,
    pub prevent_uninstall: bool,
    pub prevent_service_stop: bool,
    pub prevent_modification: bool,
    /// Exported as a TS `number`; see the note on [`UsageStat`] — seconds can
    /// never approach the 2^53 precision ceiling.
    ///
    /// `None` while the protection runs until unlocked.
    #[specta(type = Option<specta_typescript::Number>)]
    pub remaining_seconds: Option<u64>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The result of a successful [`Command`].
///
/// Deliberately typed — the old command layer returned `serde_json::Value` from
/// 24 of its 55 commands, which meant the frontend hand-decoded shapes that the
/// compiler never checked.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CommandResult {
    /// Succeeded, nothing to return.
    Unit,
    BlockList(Box<BlockList>),
    BlockLists(Vec<BlockList>),
    WebsiteRule(Box<WebsiteRule>),
    AppRule(Box<AppRule>),
    Exception(Box<ExceptionRule>),
    /// A number of affected items — e.g. rules imported or cleared.
    Count(u32),
    Stats(Vec<UsageStat>),
    BlockedEvents(Vec<BlockedEvent>),
    ProtectionStatus(Vec<ProtectionInfo>),
    ScheduledProtectionStatus(Vec<ScheduledProtectionStatus>),
    SharedAllowanceStatus(Vec<focuser_common::allowance::SharedAllowanceStatus>),
    BlockingHealth(BlockingHealth),
    Activity(Box<focuser_common::activity::ActivityOverview>),
    ActivityHealth(focuser_common::activity::ActivityHealth),
    PlanRuntimes(Vec<focuser_common::plan::PlanRuntime>),
    ProtectionPolicy(focuser_common::protection::ProtectionPolicy),
    UnlockStatus(focuser_common::protection::UnlockStatus),
    UnlockChallenge(UnlockChallenge),
    Goals(Vec<focuser_common::productivity::Goal>),
    GoalProgress(Vec<focuser_common::productivity::GoalProgress>),
    Goal(focuser_common::productivity::Goal),
    Tasks(Vec<focuser_common::productivity::Task>),
    Task(focuser_common::productivity::Task),
    Notifications(Vec<focuser_common::productivity::Notification>),
    CoachInsights(Vec<focuser_common::productivity::CoachInsight>),
    FocusSessions(Vec<focuser_common::productivity::FocusSession>),
    FocusSession(Box<focuser_common::productivity::FocusSession>),
    /// A setting value; `None` when unset and no default was supplied.
    Setting(Option<String>),
    /// A yes/no outcome — e.g. "was a session actually paused".
    Flag(bool),
    /// Current Pomodoro session, or `None` when idle.
    PomodoroStatus(Option<PomodoroStatus>),
    PomodoroSession(Box<PomodoroSession>),
    PomodoroEvents(Vec<PomodoroEventDto>),
    PomodoroHistory(Vec<PomodoroHistoryEntry>),
    PomodoroPresets(Vec<PomodoroPreset>),
    Allowance(Box<Allowance>),
    Allowances(Vec<AllowanceStatus>),
    AllowanceNotifications(Vec<AllowanceNotificationDto>),
    AllowanceHistory(Vec<AllowanceUsageEntry>),
    /// Free text — an exported configuration document, or a version string.
    Text(String),
    BrowserStatus(Vec<BrowserStatus>),
    AppIcons(Vec<AppIcon>),
}

/// A challenge issued together with the moment it becomes usable.
///
/// The wait is part of the value, not a UI timer: closing the window, killing
/// the process or changing the clock does not shorten it, because the server
/// stores `ready_at` and checks it again when the answer arrives.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UnlockChallenge {
    pub block_list_id: EntityId,
    /// The exact text to retype. Deliberately not stored anywhere else.
    pub challenge: String,
    pub ready_at: chrono::DateTime<chrono::Utc>,
    /// Seconds until `ready_at`, at the moment of issue.
    pub wait_secs: u32,
}

/// One known browser and whether Focuser can see it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BrowserStatus {
    pub browser: String,
    pub display_name: String,
    pub running: bool,
    pub extension_connected: bool,
    /// Whether the extension can see private windows. Only meaningful when
    /// `extension_connected` is true. An extension too old to report it
    /// counts as allowed, as it did before this was checked.
    ///
    /// `extension_connected && !incognito_allowed` is the real gap: the
    /// extension is present and working, but a private window in this
    /// browser has nothing blocking it.
    pub incognito_allowed: bool,
    /// Where to install the extension for this browser.
    pub store_url: String,
    /// Short name for launching this browser at a URL.
    pub launch_name: String,
}

/// One application rule's icon.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppIcon {
    /// Echoed back so the caller can match icons to rows without relying on
    /// the order it sent them in.
    pub target: String,
    /// A `data:image/png;base64,…` URI, or `None` when the rule does not name
    /// a program we can find — a window title, or software since uninstalled.
    pub data_uri: Option<String>,
}

impl CommandResult {
    /// Convenience for tests and callers that know the shape they expect.
    pub fn as_block_lists(&self) -> Option<&[BlockList]> {
        match self {
            Self::BlockLists(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_block_list(&self) -> Option<&BlockList> {
        match self {
            Self::BlockList(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_count(&self) -> Option<u32> {
        match self {
            Self::Count(n) => Some(*n),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_round_trips_through_json() {
        let cmd = Command::ToggleBlockList {
            id: EntityId::nil(),
            enabled: true,
        };
        let json = serde_json::to_string(&cmd).unwrap();

        assert!(
            json.contains(r#""cmd":"toggle_block_list""#),
            "expected adjacent tagging, got {json}"
        );

        let back: Command = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            Command::ToggleBlockList { enabled: true, .. }
        ));
    }

    #[test]
    fn unit_variant_serialises_without_args() {
        let json = serde_json::to_string(&Command::ListBlockLists).unwrap();
        assert_eq!(json, r#"{"cmd":"list_block_lists"}"#);
    }
}

#[cfg(test)]
mod health_tests {
    use super::BlockingHealth;

    fn health(
        active_lists: u32,
        extension_connected: bool,
        hosts_writable: bool,
    ) -> BlockingHealth {
        BlockingHealth {
            active_lists,
            extension_connected,
            hosts_writable,
            extension_only_rules: false,
            app_usage_measurable: true,
        }
    }

    #[test]
    fn losing_both_mechanisms_with_active_lists_is_a_failure() {
        assert!(health(1, false, false).is_failing());
    }

    #[test]
    fn either_mechanism_alone_is_enough() {
        assert!(!health(1, true, false).is_failing());
        assert!(!health(1, false, true).is_failing());
    }

    #[test]
    fn nothing_is_wrong_when_nothing_is_meant_to_be_blocked() {
        // No active lists means no promise to keep, so a hosts file we cannot
        // write is not worth warning anyone about.
        assert!(!health(0, false, false).is_failing());
    }

    #[test]
    fn extension_only_rules_without_extension_is_a_failure_even_if_hosts_writable() {
        let mut h = health(1, false, true);
        h.extension_only_rules = true;
        assert!(h.is_failing());
        assert!(h.is_partial_failure());
        assert!(!h.is_total_failure());
    }

    #[test]
    fn extension_only_rules_with_extension_is_not_a_failure() {
        let mut h = health(1, true, false);
        h.extension_only_rules = true;
        assert!(!h.is_failing());
        assert!(!h.is_partial_failure());
    }
}
