//! Protection presets, unlock protocol and the tightening-only rule.
//!
//! The old protection model had one lever: on or off. Every commitment felt the
//! same, and editing a protected list was refused wholesale — which meant a
//! user could not even add a site while locked, and the only way through was
//! the full emergency exit. This module adds two things:
//!
//! * **Presets** with explicit numbers, so "Firme" means something specific
//!   (a 20-character challenge, a mandatory ten-minute wait, all three
//!   prevention flags) rather than a vibe.
//! * **A change classification** used to allow *tightening only* while a list
//!   is locked: adding a rule, removing an exception or shortening a limit is
//!   always safe to permit, because it cannot make the commitment easier to
//!   escape. Anything that loosens or is ambiguous is refused.
//!
//! The unlock wait is stored in the database, not in memory, so restarting the
//! app, killing it, or changing the clock does not shorten it. A clock that
//! moves backwards *restarts* the wait (see `UnlockStatus::from_row`), because
//! the alternative is trusting a timestamp the user controls.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::collections::BTreeSet;

use crate::types::BlockList;

/// Named profiles shown in the UI. Numbers are part of the contract and are
/// asserted in tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionPreset {
    /// A short pause. Still requires the challenge; every preset is raised to
    /// the ten-minute floor by [`ProtectionPolicy::sanitized`].
    Light,
    Moderate,
    #[default]
    Firm,
    /// No early exit at all beyond waiting out the full hour-long delay.
    Closed,
}

impl ProtectionPreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Moderate => "moderate",
            Self::Firm => "firm",
            Self::Closed => "closed",
        }
    }

    pub fn wait_secs(self) -> u32 {
        match self {
            Self::Light => 60,
            Self::Moderate => 300,
            Self::Firm => 600,
            Self::Closed => 3600,
        }
    }

    pub fn prevent_uninstall(self) -> bool {
        matches!(self, Self::Firm | Self::Closed)
    }

    pub fn prevent_service_stop(self) -> bool {
        matches!(self, Self::Moderate | Self::Firm | Self::Closed)
    }

    pub fn prevent_modification(self) -> bool {
        matches!(self, Self::Moderate | Self::Firm | Self::Closed)
    }

    pub fn all() -> [Self; 4] {
        [Self::Light, Self::Moderate, Self::Firm, Self::Closed]
    }
}

/// The single source of truth for how a commitment can be ended.
///
/// `challenge_len` is fixed at 20 for every preset: a shorter string is easier
/// to paste back without reading, which defeats the friction the challenge
/// exists to create.
pub const CHALLENGE_LEN: u32 = 20;

/// No preset may release sooner than this. The mission's contract is a
/// ten-minute floor for a deliberate exit; a preset that wants longer may set
/// it, a caller may not set less.
pub const MIN_WAIT_SECS: u32 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProtectionPolicy {
    pub preset: ProtectionPreset,
    pub prevent_uninstall: bool,
    pub prevent_service_stop: bool,
    pub prevent_modification: bool,
    pub wait_secs: u32,
    pub challenge_len: u32,
    pub max_attempts_per_hour: u32,
}

impl ProtectionPolicy {
    pub fn from_preset(preset: ProtectionPreset) -> Self {
        Self {
            preset,
            prevent_uninstall: preset.prevent_uninstall(),
            prevent_service_stop: preset.prevent_service_stop(),
            prevent_modification: preset.prevent_modification(),
            wait_secs: preset.wait_secs().max(MIN_WAIT_SECS),
            challenge_len: CHALLENGE_LEN,
            max_attempts_per_hour: 5,
        }
    }

    /// Validate a policy that came from storage or an import. A policy with a
    /// wait under the floor is raised to it rather than rejected, so a
    /// hand-edited database cannot shorten a commitment.
    pub fn sanitized(mut self) -> Self {
        self.wait_secs = self.wait_secs.max(MIN_WAIT_SECS);
        self.challenge_len = CHALLENGE_LEN;
        self.max_attempts_per_hour = self.max_attempts_per_hour.clamp(1, 20);
        self
    }
}

