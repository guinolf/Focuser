//! Turning measured state into a small, deduplicated notification centre.
//!
//! Nothing here invents an event. A reminder exists only when the database
//! already says a goal was reached or a screen-break plan is due, and the
//! stable `dedupe_key` means reopening the page does not repeat it.

use chrono::{DateTime, Local, Utc};
use focuser_common::error::Result;
use focuser_common::productivity::{
    CoachInsight, GoalKind, GoalPeriod, NotificationKind, NotificationPriority,
};
use focuser_common::protection::UnlockState;
use tracing::warn;

use crate::coach::goals_with_progress;
use crate::db::Database;
use crate::plans;

/// Human units for a measured goal, so a finished goal reads as minutes and
/// sessions rather than raw seconds.
fn goal_progress_text(kind: GoalKind, current: u64, target: u64) -> String {
    match kind {
        GoalKind::Time => format!(
            "{} de {}",
            human_duration(current),
            human_duration(target)
        ),
        GoalKind::Sessions => format!("{current} de {target} sessões"),
        GoalKind::Result => format!("{current} de {target} tarefas"),
    }
}

fn human_duration(secs: u64) -> String {
    let minutes = secs / 60;
    if minutes >= 60 {
        format!("{}h{:02}", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}min")
    }
}

/// Persist contextual reminders for the current instant.
///
/// Returns how many new rows were inserted this time. Duplicates are ignored
/// by the unique index on `dedupe_key`.
pub fn sync(db: &Database, now: DateTime<Utc>) -> Result<u32> {
    let mut added = 0;

    for progress in goals_with_progress(db, now)? {
        if !progress.reached || progress.goal.state != focuser_common::productivity::GoalState::Active {
            continue;
        }
        let scope = match progress.goal.period {
            GoalPeriod::Once => "once".to_string(),
            _ => progress.period_start.format("%Y-%m-%d").to_string(),
        };
        let key = format!("goal-reached:{}:{scope}", progress.goal.id);
        if db
            .push_notification(
                NotificationKind::GoalReached,
                NotificationPriority::Normal,
                &format!("Meta alcançada: {}", progress.goal.title),
                &format!(
                    "{} no período medido. Confirme a conclusão quando fizer sentido.",
                    goal_progress_text(progress.goal.kind, progress.current, progress.target)
                ),
                Some(&key),
            )?
            .is_some()
        {
            added += 1;
        }
    }

    for list in db.list_block_lists()? {
        if !list.enabled {
            continue;
        }
        let runtime = match plans::evaluate(db, &list, now) {
            Ok(runtime) => runtime,
            Err(error) => {
                // A broken plan must never take the notification centre down.
                warn!(list = %list.id, %error, "plan evaluation failed while syncing notifications");
                continue;
            }
        };
        let Some(due) = runtime.break_due_at else {
            continue;
        };
        let key = format!("break-due:{}:{}", list.id, due.timestamp());
        if db
            .push_notification(
                NotificationKind::BreakDue,
                NotificationPriority::Normal,
                "Hora de uma pausa",
                &format!(
                    "O plano «{}» atingiu o bloco de trabalho. Levante alguns minutos antes de voltar.",
                    list.name
                ),
                Some(&key),
            )?
            .is_some()
        {
            added += 1;
        }
    }

    for list in db.list_block_lists()? {
        let Some(end) = list.plan.as_ref().and_then(|plan| plan.quick_timer_ends_at) else {
            continue;
        };
        // Only the transition is news. An old timer should not resurface as a
        // fresh reminder every time the centre is opened.
        if end > now || now - end > chrono::Duration::minutes(30) {
            continue;
        }
        let key = format!("quick-timer-ended:{}:{}", list.id, end.timestamp());
        if db
            .push_notification(
                NotificationKind::QuickTimerEnded,
                NotificationPriority::Normal,
                "Timer rápido terminou",
                &format!(
                    "A janela de foco de «{}» terminou. O horário normal volta a valer.",
                    list.name
                ),
                Some(&key),
            )?
            .is_some()
        {
            added += 1;
        }
    }

    for list in db.list_block_lists()? {
        if !list.has_active_protection() {
            continue;
        }
        let id = list.id.to_string();
        let policy = db.protection_policy(&id)?.unwrap_or_default().sanitized();
        let status = db.unlock_status_at(&id, policy, now)?;
        if status.state != UnlockState::Ready {
            continue;
        }
        let Some(ready_at) = status.ready_at else {
            continue;
        };
        let key = format!("unlock-ready:{id}:{}", ready_at.timestamp());
        if db
            .push_notification(
                NotificationKind::UnlockReady,
                NotificationPriority::Normal,
                "Desafio de desbloqueio liberado",
                &format!(
                    "A espera de «{}» terminou. O bloqueio continua ativo até o desafio ser confirmado.",
                    list.name
                ),
                Some(&key),
            )?
            .is_some()
        {
            added += 1;
        }
    }

    let day = now.with_timezone(&Local).date_naive();
    for status in db.allowance_statuses_with_shared()? {
        if !status.allowance.enabled || status.paused_by_shared {
            continue;
        }
        let id = status.allowance.id;
        let exhausted = status.exhausted;
        let key = if exhausted {
            format!("allowance-exhausted:{id}:{day}")
        } else if status.remaining_secs <= 300 && status.used_today_secs > 0 {
            format!("allowance-warning:{id}:{day}")
        } else {
            continue;
        };
        let (kind, title, body) = if exhausted {
            (
                NotificationKind::AllowanceExhausted,
                "Cota diária esgotada",
                format!(
                    "«{}» atingiu o limite de hoje. O tempo restante só volta no próximo dia local.",
                    status.allowance.target.value_str()
                ),
            )
        } else {
            (
                NotificationKind::AllowanceWarning,
                "Cota diária quase no fim",
                format!(
                    "«{}» tem {} min restantes hoje.",
                    status.allowance.target.value_str(),
                    status.remaining_secs.div_ceil(60)
                ),
            )
        };
        if db
            .push_notification(kind, NotificationPriority::Normal, title, &body, Some(&key))?
            .is_some()
        {
            added += 1;
        }
    }

    Ok(added)
}

