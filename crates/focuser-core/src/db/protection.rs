//! Persistence for protection policy and the delayed unlock protocol.
//!
//! The wait lives here rather than in memory on purpose: an app restart, a
//! crash, or a `SIGKILL` must not shorten it. Every read recomputes the
//! remaining time from `(requested_at, ready_at, now)` and treats a clock that
//! moved backwards as "restart the wait", so the stored timestamp can be
//! trusted only in the direction that hurts.
//!
//! Wrong answers are counted per list in a rolling hour. The count is used both
//! to refuse issuing a new challenge and to lock the panel out for the rest of
//! the window, which makes brute-forcing a 20-character string impractical
//! without pretending the check is cryptographically enforced.

use chrono::{DateTime, Duration, Utc};
use focuser_common::error::{FocuserError, Result};
use focuser_common::protection::{
    MIN_WAIT_SECS, ProtectionPolicy, UnlockState, UnlockStatus, remaining_wait,
};
use rusqlite::params;
use tracing::warn;

use super::Database;

/// One stored unlock request. The challenge is secret and never leaves this
/// module except through the command that generated it.
#[derive(Debug, Clone)]
pub struct StoredUnlockRequest {
    pub challenge: String,
    pub requested_at: DateTime<Utc>,
    pub ready_at: DateTime<Utc>,
    pub wait_secs: u32,
    pub state: UnlockState,
}

fn policy_key(block_list_id: &str) -> String {
    format!("protection_policy:{block_list_id}")
}

impl Database {
    pub fn protection_policy(&self, block_list_id: &str) -> Result<Option<ProtectionPolicy>> {
        let Some(value) = self.get_setting(&policy_key(block_list_id))? else {
            return Ok(None);
        };
        match serde_json::from_str::<ProtectionPolicy>(&value) {
            Ok(policy) => Ok(Some(policy.sanitized())),
            Err(e) => {
                // A corrupt policy must not silently become "no policy", which
                // would mean an immediate unlock. Fall back to the strictest
                // defaults instead.
                warn!(error = %e, "unreadable protection policy, using firm defaults");
                Ok(Some(ProtectionPolicy::default()))
            }
        }
    }

    pub fn set_protection_policy(
        &self,
        block_list_id: &str,
        policy: ProtectionPolicy,
    ) -> Result<ProtectionPolicy> {
        let policy = policy.sanitized();
        let json = serde_json::to_string(&policy)?;
        self.set_setting(&policy_key(block_list_id), &json)?;
        Ok(policy)
    }

