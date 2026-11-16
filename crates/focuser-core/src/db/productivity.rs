//! Persistence for goals, tasks, focus sessions and notifications.
//!
//! Same JSON-column approach as `block_lists`: the Rust type is the schema,
//! the row carries only the fields needed to filter cheaply (`state`,
//! `status`, `goal_id`, timestamps). That keeps migrations boring and lets a
//! new optional field ship without a table rewrite.

use chrono::{DateTime, NaiveDate, Utc};
use focuser_common::error::{FocuserError, Result};
use focuser_common::productivity::{
    FocusSession, Goal, GoalState, Notification, NotificationKind, NotificationPriority, Task,
    TaskStatus,
};
use focuser_common::types::EntityId;
use rusqlite::{OptionalExtension, params};

use super::Database;

impl Database {
    // ─── Goals ─────────────────────────────────────────────

    pub fn save_goal(&self, goal: &Goal) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let json = serde_json::to_string(goal)?;
        conn.execute(
            "INSERT INTO goals (id, data, state, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data, state = excluded.state,
                updated_at = excluded.updated_at",
            params![
                goal.id.to_string(),
                json,
                goal_state_str(goal.state),
                goal.updated_at.to_rfc3339()
            ],
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_goal(&self, id: EntityId) -> Result<Goal> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let json: Option<String> = conn
            .query_row(
                "SELECT data FROM goals WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        match json {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Err(FocuserError::InvalidConfig("goal not found".into())),
        }
    }

