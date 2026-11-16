//! Local, deterministic coach.
//!
//! No model, no account, no network: every insight is a rule over measured
//! activity, and every one carries the numbers it was derived from. When there
//! is nothing to say it returns nothing — an empty list is a valid answer,
//! unlike the fake "insights" a marketing page would generate.

use chrono::{Datelike, Duration, Local, Utc};
use focuser_common::activity::{ActivityMeasurement, ActivitySource};
use focuser_common::error::Result;
use focuser_common::productivity::{
    CoachAction, CoachInsight, GoalKind, GoalPeriod, GoalState, NotificationPriority,
};

use crate::db::Database;
use crate::plans;

/// Build insights for the last seven days, newest evidence first.
pub fn insights(
    db: &Database,
    now: chrono::DateTime<Utc>,
    extension_connected: bool,
    app_usage_measurable: bool,
    collection_enabled: bool,
) -> Result<Vec<CoachInsight>> {
    let today = now.with_timezone(&Local).date_naive();
    let week_ago = today - Duration::days(6);
    // Activity for the insight window is measured, not estimated; targets and
    // goals below come from the same database the UI reads.
    let overview = crate::activity::overview(
        db,
        week_ago,
        today,
        extension_connected,
        app_usage_measurable,
        collection_enabled,
    )?;
    let mut out = Vec::new();

    if overview.measurement == ActivityMeasurement::Unavailable {
        out.push(CoachInsight {
            id: "activity-unavailable".into(),
            title: "Sem dados de atividade nesta máquina".into(),
            body: "Nem o navegador nem o observador local estão reportando. As sugestões abaixo ficam limitadas até isso mudar.".into(),
            evidence: vec!["nenhuma fonte de atividade conectada".into()],
            severity: NotificationPriority::Low,
            action: None,
        });
        return Ok(out);
    }

    // The biggest target of the week, if it is a meaningful share of the time.
    let total: u64 = overview.targets.iter().map(|t| t.foreground_secs).sum();
    if total > 0 {
        let top = overview
            .targets
            .iter()
            .max_by_key(|t| t.foreground_secs)
            .expect("non-empty");
        let share = top.foreground_secs as f64 / total as f64;
        if share >= 0.35 && top.foreground_secs >= 15 * 60 {
            let minutes = (top.foreground_secs / 60).min(u32::MAX as u64) as u32;
            out.push(CoachInsight {
                id: format!("top-target-{}", top.target),
                title: format!("{} concentra {:.0}% do tempo da semana", top.target, share * 100.0),
                body: format!(
                    "Foram {} em sete dias. Um limite diário ou uma pausa planejada costuma ser mais eficaz que bloquear de uma vez.",
                    format_secs(top.foreground_secs)
                ),
                evidence: vec![
                    format!("{} de tempo em primeiro plano", format_secs(top.foreground_secs)),
                    format!("{:.0}% do total medido", share * 100.0),
                ],
                severity: NotificationPriority::Low,
                action: Some(match top.source {
                    ActivitySource::BrowserTab => CoachAction::CreateAllowance {
                        hostname: top.target.clone(),
                        minutes: (minutes / 7).max(15),
                    },
                    ActivitySource::ForegroundApp => CoachAction::OpenPlans,
                }),
            });
        }
    }

    // Long unbroken stretches: the screen-break plan exists for this.
    if let Some(goal) = db
        .list_goals(false)?
        .into_iter()
        .find(|g| g.kind == GoalKind::Time && g.state == GoalState::Active)
    {
        let progress = goal_progress(db, &goal, goal.period.start(today), today, now)?;
        let expected =
            goal.target_value as u64 * 60 * u64::from(period_fraction(goal.period, today)) / 100;
        if progress < expected {
            out.push(CoachInsight {
                id: format!("goal-behind-{}", goal.id),
                title: format!("Meta atrás do ritmo: {}", goal.title),
                body: format!(
                    "Até agora são {} de {}. O período ainda não terminou; ajuste o plano ou reserve um bloco na agenda.",
                    format_secs(progress),
                    format_secs(goal.target_value as u64 * 60)
                ),
                evidence: vec![
                    format!("progresso medido: {}", format_secs(progress)),
                    format!("esperado para hoje: {}", format_secs(expected)),
                ],
                severity: NotificationPriority::Normal,
                action: Some(CoachAction::OpenPlans),
            });
        }
    }

    if overview.targets.iter().any(|t| t.foreground_secs >= 2 * 3600) {
        out.push(CoachInsight {
            id: "screen-break".into(),
            title: "Blocos longos sem pausa".into(),
            body: "Uma lista com modo Pausa de tela lembra você de levantar sem bloquear nada.".into(),
            evidence: vec!["há um alvo com 2h ou mais em primeiro plano na semana".into()],
            severity: NotificationPriority::Low,
            action: Some(CoachAction::AdjustScreenBreak {
                work_minutes: 50,
                break_minutes: 10,
            }),
        });
    }

    // Overdue tasks are concrete and never invented.
    let overdue = db
        .list_tasks(false)?
        .into_iter()
        .filter(|t| {
            t.status != focuser_common::productivity::TaskStatus::Done
                && t.due_date.is_some_and(|d| d < today)
        })
        .count();
    if overdue > 0 {
        out.push(CoachInsight {
            id: "tasks-overdue".into(),
            title: format!("{overdue} tarefa(s) atrasada(s)"),
            body: "Escolha uma e comece uma sessão de foco de 25 minutos: metade do atraso some no primeiro passo.".into(),
            evidence: vec![format!("{overdue} tarefas com data anterior a hoje")],
            severity: NotificationPriority::Normal,
            action: Some(CoachAction::OpenPlans),
        });
    }

    Ok(out)
}