    pub fn clear_protection_policy(&self, block_list_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.execute(
            "DELETE FROM settings WHERE key = ?1",
            params![policy_key(block_list_id)],
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    /// Begin (or restart) the wait for a list. Any previous request is
    /// replaced, and a previously issued challenge becomes useless.
    pub fn start_unlock_request(
        &self,
        block_list_id: &str,
        challenge: &str,
        wait_secs: u32,
        now: DateTime<Utc>,
    ) -> Result<StoredUnlockRequest> {
        let wait_secs = wait_secs.max(MIN_WAIT_SECS);
        let ready_at = now + Duration::seconds(i64::from(wait_secs));
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO unlock_requests
                (block_list_id, challenge, requested_at, ready_at, wait_secs, state, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?3)
             ON CONFLICT(block_list_id) DO UPDATE SET
                challenge = excluded.challenge,
                requested_at = excluded.requested_at,
                ready_at = excluded.ready_at,
                wait_secs = excluded.wait_secs,
                state = excluded.state,
                updated_at = excluded.updated_at",
            params![
                block_list_id,
                challenge,
                now.to_rfc3339(),
                ready_at.to_rfc3339(),
                wait_secs,
                "waiting",
            ],
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(StoredUnlockRequest {
            challenge: challenge.to_string(),
            requested_at: now,
            ready_at,
            wait_secs,
            state: UnlockState::Waiting,
        })
    }

    pub fn unlock_request(&self, block_list_id: &str) -> Result<Option<StoredUnlockRequest>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let row = conn
            .query_row(
                "SELECT challenge, requested_at, ready_at, wait_secs, state
                 FROM unlock_requests WHERE block_list_id = ?1",
                params![block_list_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional_row()
            .map_err(|e| FocuserError::Database(e.to_string()))?;

        let Some((challenge, requested, ready, wait_secs, state)) = row else {
            return Ok(None);
        };
        let parse = |s: &str| {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| FocuserError::Database(e.to_string()))
        };
        Ok(Some(StoredUnlockRequest {
            challenge,
            requested_at: parse(&requested)?,
            ready_at: parse(&ready)?,
            wait_secs,
            state: match state.as_str() {
                "ready" => UnlockState::Ready,
                "cancelled" => UnlockState::Cancelled,
                _ => UnlockState::Waiting,
            },
        }))
    }

    pub fn cancel_unlock_request(&self, block_list_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.execute(
            "UPDATE unlock_requests SET state = 'cancelled', updated_at = ?2
             WHERE block_list_id = ?1",
            params![block_list_id, Utc::now().to_rfc3339()],
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    /// Drop the request after a successful unlock, or after a wrong answer
    /// (which restarts the wait on the next request).
    pub fn clear_unlock_request(&self, block_list_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.execute(
            "DELETE FROM unlock_requests WHERE block_list_id = ?1",
            params![block_list_id],
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn record_unlock_attempt(
        &self,
        block_list_id: &str,
        ok: bool,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO unlock_attempts (block_list_id, at, ok) VALUES (?1, ?2, ?3)",
            params![block_list_id, now.to_rfc3339(), ok],
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    /// Failed answers inside the rolling hour.
    pub fn failed_unlock_attempts(
        &self,
        block_list_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<DateTime<Utc>>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let since = (now - Duration::hours(1)).to_rfc3339();
        let mut stmt = conn
            .prepare(
                "SELECT at FROM unlock_attempts
                 WHERE block_list_id = ?1 AND ok = 0 AND at >= ?2
                 ORDER BY at ASC",
            )
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![block_list_id, since], |row| row.get::<_, String>(0))
            .map_err(|e| FocuserError::Database(e.to_string()))?
            .filter_map(std::result::Result::ok)
            .filter_map(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .collect();
        Ok(rows)
    }

    /// Build the status shown by the protection panel.
    pub fn unlock_status(&self, block_list_id: &str, policy: ProtectionPolicy) -> Result<UnlockStatus> {
        self.unlock_status_at(block_list_id, policy, Utc::now())
    }

    /// Same as [`Self::unlock_status`] with an explicit instant, so callers
    /// with an injected clock (and tests) see the same countdown.
    pub fn unlock_status_at(
        &self,
        block_list_id: &str,
        policy: ProtectionPolicy,
        now: DateTime<Utc>,
    ) -> Result<UnlockStatus> {
        let attempts = self.failed_unlock_attempts(block_list_id, now)?;
        let max = policy.max_attempts_per_hour;
        let attempts_remaining = max.saturating_sub(attempts.len() as u32);
        let locked_out_until = if attempts.len() as u32 >= max {
            attempts
                .first()
                .map(|first| *first + Duration::hours(1))
        } else {
            None
        };

        let Some(request) = self.unlock_request(block_list_id)? else {
            return Ok(UnlockStatus {
                attempts_remaining,
                locked_out_until,
                ..UnlockStatus::idle(block_list_id.to_string(), policy)
            });
        };

        let remaining = remaining_wait(request.requested_at, request.ready_at, now);
        let state = match request.state {
            UnlockState::Cancelled => UnlockState::Cancelled,
            _ if remaining == 0 => UnlockState::Ready,
            _ => UnlockState::Waiting,
        };
        Ok(UnlockStatus {
            block_list_id: block_list_id.to_string(),
            state,
            requested_at: Some(request.requested_at),
            ready_at: Some(request.ready_at),
            remaining_secs: remaining,
            attempts_remaining,
            locked_out_until,
            challenge_len: policy.challenge_len,
        })
    }
}

/// `OptionalExtension` without importing it in every call site.
trait OptionalRow<T> {
    fn optional_row(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalRow<T> for rusqlite::Result<T> {
    fn optional_row(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use focuser_common::protection::ProtectionPreset;

    #[test]
    fn an_unstarted_request_is_idle() {
        let db = Database::open_in_memory().unwrap();
        let status = db
            .unlock_status("list", ProtectionPolicy::default())
            .unwrap();
        assert_eq!(status.state, UnlockState::Idle);
        assert_eq!(status.remaining_secs, 0);
    }

    #[test]
    fn a_new_request_waits_the_full_minimum_even_if_asked_for_less() {
        let db = Database::open_in_memory().unwrap();
        let now = Utc::now();
        db.start_unlock_request("list", "ABCDEFGHIJKLMNOPQRST", 1, now)
            .unwrap();
        let status = db
            .unlock_status("list", ProtectionPolicy::default())
            .unwrap();
        assert_eq!(status.state, UnlockState::Waiting);
        assert!(status.remaining_secs >= MIN_WAIT_SECS as u64 - 1);
    }

    #[test]
    fn the_request_survives_a_reopen_of_the_same_database() {
        let dir = std::env::temp_dir().join(format!("focuser-unlock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("focuser.db");
        {
            let db = Database::open(&path).unwrap();
            db.start_unlock_request("list", "ABCDEFGHIJKLMNOPQRST", 600, Utc::now())
                .unwrap();
        }
        {
            let db = Database::open(&path).unwrap();
            let request = db.unlock_request("list").unwrap().unwrap();
            assert_eq!(request.challenge, "ABCDEFGHIJKLMNOPQRST");
            assert_eq!(request.state, UnlockState::Waiting);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_wait_never_shortens_and_a_clock_rollback_restarts_it() {
        let db = Database::open_in_memory().unwrap();
        let start = Utc::now();
        db.start_unlock_request("list", "ABCDEFGHIJKLMNOPQRST", 600, start)
            .unwrap();
        let policy = ProtectionPolicy::default();

        let halfway = db
            .unlock_status_at("list", policy.clone(), start + Duration::seconds(300))
            .unwrap();
        assert_eq!(halfway.state, UnlockState::Waiting);
        assert_eq!(halfway.remaining_secs, 300);

        // The machine clock moving backwards is treated as hostile: the full
        // wait still lies ahead rather than being skipped.
        let rolled_back = db
            .unlock_status_at("list", policy.clone(), start - Duration::seconds(3600))
            .unwrap();
        assert_eq!(rolled_back.state, UnlockState::Waiting);
        assert!(rolled_back.remaining_secs >= 600);

        let done = db
            .unlock_status_at("list", policy, start + Duration::seconds(601))
            .unwrap();
        assert_eq!(done.state, UnlockState::Ready);
        assert_eq!(done.remaining_secs, 0);
    }

    #[test]
    fn failed_answers_eventually_lock_the_panel_out() {
        let db = Database::open_in_memory().unwrap();
        let now = Utc::now();
        for _ in 0..5 {
            db.record_unlock_attempt("list", false, now).unwrap();
        }
        let status = db
            .unlock_status("list", ProtectionPolicy::default())
            .unwrap();
        assert_eq!(status.attempts_remaining, 0);
        assert!(status.locked_out_until.is_some());
    }

    #[test]
    fn a_correct_answer_does_not_count_as_a_failed_attempt() {
        let db = Database::open_in_memory().unwrap();
        db.record_unlock_attempt("list", true, Utc::now()).unwrap();
        assert_eq!(db.failed_unlock_attempts("list", Utc::now()).unwrap().len(), 0);
        let status = db
            .unlock_status("list", ProtectionPolicy::default())
            .unwrap();
        assert_eq!(status.attempts_remaining, 5);
    }

    #[test]
    fn a_policy_round_trips_and_is_sanitized_on_read() {
        let db = Database::open_in_memory().unwrap();
        db.set_protection_policy("list", ProtectionPolicy::from_preset(ProtectionPreset::Closed))
            .unwrap();
        let policy = db.protection_policy("list").unwrap().unwrap();
        assert_eq!(policy.preset, ProtectionPreset::Closed);
        assert!(policy.wait_secs >= MIN_WAIT_SECS);
    }

    #[test]
    fn a_corrupt_policy_falls_back_to_firm_not_to_none() {
        let db = Database::open_in_memory().unwrap();
        db.set_setting("protection_policy:list", "{not json").unwrap();
        let policy = db.protection_policy("list").unwrap().unwrap();
        assert_eq!(policy.preset, ProtectionPreset::Firm);
        assert!(policy.wait_secs >= MIN_WAIT_SECS);
    }

    #[test]
    fn restarting_a_request_replaces_the_old_one() {
        let db = Database::open_in_memory().unwrap();
        db.start_unlock_request("list", "AAAAAAAAAAAAAAAAAAAA", 600, Utc::now())
            .unwrap();
        db.start_unlock_request("list", "BBBBBBBBBBBBBBBBBBBB", 600, Utc::now())
            .unwrap();
        assert_eq!(
            db.unlock_request("list").unwrap().unwrap().challenge,
            "BBBBBBBBBBBBBBBBBBBB"
        );
    }
}
