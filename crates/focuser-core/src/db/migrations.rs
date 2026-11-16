use focuser_common::error::{FocuserError, Result};
use focuser_common::types::BlockList;
use rusqlite::Connection;
use tracing::info;

/// One migration step: either schema SQL, or a data transform that needs
/// Rust to deserialize and rewrite rows (SQLite alone cannot rewrite the
/// JSON `data` column against Serde's rules).
enum Step {
    Sql(&'static str),
    Code(fn(&Connection) -> Result<()>),
}

/// Reclassify website rules that were stored as the wrong match type —
/// `Wildcard("*word*")` into `Keyword("word")`, and any `Domain` value that
/// was typed with a `*` in it into `Wildcard` or `Keyword` — see
/// [`focuser_common::types::WebsiteMatchType::simplify`].
///
/// Idempotent, so it is safe to run again as a later migration when
/// `simplify` itself grows to catch more shapes: rules `simplify` already
/// fixed just come back unchanged.
fn reclassify_mistyped_website_rules(conn: &Connection) -> Result<()> {
    rewrite_block_lists(conn, |list| {
        let mut changed = false;
        for rule in &mut list.websites {
            let before = rule.match_type.clone();
            rule.match_type.simplify();
            changed |= rule.match_type != before;
        }
        changed
    })
}

/// Load every block list, let `edit` change it, and write back the ones it
/// reports as changed. Rows that no longer deserialize are left alone.
fn rewrite_block_lists(
    conn: &Connection,
    mut edit: impl FnMut(&mut BlockList) -> bool,
) -> Result<()> {
    let mut stmt = conn
        .prepare("SELECT id, data FROM block_lists")
        .map_err(|e| FocuserError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| FocuserError::Database(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| FocuserError::Database(e.to_string()))?;
    drop(stmt);

    for (id, json) in rows {
        let Ok(mut list) = serde_json::from_str::<BlockList>(&json) else {
            continue;
        };
        if !edit(&mut list) {
            continue;
        }

        let updated =
            serde_json::to_string(&list).map_err(|e| FocuserError::Database(e.to_string()))?;
        conn.execute(
            "UPDATE block_lists SET data = ?1 WHERE id = ?2",
            rusqlite::params![updated, id],
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?;
    }

    Ok(())
}

/// A focus session can be locked for some rounds (#47). Looked for first, so
/// running it again changes nothing.
fn add_pomodoro_lock(conn: &Connection) -> Result<()> {
    if conn
        .prepare("SELECT locked_rounds FROM pomodoro_sessions LIMIT 0")
        .is_ok()
    {
        return Ok(());
    }
    conn.execute_batch(
        "ALTER TABLE pomodoro_sessions ADD COLUMN locked_rounds INTEGER NOT NULL DEFAULT 0;",
    )
    .map_err(|e| FocuserError::Database(e.to_string()))
}

/// Tasks gained an explicit completion timestamp so weekly goals can count
/// only the tasks finished inside their period. Older databases have the
/// table without the column; a re-run (the shared-allowance test rewinds
/// `schema_version`) must not fail with a duplicate column.
fn add_task_completed_at(conn: &Connection) -> Result<()> {
    let has_column = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'completed_at'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| FocuserError::Database(e.to_string()))?
        > 0;
    if !has_column {
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN completed_at TEXT;")
            .map_err(|e| FocuserError::Database(e.to_string()))?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_tasks_completed
            ON tasks(goal_id, completed_at);",
    )
    .map_err(|e| FocuserError::Database(e.to_string()))
}

/// Run all database migrations.
pub fn run_all(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        );",
    )
    .map_err(|e| FocuserError::Database(e.to_string()))?;