/// Keep the latest deterministic coach tips in the same centre, at most once
/// per day and per insight.
pub fn remember_insights(
    db: &Database,
    insights: &[CoachInsight],
    now: DateTime<Utc>,
) -> Result<u32> {
    let day = now.format("%Y-%m-%d");
    let mut added = 0;
    for insight in insights {
        let key = format!("coach:{day}:{}", insight.id);
        if db
            .push_notification(
                NotificationKind::CoachTip,
                insight.severity,
                &insight.title,
                &insight.body,
                Some(&key),
            )?
            .is_some()
        {
            added += 1;
        }
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use focuser_common::productivity::{Goal, GoalKind, Task, TaskStatus};

    #[test]
    fn a_reached_goal_is_remembered_once_per_period() {
        let db = Database::open_in_memory().unwrap();
        let now = Utc::now();
        let goal = Goal::new("ship the report", GoalKind::Result, 1);
        db.save_goal(&goal).unwrap();
        let mut task = Task::new("write it");
        task.goal_id = Some(goal.id);
        task.status = TaskStatus::Done;
        task.completed_at = Some(now);
        db.save_task(&task).unwrap();

        assert_eq!(sync(&db, now).unwrap(), 1);
        assert_eq!(sync(&db, now).unwrap(), 0, "same period must not repeat");
        let rows = db.list_notifications(10, false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, NotificationKind::GoalReached);
        assert!(rows[0].dedupe_key.as_deref().unwrap().contains(&goal.id.to_string()));

        // Describing old evidence must stay idempotent too.
        let insights = vec![CoachInsight {
            id: "one".into(),
            title: "One".into(),
            body: "Body".into(),
            evidence: vec![],
            severity: NotificationPriority::Low,
            action: None,
        }];
        assert_eq!(remember_insights(&db, &insights, now).unwrap(), 1);
        assert_eq!(remember_insights(&db, &insights, now).unwrap(), 0);
        assert_eq!(db.list_notifications(10, false).unwrap().len(), 2);
    }

    #[test]
    fn a_finished_quick_timer_is_reported_once() {
        let db = Database::open_in_memory().unwrap();
        let now = Utc::now();
        let mut list = focuser_common::types::BlockList::new("Focus");
        list.plan = Some(focuser_common::plan::Plan {
            quick_timer_ends_at: Some(now - chrono::Duration::minutes(1)),
            ..Default::default()
        });
        db.create_block_list(&list).unwrap();

        assert_eq!(sync(&db, now).unwrap(), 1);
        assert_eq!(sync(&db, now).unwrap(), 0);
        let rows = db.list_notifications(10, false).unwrap();
        assert_eq!(rows[0].kind, NotificationKind::QuickTimerEnded);
    }

    #[test]
    fn a_ready_unlock_is_announced_without_unlocking() {
        let db = Database::open_in_memory().unwrap();
        let now = Utc::now();
        let mut list = focuser_common::types::BlockList::new("Locked");
        list.protection = Some(focuser_common::types::Protection {
            prevent_uninstall: false,
            prevent_service_stop: false,
            prevent_modification: false,
            started_at: now - chrono::Duration::days(1),
            expires_at: Some(now + chrono::Duration::days(1)),
        });
        db.create_block_list(&list).unwrap();
        db.start_unlock_request(
            &list.id.to_string(),
            "challenge",
            600,
            now - chrono::Duration::minutes(11),
        )
        .unwrap();

        assert_eq!(sync(&db, now).unwrap(), 1);
        assert_eq!(sync(&db, now).unwrap(), 0);
        let rows = db.list_notifications(10, false).unwrap();
        assert_eq!(rows[0].kind, NotificationKind::UnlockReady);
    }

    #[test]
    fn goal_reminders_use_minutes_and_units_instead_of_raw_seconds() {
        assert_eq!(goal_progress_text(GoalKind::Time, 1500, 6000), "25min de 1h40");
        assert_eq!(
            goal_progress_text(GoalKind::Sessions, 2, 5),
            "2 de 5 sessões"
        );
        assert_eq!(
            goal_progress_text(GoalKind::Result, 1, 3),
            "1 de 3 tarefas"
        );
    }
}