impl Default for ProtectionPolicy {
    fn default() -> Self {
        Self::from_preset(ProtectionPreset::Firm)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum UnlockState {
    /// Nothing asked for.
    Idle,
    /// A request is waiting out its delay. The challenge can be answered
    /// during the wait, but nothing unlocks before `ready_at`.
    Waiting,
    /// The wait elapsed; the unlock can be confirmed.
    Ready,
    /// Cancelled by the user before it became ready.
    Cancelled,
}

/// What the protection panel shows for one list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct UnlockStatus {
    pub block_list_id: String,
    pub state: UnlockState,
    #[serde(default)]
    pub requested_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub ready_at: Option<DateTime<Utc>>,
    #[specta(type = specta_typescript::Number)]
    pub remaining_secs: u64,
    pub attempts_remaining: u32,
    #[serde(default)]
    pub locked_out_until: Option<DateTime<Utc>>,
    pub challenge_len: u32,
}

impl UnlockStatus {
    pub fn idle(block_list_id: String, policy: ProtectionPolicy) -> Self {
        Self {
            block_list_id,
            state: UnlockState::Idle,
            requested_at: None,
            ready_at: None,
            remaining_secs: 0,
            attempts_remaining: policy.max_attempts_per_hour,
            locked_out_until: None,
            challenge_len: policy.challenge_len,
        }
    }
}

/// Outcome of answering a challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct UnlockOutcome {
    pub status: UnlockStatus,
    /// True only when protection was actually removed (legacy immediate
    /// policies, or a confirm after the wait).
    pub unlocked: bool,
}

/// How a wholesale list edit would change the commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChangeClass {
    /// Cannot make the commitment easier to escape.
    Tightening,
    /// Makes it easier; refused while the list is protected.
    Loosening,
    /// Neither direction with confidence, e.g. a rule swapped for another.
    Ambiguous,
    /// Touch only fields that do not affect enforcement.
    Neutral,
}

impl ChangeClass {
    pub fn may_proceed(self) -> bool {
        matches!(self, Self::Tightening | Self::Neutral)
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Tightening => "tightens the commitment",
            Self::Loosening => "loosens the commitment",
            Self::Ambiguous => "changes the commitment in a way that cannot be verified as tightening",
            Self::Neutral => "does not change enforcement",
        }
    }
}