/// Percentage of the goal period already elapsed. Monday starts the week, as
/// in the rest of the model.
fn period_fraction(period: GoalPeriod, today: chrono::NaiveDate) -> u32 {
    match period {
        GoalPeriod::Day => 100,
        GoalPeriod::Week => {
            let elapsed = today.weekday().num_days_from_monday() as u32 + 1;
            elapsed * 100 / 7
        }
        GoalPeriod::Month => {
            let elapsed = today.day();
            let days = chrono::NaiveDate::from_ymd_opt(
                today.year(),
                today.month(),
                1,
            )
            .and_then(|first| {
                let next = if today.month() == 12 {
                    chrono::NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
                } else {
                    chrono::NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
                };
                next.map(|n| (n - first).num_days() as u32)
            })
            .unwrap_or(30);
            elapsed * 100 / days.max(1)
        }
        GoalPeriod::Once => 100,
    }
}

fn goal_progress(
    db: &Database,
    goal: &focuser_common::productivity::Goal,
    period_start: chrono::NaiveDate,
    today: chrono::NaiveDate,
    now: chrono::DateTime<Utc>,
) -> Result<u64> {
    // Outcome goals are measured by tasks completed inside the same period,
    // so they do not need a plan.
    if goal.kind == GoalKind::Result {
        let from = match goal.period {
            GoalPeriod::Once => None,
            _ => Some(crate::db::activity::local_range(period_start, today)?.0),
        };
        return Ok(u64::from(db.completed_tasks_for_goal_since(goal.id, from)?));
    }
    let Some(list_id) = goal.block_list_id else {
        return Ok(0);
    };
    let lists = db.list_block_lists()?;
    let Some(list) = lists.iter().find(|l| l.id == list_id) else {
        return Ok(0);
    };
    // A weekly goal compares a week of usage with its weekly target. Using
    // only today's usage made every period longer than a day under-report.
    let from = match goal.period {
        GoalPeriod::Once => chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap_or(period_start),
        _ => period_start,
    };
    let usage = plans::measure_plan_usage_between(db, list, from, today, now)?;
    Ok(match goal.kind {
        GoalKind::Time => usage.used_secs,
        GoalKind::Sessions => u64::from(usage.sessions_used),
        GoalKind::Result => unreachable!("handled above"),
    })
}

/// Every active goal together with the value actually measured for its
/// current period. This is the only source the UI uses for progress; no
/// percentage is invented when a goal has no linked plan.
pub fn goals_with_progress(
    db: &Database,
    now: chrono::DateTime<Utc>,
) -> Result<Vec<focuser_common::productivity::GoalProgress>> {
    let today = now.with_timezone(&Local).date_naive();
    let mut out = Vec::new();
    for goal in db.list_goals(false)? {
        let period_start = goal.period.start(today);
        let current = goal_progress(db, &goal, period_start, today, now)?;
        out.push(focuser_common::productivity::GoalProgress::from_goal(
            goal,
            current,
            period_start,
        ));
    }
    Ok(out)
}

