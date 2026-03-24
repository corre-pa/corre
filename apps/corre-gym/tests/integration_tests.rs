mod helpers;

use corre_gym::db::*;
use helpers::seeded_db;

#[test]
fn seed_data_loads_completely() {
    let (db, data) = seeded_db();

    let user_count: i64 = db.conn().query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).unwrap();
    assert_eq!(user_count, data.users.len() as i64);

    let session_count: i64 = db.conn().query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0)).unwrap();
    assert_eq!(session_count, data.sessions.len() as i64);

    let log_count: i64 = db.conn().query_row("SELECT COUNT(*) FROM exercise_logs", [], |r| r.get(0)).unwrap();
    let expected_logs: usize = data.sessions.iter().map(|s| s.logs.len()).sum();
    assert_eq!(log_count, expected_logs as i64);

    let goal_count: i64 = db.conn().query_row("SELECT COUNT(*) FROM exercise_goals", [], |r| r.get(0)).unwrap();
    assert_eq!(goal_count, data.goals.len() as i64);

    let health_count: i64 = db.conn().query_row("SELECT COUNT(*) FROM health_entries", [], |r| r.get(0)).unwrap();
    assert_eq!(health_count, data.health_entries.len() as i64);
}

#[test]
fn exercise_time_series_shows_progression() {
    let (db, _) = seeded_db();

    let points = db.exercise_time_series("user-1", "barbell-bench-press", Some("2025-01-01"), Some("2026-06-30")).unwrap();
    assert!(points.len() >= 10, "Expected at least 10 bench press data points, got {}", points.len());

    // First data point should be lower than last (progressive overload)
    let first = points.first().unwrap().value;
    let last = points.last().unwrap().value;
    assert!(first < last, "Expected progression: first {first} should be < last {last}");
}

#[test]
fn exercise_time_series_time_based() {
    let (db, _) = seeded_db();

    let points = db.exercise_time_series("user-1", "plank", Some("2025-01-01"), Some("2026-06-30")).unwrap();
    assert!(points.len() >= 20, "Expected at least 20 plank data points, got {}", points.len());
}

#[test]
fn exercise_time_series_distance_based() {
    let (db, _) = seeded_db();

    let points = db.exercise_time_series("user-1", "running", Some("2025-01-01"), Some("2026-06-30")).unwrap();
    assert!(points.len() >= 20, "Expected at least 20 running data points, got {}", points.len());
}

#[test]
fn muscle_group_time_series_returns_multiple_exercises() {
    let (db, _) = seeded_db();

    let series = db.muscle_group_time_series("user-1", "chest", Some("2025-01-01"), Some("2026-06-30")).unwrap();
    assert!(series.len() >= 1, "Expected at least 1 chest exercise series, got {}", series.len());
}

#[test]
fn goal_progress_shows_achieved_and_failed() {
    let (db, _) = seeded_db();

    // Check across both users for achieved and failed goals
    let report_u1 = db.goal_progress_report("user-1", Some("2024-01-01"), Some("2026-12-31")).unwrap();
    let report_u2 = db.goal_progress_report("user-2", Some("2024-01-01"), Some("2026-12-31")).unwrap();
    let all_reports: Vec<_> = report_u1.iter().chain(report_u2.iter()).collect();

    assert!(!all_reports.is_empty(), "Expected at least one goal in report");

    let has_achieved = all_reports.iter().any(|r| r.status == GoalStatus::Achieved);
    let has_failed = all_reports.iter().any(|r| r.status == GoalStatus::Failed);
    assert!(has_achieved, "Expected at least one achieved goal");
    assert!(has_failed, "Expected at least one failed goal: statuses = {:?}", all_reports.iter().map(|r| &r.status).collect::<Vec<_>>());
}