/// Compare two versions of a list and say which direction the edit moves.
///
/// Deliberately conservative: when a change both adds and removes coverage, or
/// rewrites a rule in place, it is [`ChangeClass::Ambiguous`] and therefore
/// refused under a lock. Declaring it "tightening" on the strength of the one
/// favourable half is exactly the hole this rule exists to close.
pub fn classify_change(old: &BlockList, new: &BlockList) -> ChangeClass {
    let mut tightening = false;
    let mut loosening = false;

    if old.enabled != new.enabled {
        if new.enabled {
            tightening = true;
        } else {
            loosening = true;
        }
    }

    // Protection and locks. Any reduction is a loosening.
    if old.protection.is_some() != new.protection.is_some() {
        if new.protection.is_some() {
            tightening = true;
        } else {
            loosening = true;
        }
    }
    if old.scheduled_protection.is_some() != new.scheduled_protection.is_some() {
        if new.scheduled_protection.is_some() {
            tightening = true;
        } else {
            loosening = true;
        }
    }
    if old.lock.is_some() && new.lock.is_none() {
        loosening = true;
    }
    if old.lock.is_none() && new.lock.is_some() {
        tightening = true;
    }
    let old_flags = old
        .protection
        .as_ref()
        .map(|p| (p.prevent_uninstall, p.prevent_service_stop, p.prevent_modification));
    let new_flags = new
        .protection
        .as_ref()
        .map(|p| (p.prevent_uninstall, p.prevent_service_stop, p.prevent_modification));
    if let (Some(a), Some(b)) = (old_flags, new_flags) {
        if a != b {
            // Removing a prevention flag is a loosening; adding one tightens.
            let removed = (!b.0 && a.0) || (!b.1 && a.1) || (!b.2 && a.2);
            let added = (b.0 && !a.0) || (b.1 && !a.1) || (b.2 && !a.2);
            loosening |= removed;
            tightening |= added;
        }
    }

    // Rules. Keyed by identity plus match, so an in-place edit reads as a
    // remove plus an add and lands in Ambiguous, not Tightening.
    let old_sites = rule_keys(&old.websites);
    let new_sites = rule_keys(&new.websites);
    tightening |= !new_sites.difference(&old_sites).next().is_none();
    loosening |= !old_sites.difference(&new_sites).next().is_none();

    let old_apps = rule_keys(&old.applications);
    let new_apps = rule_keys(&new.applications);
    tightening |= !new_apps.difference(&old_apps).next().is_none();
    loosening |= !old_apps.difference(&new_apps).next().is_none();

    // Exceptions run the other way: removing one blocks more.
    let old_exc = rule_keys(&old.exceptions);
    let new_exc = rule_keys(&new.exceptions);
    loosening |= !new_exc.difference(&old_exc).next().is_none();
    tightening |= !old_exc.difference(&new_exc).next().is_none();

    // Schedule: dropping it blocks all day (tightening); adding hours
    // restricts blocking to them (loosening).
    match (&old.schedule, &new.schedule) {
        (None, Some(_)) => loosening = true,
        (Some(_), None) => tightening = true,
        (Some(a), Some(b)) => {
            let a = schedule_keys(a);
            let b = schedule_keys(b);
            tightening |= !b.difference(&a).next().is_none();
            loosening |= !a.difference(&b).next().is_none();
        }
        (None, None) => {}
    }

    if value_of(&old.shared_allowance) != value_of(&new.shared_allowance) {
        match (&old.shared_allowance, &new.shared_allowance) {
            (None, Some(_)) => loosening = true,
            (Some(_), None) => tightening = true,
            (Some(a), Some(b)) => {
                if b.minutes > a.minutes {
                    loosening = true;
                } else if b.minutes < a.minutes {
                    tightening = true;
                }
            }
            (None, None) => {}
        }
    }

    // Plan limits: a higher limit is more allowed time (loosening); a lower
    // one tightens. Mode changes are ambiguous by construction.
    match (&old.plan, &new.plan) {
        (None, Some(p)) if p.mode != crate::plan::PlanMode::Block => loosening = true,
        (Some(p), None) if p.mode != crate::plan::PlanMode::Block => loosening = true,
        (Some(a), Some(b)) => {
            if a.mode != b.mode {
                loosening = true;
                tightening = true;
            }
            let old_limit = a.daily_limit_secs();
            let new_limit = b.daily_limit_secs();
            match (old_limit, new_limit) {
                (None, Some(_)) => loosening = true,
                (Some(_), None) => tightening = true,
                (Some(x), Some(y)) if y > x => loosening = true,
                (Some(x), Some(y)) if y < x => tightening = true,
                _ => {}
            }
            let old_sessions = a.session_limit.map(|s| s.max_sessions);
            let new_sessions = b.session_limit.map(|s| s.max_sessions);
            match (old_sessions, new_sessions) {
                (None, Some(_)) => loosening = true,
                (Some(_), None) => tightening = true,
                (Some(x), Some(y)) if y > x => loosening = true,
                (Some(x), Some(y)) if y < x => tightening = true,
                _ => {}
            }
            // Starting a timer adds a commitment; clearing a running one removes it.
            match (a.quick_timer_ends_at, b.quick_timer_ends_at) {
                (None, Some(_)) => tightening = true,
                (Some(_), None) => loosening = true,
                (Some(x), Some(y)) if y > x => tightening = true,
                (Some(x), Some(y)) if y < x => loosening = true,
                _ => {}
            }
            if a.pauses != b.pauses {
                // A more permissive pause policy is a loosening.
                loosening = true;
            }
        }
        _ => {}
    }

    if value_of(&old.breaks) != value_of(&new.breaks) {
        // Break reminders are advisory; changing them neither blocks nor frees.
        return if tightening || loosening {
            ChangeClass::Ambiguous
        } else {
            ChangeClass::Neutral
        };
    }

    match (tightening, loosening) {
        (true, true) => ChangeClass::Ambiguous,
        (true, false) => ChangeClass::Tightening,
        (false, true) => ChangeClass::Loosening,
        (false, false) => ChangeClass::Neutral,
    }
}

fn value_of<T: Serialize>(value: &T) -> Option<Value> {
    serde_json::to_value(value).ok()
}

fn rule_keys<T: Serialize>(items: &[T]) -> BTreeSet<String> {
    items
        .iter()
        .filter_map(|item| serde_json::to_string(item).ok())
        .collect()
}

fn schedule_keys(schedule: &crate::types::Schedule) -> BTreeSet<String> {
    schedule
        .time_slots
        .iter()
        .filter_map(|slot| serde_json::to_value(slot).ok())
        .map(|v: Value| v.to_string())
        .collect()
}