    pub fn list_goals(&self, include_archived: bool) -> Result<Vec<Goal>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let sql = if include_archived {
            "SELECT data FROM goals ORDER BY updated_at DESC"
        } else {
            "SELECT data FROM goals WHERE state != 'archived' ORDER BY updated_at DESC"
        };
        let mut stmt = conn.prepare(sql).map_err(|e| FocuserError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| FocuserError::Database(e.to_string()))?
            .filter_map(std::result::Result::ok)
            .filter_map(|json| serde_json::from_str::<Goal>(&json).ok())
            .collect();
        Ok(rows)
    }

    pub fn delete_goal(&self, id: EntityId) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let tx = conn
            .transaction()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let orphans: Vec<(String, String)> = {
            let mut stmt = tx
                .prepare("SELECT id, data FROM tasks WHERE goal_id = ?1")
                .map_err(|e| FocuserError::Database(e.to_string()))?;
            stmt.query_map(params![id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| FocuserError::Database(e.to_string()))?
            .filter_map(std::result::Result::ok)
            .collect()
        };
        tx.execute("DELETE FROM goals WHERE id = ?1", params![id.to_string()])
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        // Tasks keep existing; a deleted goal must not take work with it, and
        // the JSON copy has to forget the link too or a reload restores it.
        for (task_id, json) in orphans {
            let mut task: Task = serde_json::from_str(&json)?;
            task.goal_id = None;
            task.updated_at = Utc::now();
            tx.execute(
                "UPDATE tasks SET data = ?2, goal_id = NULL, updated_at = ?3 WHERE id = ?1",
                params![
                    task_id,
                    serde_json::to_string(&task)?,
                    task.updated_at.to_rfc3339()
                ],
            )
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    // ─── Tasks ─────────────────────────────────────────────

    pub fn save_task(&self, task: &Task) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let json = serde_json::to_string(task)?;
        conn.execute(
            "INSERT INTO tasks (id, data, status, goal_id, updated_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data, status = excluded.status,
                goal_id = excluded.goal_id, updated_at = excluded.updated_at,
                completed_at = excluded.completed_at",
            params![
                task.id.to_string(),
                json,
                task_status_str(task.status),
                task.goal_id.map(|id| id.to_string()),
                task.updated_at.to_rfc3339(),
                task.completed_at.map(|at| at.to_rfc3339())
            ],
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_task(&self, id: EntityId) -> Result<Task> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let json: Option<String> = conn
            .query_row(
                "SELECT data FROM tasks WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        match json {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Err(FocuserError::InvalidConfig("task not found".into())),
        }
    }

    pub fn list_tasks(&self, include_archived: bool) -> Result<Vec<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let sql = if include_archived {
            "SELECT data FROM tasks ORDER BY updated_at DESC"
        } else {
            "SELECT data FROM tasks WHERE status != 'archived' ORDER BY updated_at DESC"
        };
        let mut stmt = conn.prepare(sql).map_err(|e| FocuserError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| FocuserError::Database(e.to_string()))?
            .filter_map(std::result::Result::ok)
            .filter_map(|json| serde_json::from_str::<Task>(&json).ok())
            .collect();
        Ok(rows)
    }

    pub fn delete_task(&self, id: EntityId) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.execute("DELETE FROM tasks WHERE id = ?1", params![id.to_string()])
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    /// Number of completed tasks linked to a goal, used for `Result` progress.
    pub fn completed_tasks_for_goal(&self, goal_id: EntityId) -> Result<u32> {
        self.completed_tasks_for_goal_since(goal_id, None)
    }

    /// Completed tasks in the current goal period. `from` is the local period
    /// start converted to UTC by the caller; `None` means the whole history,
    /// which is what a one-time goal asks for.
    pub fn completed_tasks_for_goal_since(
        &self,
        goal_id: EntityId,
        from: Option<DateTime<Utc>>,
    ) -> Result<u32> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let count = match from {
            Some(at) => conn.query_row(
                "SELECT COUNT(*) FROM tasks
                 WHERE goal_id = ?1 AND status = 'done' AND completed_at >= ?2",
                params![goal_id.to_string(), at.to_rfc3339()],
                |row| row.get::<_, i64>(0),
            ),
            None => conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE goal_id = ?1 AND status = 'done'",
                params![goal_id.to_string()],
                |row| row.get::<_, i64>(0),
            ),
        };
        count
            .map(|n| n.max(0) as u32)
            .map_err(|e| FocuserError::Database(e.to_string()))
    }

    // ─── Focus sessions ────────────────────────────────────

    pub fn save_focus_session(&self, session: &FocusSession) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let json = serde_json::to_string(session)?;
        conn.execute(
            "INSERT INTO focus_sessions (id, data, started_at, ended_at, goal_id, task_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data, ended_at = excluded.ended_at",
            params![
                session.id.to_string(),
                json,
                session.started_at.to_rfc3339(),
                session.ended_at.map(|t| t.to_rfc3339()),
                session.goal_id.map(|id| id.to_string()),
                session.task_id.map(|id| id.to_string())
            ],
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_focus_session(&self, id: EntityId) -> Result<FocusSession> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let data = conn
            .query_row(
                "SELECT data FROM focus_sessions WHERE id = ?1",
                params![id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| FocuserError::Database(e.to_string()))?
            .ok_or_else(|| FocuserError::InvalidConfig(format!("focus session not found: {id}")))?;
        serde_json::from_str(&data).map_err(|e| FocuserError::Database(e.to_string()))
    }

    pub fn focus_sessions_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<FocusSession>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT data FROM focus_sessions
                 WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at ASC",
            )
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![from.to_rfc3339(), to.to_rfc3339()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| FocuserError::Database(e.to_string()))?
            .filter_map(std::result::Result::ok)
            .filter_map(|json| serde_json::from_str::<FocusSession>(&json).ok())
            .collect();
        Ok(rows)
    }

    /// Completed sessions for a goal since a local date.
    pub fn completed_sessions_for_goal(&self, goal_id: EntityId, from: DateTime<Utc>) -> Result<u32> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.query_row(
            "SELECT COUNT(*) FROM focus_sessions
             WHERE goal_id = ?1 AND started_at >= ?2 AND ended_at IS NOT NULL",
            params![goal_id.to_string(), from.to_rfc3339()],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n.max(0) as u32)
        .map_err(|e| FocuserError::Database(e.to_string()))
    }

    // ─── Notifications ─────────────────────────────────────

    /// Insert a notification. A `dedupe_key` that already exists is ignored,
    /// so a reminder loop cannot fill the centre with the same line.
    pub fn push_notification(
        &self,
        kind: NotificationKind,
        priority: NotificationPriority,
        title: &str,
        body: &str,
        dedupe_key: Option<&str>,
    ) -> Result<Option<Notification>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let created_at = Utc::now();
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO notifications
                    (kind, priority, title, body, created_at, dedupe_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    kind_str(kind),
                    priority_str(priority),
                    title,
                    body,
                    created_at.to_rfc3339(),
                    dedupe_key
                ],
            )
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        if inserted == 0 {
            return Ok(None);
        }
        let id = conn.last_insert_rowid();
        Ok(Some(Notification {
            id,
            kind,
            priority,
            title: title.to_string(),
            body: body.to_string(),
            created_at,
            read_at: None,
            dedupe_key: dedupe_key.map(str::to_string),
        }))
    }

    pub fn list_notifications(&self, limit: u32, unread_only: bool) -> Result<Vec<Notification>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let sql = if unread_only {
            "SELECT id, kind, priority, title, body, created_at, read_at, dedupe_key
             FROM notifications WHERE read_at IS NULL ORDER BY id DESC LIMIT ?1"
        } else {
            "SELECT id, kind, priority, title, body, created_at, read_at, dedupe_key
             FROM notifications ORDER BY id DESC LIMIT ?1"
        };
        let mut stmt = conn.prepare(sql).map_err(|e| FocuserError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![limit.min(500)], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(|e| FocuserError::Database(e.to_string()))?
            .filter_map(std::result::Result::ok)
            .filter_map(
                |(id, kind, priority, title, body, created, read_at, dedupe)| {
                    Some(Notification {
                        id,
                        kind: parse_kind(&kind)?,
                        priority: parse_priority(&priority)?,
                        title,
                        body,
                        created_at: DateTime::parse_from_rfc3339(&created)
                            .ok()?
                            .with_timezone(&Utc),
                        read_at: read_at
                            .as_deref()
                            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                            .map(|dt| dt.with_timezone(&Utc)),
                        dedupe_key: dedupe,
                    })
                },
            )
            .collect();
        Ok(rows)
    }

    pub fn mark_notification_read(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.execute(
            "UPDATE notifications SET read_at = ?1 WHERE id = ?2 AND read_at IS NULL",
            params![Utc::now().to_rfc3339(), id],
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn mark_all_notifications_read(&self) -> Result<u32> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.execute(
            "UPDATE notifications SET read_at = ?1 WHERE read_at IS NULL",
            params![Utc::now().to_rfc3339()],
        )
        .map(|n| n as u32)
        .map_err(|e| FocuserError::Database(e.to_string()))
    }

    pub fn clear_notifications(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.execute("DELETE FROM notifications", [])
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        Ok(())
    }

    /// Daily focus totals, as `(local_date, focused_secs)`.
    pub fn focus_seconds_by_day(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<(NaiveDate, u64)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT date(started_at, 'localtime'), SUM(focused_secs) FROM (
                    SELECT started_at,
                           CAST(COALESCE(json_extract(data, '$.focused_secs'), 0) AS INTEGER) AS focused_secs
                    FROM focus_sessions WHERE ended_at IS NOT NULL
                 )
                 WHERE date(started_at) >= ?1 AND date(started_at) <= ?2
                 GROUP BY date(started_at, 'localtime') ORDER BY 1",
            )
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![from.to_string(), to.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| FocuserError::Database(e.to_string()))?
            .filter_map(std::result::Result::ok)
            .filter_map(|(day, secs)| {
                NaiveDate::parse_from_str(&day, "%Y-%m-%d")
                    .ok()
                    .map(|date| (date, secs.max(0) as u64))
            })
            .collect();
        Ok(rows)
    }
}