    let current_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let migrations: &[(&str, Step)] = &[
        (
            "v1: block_lists and statistics",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS block_lists (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                data TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS statistics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                domain_or_app TEXT NOT NULL,
                duration_seconds INTEGER NOT NULL DEFAULT 0,
                blocked_attempts INTEGER NOT NULL DEFAULT 0,
                date TEXT NOT NULL,
                UNIQUE(domain_or_app, date)
            );

            CREATE TABLE IF NOT EXISTS active_blocks (
                block_list_id TEXT PRIMARY KEY,
                started_at TEXT NOT NULL,
                expires_at TEXT,
                lock_type TEXT,
                lock_data TEXT
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
            ),
        ),
        (
            "v2: blocked_events for fine-grained timeline",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS blocked_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                domain_or_app TEXT NOT NULL,
                timestamp TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_blocked_events_timestamp
                ON blocked_events(timestamp);
            CREATE INDEX IF NOT EXISTS idx_blocked_events_domain
                ON blocked_events(domain_or_app);",
            ),
        ),
        (
            "v3: pomodoro sessions and phase log",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS pomodoro_sessions (
                id TEXT PRIMARY KEY,
                block_list_id TEXT NOT NULL,
                work_secs INTEGER NOT NULL,
                short_break_secs INTEGER NOT NULL,
                long_break_secs INTEGER NOT NULL,
                cycles_until_long_break INTEGER NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                completed_cycles INTEGER NOT NULL DEFAULT 0,
                current_phase TEXT NOT NULL,
                current_cycle INTEGER NOT NULL DEFAULT 1,
                phase_started_at TEXT NOT NULL,
                paused_remaining_secs INTEGER,
                prev_enabled INTEGER NOT NULL DEFAULT 1
            );

            CREATE INDEX IF NOT EXISTS idx_pomodoro_active
                ON pomodoro_sessions(ended_at);

            CREATE TABLE IF NOT EXISTS pomodoro_phases (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                phase_type TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                cycle_number INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_pomodoro_phases_session
                ON pomodoro_phases(session_id);",
            ),
        ),
        (
            "v4: allowances and daily usage",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS allowances (
                id TEXT PRIMARY KEY,
                match_type TEXT NOT NULL,
                match_value TEXT NOT NULL,
                daily_limit_secs INTEGER NOT NULL,
                strict_mode INTEGER NOT NULL DEFAULT 1,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_allowances_value
                ON allowances(match_value);

            CREATE TABLE IF NOT EXISTS allowance_usage (
                allowance_id TEXT NOT NULL,
                usage_date TEXT NOT NULL,
                used_secs INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (allowance_id, usage_date)
            );

            CREATE INDEX IF NOT EXISTS idx_allowance_usage_date
                ON allowance_usage(usage_date);",
            ),
        ),
        (
            "v5: unlock_challenges for random-text locks",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS unlock_challenges (
                block_list_id TEXT PRIMARY KEY,
                challenge TEXT NOT NULL
            );",
            ),
        ),
        (
            "v6: shared scheduled allowance activity",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS shared_allowance_usage (
                block_list_id TEXT NOT NULL,
                occurrence_start INTEGER NOT NULL,
                intervals TEXT NOT NULL DEFAULT '[]',
                PRIMARY KEY (block_list_id, occurrence_start)
            );",
            ),
        ),
        (
            "v7: stable shared allowance occurrence anchors",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS shared_allowance_occurrences (
                block_list_id TEXT PRIMARY KEY,
                usage_start INTEGER NOT NULL,
                ends_at INTEGER NOT NULL
            );",
            ),
        ),
        (
            "v8: reclassify mistyped website rules (*word* wildcards, Domain values with a *)",
            Step::Code(reclassify_mistyped_website_rules),
        ),
        (
            "v9: focus sessions locked for a number of rounds",
            Step::Code(add_pomodoro_lock),
        ),
        (
            "v10: activity samples and visits",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS activity_samples (
                bucket_start INTEGER NOT NULL,
                source TEXT NOT NULL,
                target TEXT NOT NULL,
                device_id TEXT NOT NULL DEFAULT 'local',
                foreground_secs INTEGER NOT NULL DEFAULT 0,
                interaction_secs INTEGER NOT NULL DEFAULT 0,
                idle_secs INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (bucket_start, source, target, device_id)
            );

            CREATE INDEX IF NOT EXISTS idx_activity_samples_bucket
                ON activity_samples(bucket_start);
            CREATE INDEX IF NOT EXISTS idx_activity_samples_target
                ON activity_samples(target, bucket_start);

            CREATE TABLE IF NOT EXISTS activity_visits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                at INTEGER NOT NULL,
                source TEXT NOT NULL,
                target TEXT NOT NULL,
                device_id TEXT NOT NULL DEFAULT 'local',
                UNIQUE(at, source, target, device_id)
            );

            CREATE INDEX IF NOT EXISTS idx_activity_visits_at
                ON activity_visits(at);",
            ),
        ),
        (
            "v11: persistent unlock requests and attempts",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS unlock_requests (
                block_list_id TEXT PRIMARY KEY,
                challenge TEXT NOT NULL,
                requested_at TEXT NOT NULL,
                ready_at TEXT NOT NULL,
                wait_secs INTEGER NOT NULL,
                state TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS unlock_attempts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                block_list_id TEXT NOT NULL,
                at TEXT NOT NULL,
                ok INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_unlock_attempts_list
                ON unlock_attempts(block_list_id, at);",
            ),
        ),
        (
            "v12: local notifications",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS notifications (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                priority TEXT NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                created_at TEXT NOT NULL,
                read_at TEXT,
                dedupe_key TEXT
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_notifications_dedupe
                ON notifications(dedupe_key) WHERE dedupe_key IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_notifications_created
                ON notifications(created_at);",
            ),
        ),
        (
            "v13: goals, tasks and focus sessions",
            Step::Sql(
                "CREATE TABLE IF NOT EXISTS goals (
                id TEXT PRIMARY KEY,
                data TEXT NOT NULL,
                state TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_goals_state ON goals(state);

            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                data TEXT NOT NULL,
                status TEXT NOT NULL,
                goal_id TEXT,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_goal ON tasks(goal_id);

            CREATE TABLE IF NOT EXISTS focus_sessions (
                id TEXT PRIMARY KEY,
                data TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                goal_id TEXT,
                task_id TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_focus_sessions_started
                ON focus_sessions(started_at);",
            ),
        ),
        (
            "v14: task completion time",
            Step::Code(add_task_completed_at),
        ),
    ];

    for (i, (name, step)) in migrations.iter().enumerate() {
        let version = (i + 1) as i64;
        if version > current_version {
            info!("Running migration {version}: {name}");
            match step {
                Step::Sql(sql) => conn.execute_batch(sql).map_err(|e| {
                    FocuserError::Database(format!("Migration {version} failed: {e}"))
                })?,
                Step::Code(f) => f(conn).map_err(|e| {
                    FocuserError::Database(format!("Migration {version} failed: {e}"))
                })?,
            }
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                rusqlite::params![version],
            )
            .map_err(|e| FocuserError::Database(e.to_string()))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use focuser_common::types::{WebsiteMatchType, WebsiteRule};

    /// Just the `block_lists` table, as it looked before migration v8 —
    /// enough to exercise [`reclassify_mistyped_website_rules`] on its own,
    /// without going through `run_all` (which would apply it to an empty
    /// table and leave nothing to reclassify).
    fn conn_with_a_stored_list(list: &BlockList) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE block_lists (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                data TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO block_lists (id, name, data, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            rusqlite::params![
                list.id.to_string(),
                list.name,
                serde_json::to_string(list).unwrap(),
                list.enabled,
                list.created_at.to_rfc3339(),
            ],
        )
        .unwrap();
        conn
    }

    fn stored_match_type(
        conn: &Connection,
        id: focuser_common::types::EntityId,
    ) -> WebsiteMatchType {
        let json: String = conn
            .query_row(
                "SELECT data FROM block_lists WHERE id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_str::<BlockList>(&json).unwrap().websites[0]
            .match_type
            .clone()
    }

    #[test]
    fn a_star_word_star_wildcard_already_in_the_database_becomes_a_keyword() {
        let mut list = BlockList::new("Distractions");
        list.websites.push(WebsiteRule::wildcard("*casino*"));
        let conn = conn_with_a_stored_list(&list);

        reclassify_mistyped_website_rules(&conn).unwrap();

        assert_eq!(
            stored_match_type(&conn, list.id),
            WebsiteMatchType::Keyword("casino".into())
        );
    }

    #[test]
    fn a_real_glob_wildcard_is_left_alone() {
        let mut list = BlockList::new("Distractions");
        list.websites.push(WebsiteRule::wildcard("*.reddit.com"));
        let conn = conn_with_a_stored_list(&list);

        reclassify_mistyped_website_rules(&conn).unwrap();

        assert_eq!(
            stored_match_type(&conn, list.id),
            WebsiteMatchType::Wildcard("*.reddit.com".into())
        );
    }

    #[test]
    fn a_list_with_nothing_to_reclassify_is_not_rewritten() {
        let mut list = BlockList::new("Distractions");
        list.websites.push(WebsiteRule::domain("reddit.com"));
        let conn = conn_with_a_stored_list(&list);

        reclassify_mistyped_website_rules(&conn).unwrap();

        assert_eq!(
            stored_match_type(&conn, list.id),
            WebsiteMatchType::Domain("reddit.com".into())
        );
    }

    #[test]
    fn a_domain_typed_with_a_star_word_star_becomes_a_keyword() {
        let mut list = BlockList::new("Distractions");
        list.websites.push(WebsiteRule::domain("*casino*"));
        let conn = conn_with_a_stored_list(&list);

        reclassify_mistyped_website_rules(&conn).unwrap();

        assert_eq!(
            stored_match_type(&conn, list.id),
            WebsiteMatchType::Keyword("casino".into())
        );
    }

    #[test]
    fn a_domain_typed_with_a_real_glob_moves_to_wildcard() {
        let mut list = BlockList::new("Distractions");
        list.websites.push(WebsiteRule::domain("*free*games*"));
        let conn = conn_with_a_stored_list(&list);

        reclassify_mistyped_website_rules(&conn).unwrap();

        assert_eq!(
            stored_match_type(&conn, list.id),
            WebsiteMatchType::Wildcard("*free*games*".into())
        );
    }

    #[test]
    fn a_domain_too_short_to_safely_promote_is_left_exactly_as_broken_as_it_was() {
        let mut list = BlockList::new("Distractions");
        list.websites.push(WebsiteRule::domain("*r"));
        let conn = conn_with_a_stored_list(&list);

        reclassify_mistyped_website_rules(&conn).unwrap();

        assert_eq!(
            stored_match_type(&conn, list.id),
            WebsiteMatchType::Domain("*r".into())
        );
    }
}