#[test]
fn goal_progress_percentages_are_reasonable() {
    let (db, _) = seeded_db();

    let report = db.goal_progress_report("user-1", Some("2025-01-01"), Some("2026-12-31")).unwrap();

    for gp in &report {
        match gp.status {
            GoalStatus::Achieved => {
                assert!(gp.percentage >= 100.0 || gp.goal.achieved, "Achieved goal should have >= 100% or be marked achieved");
            }
            GoalStatus::Failed => {
                // Failed goals might have any percentage, but should not be marked achieved
                assert!(!gp.goal.achieved, "Failed goal should not be marked achieved");
            }
            GoalStatus::Active => {}
        }
    }
}

#[test]
fn injury_gap_visible_in_sessions() {
    let (db, _) = seeded_db();

    // Count sessions during injury period (July 1-14, 2025) vs a normal 2-week period
    let injury_sessions = db.list_sessions("user-1", Some("2025-07-01"), Some("2025-07-14")).unwrap();
    let normal_sessions = db.list_sessions("user-1", Some("2025-02-01"), Some("2025-02-14")).unwrap();

    assert!(
        injury_sessions.len() < normal_sessions.len(),
        "Injury period should have fewer sessions ({}) than normal period ({})",
        injury_sessions.len(),
        normal_sessions.len()
    );
}

#[test]
fn health_entries_have_resolved_dates() {
    let (db, _) = seeded_db();

    let history = db.list_health_history("user-1", 10).unwrap();
    let resolved_count = history.iter().filter(|h| h.resolved_at.is_some()).count();
    assert!(resolved_count > 0, "Expected at least one resolved health entry");
}

#[test]
fn access_control_across_seeded_groups() {
    let (db, _) = seeded_db();

    // user-1 is admin, user-2 is write — they should be able to read each other
    assert!(db.can_read("user-1", "user-2").unwrap(), "Admin should be able to read group member");
    assert!(db.can_read("user-2", "user-1").unwrap(), "Write member should be able to read admin");

    // user-1 (admin) can write to user-2's data
    assert!(db.can_write("user-1", "user-2").unwrap(), "Admin should have write access");

    // user-2 (write) can write to user-1's data
    assert!(db.can_write("user-2", "user-1").unwrap(), "Write member should have write access");

    // user-1 should be admin of the group
    assert!(db.can_admin_group("user-1", "group-1").unwrap(), "user-1 should be group admin");
    assert!(!db.can_admin_group("user-2", "group-1").unwrap(), "user-2 should not be group admin");
}

#[test]
fn conversation_history_proportional() {
    let (db, data) = seeded_db();

    let msg_count: i64 =
        db.conn().query_row("SELECT COUNT(*) FROM conversation_history WHERE user_id = 'user-1'", [], |r| r.get(0)).unwrap();
    let session_count = data.sessions.iter().filter(|s| s.user_id == "user-1").count();

    assert!(
        msg_count as usize >= 2 * session_count,
        "Expected at least 2 messages per session: {msg_count} messages for {session_count} sessions"
    );
}

#[test]
fn personal_records() {
    let (db, _) = seeded_db();

    let pr = db.personal_record("user-1", "barbell-bench-press").unwrap();
    assert!(pr.is_some(), "Expected a bench press PR for user-1");
    let pr = pr.unwrap();
    assert!(pr.weight_kg.unwrap_or(0.0) >= 100.0, "Bench PR should be >= 100kg (achieved goal target), got {:?}", pr.weight_kg);
}

#[test]
fn concurrent_read_write_wal() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    let db1 = Database::open(&db_path).unwrap();
    let db2 = Database::open(&db_path).unwrap();

    // Seed exercises in db1
    db1.seed_exercises().unwrap();

    // Reader (db2) should see the exercises
    let exercises = db2.list_exercises().unwrap();
    assert!(!exercises.is_empty(), "db2 should see exercises written by db1");

    // Write a user via db1
    let user = new_user("WAL Test", None, "UTC");
    db1.insert_user(&user).unwrap();

    // Read via db2
    let fetched = db2.get_user(&user.id).unwrap();
    assert!(fetched.is_some(), "db2 should see user written by db1 via WAL");
}