fn goal_state_str(state: GoalState) -> &'static str {
    match state {
        GoalState::Active => "active",
        GoalState::Completed => "completed",
        GoalState::Archived => "archived",
    }
}

fn task_status_str(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Todo => "todo",
        TaskStatus::Doing => "doing",
        TaskStatus::Done => "done",
        TaskStatus::Archived => "archived",
    }
}

fn kind_str(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::BreakDue => "break_due",
        NotificationKind::QuickTimerEnded => "quick_timer_ended",
        NotificationKind::GoalReached => "goal_reached",
        NotificationKind::GoalBehind => "goal_behind",
        NotificationKind::AllowanceWarning => "allowance_warning",
        NotificationKind::AllowanceExhausted => "allowance_exhausted",
        NotificationKind::BlockingHealth => "blocking_health",
        NotificationKind::CoachTip => "coach_tip",
        NotificationKind::UnlockReady => "unlock_ready",
    }
}

fn parse_kind(value: &str) -> Option<NotificationKind> {
    Some(match value {
        "break_due" => NotificationKind::BreakDue,
        "quick_timer_ended" => NotificationKind::QuickTimerEnded,
        "goal_reached" => NotificationKind::GoalReached,
        "goal_behind" => NotificationKind::GoalBehind,
        "allowance_warning" => NotificationKind::AllowanceWarning,
        "allowance_exhausted" => NotificationKind::AllowanceExhausted,
        "blocking_health" => NotificationKind::BlockingHealth,
        "coach_tip" => NotificationKind::CoachTip,
        "unlock_ready" => NotificationKind::UnlockReady,
        _ => return None,
    })
}