/// Remaining wait, recomputed defensively.
///
/// `now` earlier than `requested_at` is a clock that moved backwards. The wait
/// then restarts from the new "now", so turning the clock back cannot skip it.
pub fn remaining_wait(
    requested_at: DateTime<Utc>,
    ready_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> u64 {
    let effective_ready = ready_at.max(requested_at + Duration::seconds(MIN_WAIT_SECS.into()));
    if now < requested_at {
        return (effective_ready - now).num_seconds().max(0) as u64;
    }
    (effective_ready - now).num_seconds().max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Plan, PlanMode, TimeLimit};
    use crate::types::{
        ExceptionRule, Protection, WebsiteMatchType, WebsiteRule, new_id,
    };

    fn list() -> BlockList {
        BlockList::new("work")
    }

    fn site(list: &mut BlockList, domain: &str) {
        list.websites.push(WebsiteRule {
            id: new_id(),
            match_type: WebsiteMatchType::Domain(domain.into()),
            enabled: true,
        });
    }

    #[test]
    fn presets_have_explicit_numbers_and_never_release_before_the_floor() {
        for preset in ProtectionPreset::all() {
            let policy = ProtectionPolicy::from_preset(preset);
            assert_eq!(policy.challenge_len, 20);
            assert!(policy.wait_secs >= MIN_WAIT_SECS, "{preset:?}");
        }
        assert_eq!(ProtectionPreset::Firm.wait_secs(), 600);
        assert_eq!(ProtectionPolicy::default().wait_secs, 600);
    }

    #[test]
    fn a_policy_from_storage_cannot_shorten_the_wait() {
        let handmade = ProtectionPolicy {
            wait_secs: 1,
            challenge_len: 4,
            ..ProtectionPolicy::from_preset(ProtectionPreset::Firm)
        }
        .sanitized();
        assert_eq!(handmade.wait_secs, MIN_WAIT_SECS);
        assert_eq!(handmade.challenge_len, CHALLENGE_LEN);
    }

    #[test]
    fn adding_a_rule_is_a_tightening() {
        let old = list();
        let mut new = old.clone();
        site(&mut new, "youtube.com");
        assert_eq!(classify_change(&old, &new), ChangeClass::Tightening);
        assert!(classify_change(&old, &new).may_proceed());
    }

    #[test]
    fn removing_a_rule_is_a_loosening() {
        let mut old = list();
        site(&mut old, "youtube.com");
        let new = list();
        assert_eq!(classify_change(&old, &new), ChangeClass::Loosening);
        assert!(!classify_change(&old, &new).may_proceed());
    }

    #[test]
    fn adding_an_exception_is_a_loosening_and_removing_one_tightens() {
        let mut old = list();
        site(&mut old, "youtube.com");
        let mut with_exception = old.clone();
        with_exception
            .exceptions
            .push(ExceptionRule::domain("youtube.com"));
        assert_eq!(
            classify_change(&old, &with_exception),
            ChangeClass::Loosening
        );
        assert_eq!(
            classify_change(&with_exception, &old),
            ChangeClass::Tightening
        );
    }

    #[test]
    fn swapping_one_rule_for_another_is_ambiguous_not_tightening() {
        let mut old = list();
        site(&mut old, "youtube.com");
        let mut new = list();
        site(&mut new, "reddit.com");
        assert_eq!(classify_change(&old, &new), ChangeClass::Ambiguous);
    }

    #[test]
    fn renaming_a_list_is_neutral() {
        let old = list();
        let mut new = old.clone();
        new.name = "still work".into();
        new.updated_at = Utc::now();
        assert_eq!(classify_change(&old, &new), ChangeClass::Neutral);
    }

    #[test]
    fn disabling_a_list_or_dropping_protection_is_a_loosening() {
        let old = BlockList {
            protection: Some(Protection {
                prevent_uninstall: true,
                prevent_service_stop: true,
                prevent_modification: true,
                started_at: Utc::now(),
                expires_at: Some(Utc::now() + Duration::hours(1)),
            }),
            ..list()
        };
        let mut new = old.clone();
        new.enabled = false;
        assert_eq!(classify_change(&old, &new), ChangeClass::Loosening);
        new.enabled = true;
        new.protection = None;
        assert_eq!(classify_change(&old, &new), ChangeClass::Loosening);
    }

    #[test]
    fn shortening_a_time_limit_tightens_and_lengthening_loosens() {
        let old = BlockList {
            plan: Some(Plan {
                mode: PlanMode::TimeLimit,
                time_limit: Some(TimeLimit { daily_minutes: 60 }),
                ..Plan::default()
            }),
            ..list()
        };
        let mut tighter = old.clone();
        tighter.plan.as_mut().unwrap().time_limit = Some(TimeLimit { daily_minutes: 30 });
        assert_eq!(classify_change(&old, &tighter), ChangeClass::Tightening);

        let mut looser = old.clone();
        looser.plan.as_mut().unwrap().time_limit = Some(TimeLimit { daily_minutes: 90 });
        assert_eq!(classify_change(&old, &looser), ChangeClass::Loosening);
    }

    #[test]
    fn wait_recomputation_survives_a_clock_moved_backwards() {
        let requested = Utc::now();
        let ready = requested + Duration::seconds(600);
        let earlier = requested - Duration::hours(2);
        let remaining = remaining_wait(requested, ready, earlier);
        assert!(remaining >= 600, "clock rollback must not shorten the wait");
    }
}