fn format_secs(secs: u64) -> String {
    let minutes = secs / 60;
    if minutes >= 60 {
        format!("{}h{:02}", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}min")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_fractions_are_ordered() {
        let monday = chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        assert_eq!(period_fraction(GoalPeriod::Week, monday), 14);
        assert!(period_fraction(GoalPeriod::Month, monday) < 100);
        assert_eq!(period_fraction(GoalPeriod::Day, monday), 100);
    }

    #[test]
    fn outcome_goals_progress_from_completed_tasks_without_a_plan() {
        let db = Database::open_in_memory().unwrap();
        let mut goal = focuser_common::productivity::Goal::new(
            "ship the report",
            GoalKind::Result,
            2,
        );
        goal.period = GoalPeriod::Week;
        db.save_goal(&goal).unwrap();
        let mut task = focuser_common::productivity::Task::new("draft");
        task.goal_id = Some(goal.id);
        task.status = focuser_common::productivity::TaskStatus::Done;
        task.completed_at = Some(Utc::now());
        db.save_task(&task).unwrap();

        let progress = goals_with_progress(&db, Utc::now()).unwrap();
        assert_eq!(progress.len(), 1);
        assert_eq!(progress[0].current, 1);
        assert_eq!(progress[0].target, 2);
        assert_eq!(progress[0].percent, 50);
        assert!(!progress[0].reached);
    }

    fn local_noon(date: chrono::NaiveDate) -> chrono::DateTime<Utc> {
        date.and_hms_opt(12, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn a_weekly_goal_counts_the_whole_period_not_just_today() {
        use focuser_common::productivity::{Goal, GoalKind, GoalPeriod};
        use focuser_common::types::{BlockList, WebsiteRule};

        let db = Database::open_in_memory().unwrap();
        let mut list = BlockList::new("Work");
        list.websites.push(WebsiteRule::domain("youtube.com"));
        db.create_block_list(&list).unwrap();

        let today = Local::now().date_naive();
        let days_to_wednesday = (2 + 7 - i64::from(today.weekday().num_days_from_monday())) % 7;
        let wednesday = today + Duration::days(days_to_wednesday);
        let monday = wednesday - Duration::days(2);
        let now = local_noon(wednesday);

        let mut weekly = Goal::new("weekly", GoalKind::Time, 2);
        weekly.period = GoalPeriod::Week;
        weekly.block_list_id = Some(list.id);
        db.save_goal(&weekly).unwrap();
        let mut daily = Goal::new("daily", GoalKind::Time, 2);
        daily.period = GoalPeriod::Day;
        daily.block_list_id = Some(list.id);
        db.save_goal(&daily).unwrap();

        let mut first = focuser_common::activity::ActivityTick {
            source: focuser_common::activity::ActivitySource::BrowserTab,
            target: "youtube.com".into(),
            increment_secs: Some(60),
            interaction: true,
            idle: false,
            observed_at: Some(local_noon(monday)),
            device_id: Some("test".into()),
        };
        first.normalize();
        db.record_activity_tick(&first).unwrap();
        let mut second = first.clone();
        second.observed_at = Some(local_noon(wednesday));
        second.normalize();
        db.record_activity_tick(&second).unwrap();

        let progress = goals_with_progress(&db, now).unwrap();
        let weekly = progress.iter().find(|p| p.goal.id == weekly.id).unwrap();
        let daily = progress.iter().find(|p| p.goal.id == daily.id).unwrap();
        assert_eq!(weekly.current, 120, "both days belong to the week");
        assert!(weekly.reached);
        assert_eq!(daily.current, 60, "Monday is outside today");
        assert_eq!(daily.percent, 50);
    }

    #[test]
    fn result_goals_count_only_tasks_completed_in_their_period() {
        use focuser_common::productivity::{Goal, GoalKind, GoalPeriod, Task, TaskStatus};

        let db = Database::open_in_memory().unwrap();
        let today = Local::now().date_naive();
        let days_to_wednesday = (2 + 7 - i64::from(today.weekday().num_days_from_monday())) % 7;
        let wednesday = today + Duration::days(days_to_wednesday);
        let monday = wednesday - Duration::days(2);
        let now = local_noon(wednesday);

        let mut goal = Goal::new("ship", GoalKind::Result, 2);
        goal.period = GoalPeriod::Week;
        db.save_goal(&goal).unwrap();

        for (index, completed_at) in [local_noon(monday), local_noon(monday - Duration::days(7))]
            .into_iter()
            .enumerate()
        {
            let mut task = Task::new(format!("draft {index}"));
            task.goal_id = Some(goal.id);
            task.status = TaskStatus::Done;
            task.completed_at = Some(completed_at);
            db.save_task(&task).unwrap();
        }

        let progress = goals_with_progress(&db, now).unwrap();
        assert_eq!(progress[0].current, 1, "only this week's task counts");
    }

    #[test]
    fn a_machine_without_sources_gets_an_honest_message() {
        let db = Database::open_in_memory().unwrap();
        let out = insights(&db, Utc::now(), false, false, true).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "activity-unavailable");
        assert!(out[0].action.is_none());
    }
}