fn priority_str(priority: NotificationPriority) -> &'static str {
    match priority {
        NotificationPriority::Low => "low",
        NotificationPriority::Normal => "normal",
        NotificationPriority::High => "high",
    }
}

fn parse_priority(value: &str) -> Option<NotificationPriority> {
    Some(match value {
        "low" => NotificationPriority::Low,
        "normal" => NotificationPriority::Normal,
        "high" => NotificationPriority::High,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use focuser_common::productivity::{GoalKind, GoalPeriod};

    #[test]
    fn a_goal_round_trips() {
        let db = Database::open_in_memory().unwrap();
        let goal = Goal::new("Study", GoalKind::Sessions, 4);
        db.save_goal(&goal).unwrap();
        let loaded = db.get_goal(goal.id).unwrap();
        assert_eq!(loaded.title, "Study");
        assert_eq!(loaded.period, GoalPeriod::Week);
    }

    #[test]
    fn only_done_tasks_count_for_result_progress() {
        let db = Database::open_in_memory().unwrap();
        let goal = Goal::new("Ship", GoalKind::Result, 2);
        db.save_goal(&goal).unwrap();
        let mut task = Task::new("Write");
        task.goal_id = Some(goal.id);
        db.save_task(&task).unwrap();
        assert_eq!(db.completed_tasks_for_goal(goal.id).unwrap(), 0);
        task.status = TaskStatus::Done;
        task.updated_at = Utc::now();
        db.save_task(&task).unwrap();
        assert_eq!(db.completed_tasks_for_goal(goal.id).unwrap(), 1);
    }

    #[test]
    fn deleting_a_goal_keeps_its_tasks() {
        let db = Database::open_in_memory().unwrap();
        let goal = Goal::new("Ship", GoalKind::Result, 1);
        db.save_goal(&goal).unwrap();
        let mut task = Task::new("Write");
        task.goal_id = Some(goal.id);
        db.save_task(&task).unwrap();
        db.delete_goal(goal.id).unwrap();
        let loaded = db.get_task(task.id).unwrap();
        assert_eq!(loaded.goal_id, None);
    }

    #[test]
    fn a_duplicate_notification_is_dropped() {
        let db = Database::open_in_memory().unwrap();
        let first = db
            .push_notification(
                NotificationKind::BreakDue,
                NotificationPriority::High,
                "Break",
                "Stand up",
                Some("break:2026-11-15T10:00"),
            )
            .unwrap();
        assert!(first.is_some());
        let second = db
            .push_notification(
                NotificationKind::BreakDue,
                NotificationPriority::High,
                "Break",
                "Stand up",
                Some("break:2026-11-15T10:00"),
            )
            .unwrap();
        assert!(second.is_none());
        assert_eq!(db.list_notifications(10, true).unwrap().len(), 1);
    }

    #[test]
    fn marking_read_removes_from_the_unread_list() {
        let db = Database::open_in_memory().unwrap();
        let n = db
            .push_notification(
                NotificationKind::GoalReached,
                NotificationPriority::Normal,
                "Goal",
                "Done",
                None,
            )
            .unwrap()
            .unwrap();
        db.mark_notification_read(n.id).unwrap();
        assert!(db.list_notifications(10, true).unwrap().is_empty());
        assert_eq!(db.list_notifications(10, false).unwrap().len(), 1);
    }

    #[test]
    fn focus_sessions_are_counted_per_goal() {
        let db = Database::open_in_memory().unwrap();
        let goal = Goal::new("Focus", GoalKind::Sessions, 2);
        db.save_goal(&goal).unwrap();
        let session = FocusSession {
            id: focuser_common::types::new_id(),
            started_at: Utc::now() - chrono::Duration::minutes(30),
            ended_at: Some(Utc::now()),
            planned_secs: 1500,
            focused_secs: 1500,
            task_id: None,
            goal_id: Some(goal.id),
            block_list_id: None,
            source: "quick".into(),
            label: String::new(),
        };
        db.save_focus_session(&session).unwrap();
        assert_eq!(
            db.completed_sessions_for_goal(goal.id, Utc::now() - chrono::Duration::hours(1))
                .unwrap(),
            1
        );
    }
}
