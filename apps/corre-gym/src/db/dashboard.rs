use anyhow::Context as _;
use chrono::NaiveDate;
use rusqlite::params;

use super::database::Database;
use super::models::{MuscleGroupWeeklyVolume, PersonalRecord, WeekSummary};

impl Database {
    /// Total volume (sets * reps * weight_kg) per muscle group per ISO week.
    pub fn volume_by_muscle_group_weekly(&self, user_id: &str, period: &str) -> anyhow::Result<Vec<MuscleGroupWeeklyVolume>> {
        let mut stmt = self.conn().prepare(
            "SELECT strftime('%G-W%V', el.logged_at) AS week, \
                    mg.name AS muscle_group, \
                    SUM(el.sets * el.reps * el.weight_kg) AS total_volume \
             FROM exercise_logs el \
             JOIN exercises e ON el.exercise_id = e.id \
             JOIN muscle_groups mg ON e.muscle_group_id = mg.id \
             WHERE el.user_id = ?1 \
               AND el.logged_at >= datetime('now', ?2) \
               AND el.sets IS NOT NULL AND el.reps IS NOT NULL AND el.weight_kg IS NOT NULL \
             GROUP BY week, mg.name \
             ORDER BY week, mg.name",
        )?;
        let rows = stmt.query_map(params![user_id, period], |row| {
            Ok(MuscleGroupWeeklyVolume { week: row.get(0)?, muscle_group: row.get(1)?, total_volume: row.get(2)? })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to query volume by muscle group weekly")
    }

    /// All-time personal records per exercise. One PR per exercise with the date it was achieved.
    pub fn personal_records(&self, user_id: &str) -> anyhow::Result<Vec<PersonalRecord>> {
        let mut stmt = self.conn().prepare(
            "SELECT e.id, e.name, mg.name, mt.name, pr.best_value, pr.logged_at \
             FROM ( \
                 SELECT el.exercise_id, \
                        CASE mt.name \
                            WHEN 'weight_reps' THEN el.weight_kg \
                            WHEN 'time_based' THEN CAST(el.duration_secs AS REAL) \
                            WHEN 'distance_based' THEN el.distance_m \
                            ELSE CAST(el.level AS REAL) \
                        END AS best_value, \
                        el.logged_at, \
                        ROW_NUMBER() OVER ( \
                            PARTITION BY el.exercise_id \
                            ORDER BY CASE mt.name \
                                WHEN 'weight_reps' THEN el.weight_kg \
                                WHEN 'time_based' THEN CAST(el.duration_secs AS REAL) \
                                WHEN 'distance_based' THEN el.distance_m \
                                ELSE CAST(el.level AS REAL) \
                            END DESC \
                        ) AS rn \
                 FROM exercise_logs el \
                 JOIN exercises e ON el.exercise_id = e.id \
                 JOIN measurement_types mt ON e.measurement_type_id = mt.id \
                 WHERE el.user_id = ?1 \
                   AND CASE mt.name \
                           WHEN 'weight_reps' THEN el.weight_kg IS NOT NULL \
                           WHEN 'time_based' THEN el.duration_secs IS NOT NULL \
                           WHEN 'distance_based' THEN el.distance_m IS NOT NULL \
                           ELSE el.level IS NOT NULL \
                       END \
             ) pr \
             JOIN exercises e ON pr.exercise_id = e.id \
             JOIN muscle_groups mg ON e.muscle_group_id = mg.id \
             JOIN measurement_types mt ON e.measurement_type_id = mt.id \
             WHERE pr.rn = 1 \
             ORDER BY mg.name, e.name",
        )?;
        let rows = stmt.query_map(params![user_id], |row| {
            Ok(PersonalRecord {
                exercise_id: row.get(0)?,
                exercise_name: row.get(1)?,
                muscle_group: row.get(2)?,
                measurement_type: row.get(3)?,
                value: row.get(4)?,
                achieved_at: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to query personal records")
    }

    /// Consecutive days with at least one completed session, counting backwards from today.
    /// Allows "yesterday" as the start if today has no session yet.
    pub fn workout_streak(&self, user_id: &str) -> anyhow::Result<i32> {
        let mut stmt = self.conn().prepare(
            "SELECT DISTINCT date(started_at) FROM sessions \
             WHERE user_id = ?1 AND ended_at IS NOT NULL \
             ORDER BY date(started_at) DESC \
             LIMIT 400",
        )?;
        let dates: Vec<String> =
            stmt.query_map(params![user_id], |row| row.get(0))?.collect::<Result<Vec<_>, _>>().context("Failed to query streak dates")?;

        let today = chrono::Utc::now().date_naive();
        let mut streak = 0i32;
        let mut expected = today;

        for date_str in &dates {
            let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")?;
            if date == expected {
                streak += 1;
                expected -= chrono::Duration::days(1);
            } else if streak == 0 && date == today - chrono::Duration::days(1) {
                // Allow starting from yesterday if today has no session
                streak += 1;
                expected = date - chrono::Duration::days(1);
            } else {
                break;
            }
        }
        Ok(streak)
    }

    /// Quick stats for the current ISO week: session count and total volume.
    pub fn week_summary(&self, user_id: &str) -> anyhow::Result<WeekSummary> {
        let row: (i32, f64) = self.conn().query_row(
            "SELECT COUNT(DISTINCT s.id), \
                    COALESCE(SUM( \
                        CASE WHEN el.sets IS NOT NULL AND el.reps IS NOT NULL AND el.weight_kg IS NOT NULL \
                             THEN el.sets * el.reps * el.weight_kg ELSE 0 END \
                    ), 0) \
             FROM sessions s \
             LEFT JOIN exercise_logs el ON el.session_id = s.id \
             WHERE s.user_id = ?1 \
               AND s.started_at >= date('now', 'weekday 1', '-7 days') \
               AND s.started_at < date('now', 'weekday 1')",
            params![user_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(WeekSummary { session_count: row.0, total_volume: row.1 })
    }

    /// Check if a user is a member of a specific group (any level).
    pub fn is_group_member(&self, user_id: &str, group_id: &str) -> anyhow::Result<bool> {
        let mut stmt = self.conn().prepare("SELECT 1 FROM group_members WHERE user_id = ?1 AND group_id = ?2 LIMIT 1")?;
        let exists = stmt.query_map(params![user_id, group_id], |row| row.get::<_, i32>(0))?.next().is_some();
        Ok(exists)
    }

    /// Get exercise logs for a user with pagination support.
    /// Returns (logs, total_count).
    pub fn get_logs_paginated(
        &self,
        user_id: &str,
        from: Option<&str>,
        to: Option<&str>,
        exercise_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<(Vec<super::models::ExerciseLog>, i64)> {
        let count: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM exercise_logs \
             WHERE user_id = ?1 \
               AND (?2 IS NULL OR logged_at >= ?2) \
               AND (?3 IS NULL OR logged_at <= ?3) \
               AND (?4 IS NULL OR exercise_id = ?4)",
            params![user_id, from, to, exercise_id],
            |row| row.get(0),
        )?;

        let mut stmt = self.conn().prepare(
            "SELECT id, user_id, exercise_id, session_id, logged_at, \
                    sets, reps, weight_kg, duration_secs, distance_m, level, difficulty, notes \
             FROM exercise_logs \
             WHERE user_id = ?1 \
               AND (?2 IS NULL OR logged_at >= ?2) \
               AND (?3 IS NULL OR logged_at <= ?3) \
               AND (?4 IS NULL OR exercise_id = ?4) \
             ORDER BY logged_at DESC \
             LIMIT ?5 OFFSET ?6",
        )?;
        let rows = stmt.query_map(params![user_id, from, to, exercise_id, limit, offset], |row| {
            Ok(super::models::ExerciseLog {
                id: row.get(0)?,
                user_id: row.get(1)?,
                exercise_id: row.get(2)?,
                session_id: row.get(3)?,
                logged_at: row.get(4)?,
                sets: row.get(5)?,
                reps: row.get(6)?,
                weight_kg: row.get(7)?,
                duration_secs: row.get(8)?,
                distance_m: row.get(9)?,
                level: row.get(10)?,
                difficulty: super::models::Difficulty::from_str_loose(&row.get::<_, String>(11)?),
                notes: row.get(12)?,
            })
        })?;
        let logs = rows.collect::<Result<Vec<_>, _>>().context("Failed to get paginated logs")?;
        Ok((logs, count))
    }
}

#[cfg(test)]
mod tests {
    use super::super::models::{new_exercise_log, new_user};
    use super::*;

    fn test_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.seed_exercises().unwrap();
        db
    }

    fn setup_user_and_exercise(db: &Database) -> (String, String) {
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises().unwrap();
        (user.id, exercises[0].id.clone())
    }

    #[test]
    fn volume_by_muscle_group_weekly_aggregates() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);

        for day in [1, 2, 3] {
            let mut log = new_exercise_log(&user_id, &ex_id, None);
            log.logged_at = format!("2025-06-{day:02} 10:00:00");
            log.sets = Some(3);
            log.reps = Some(10);
            log.weight_kg = Some(80.0);
            db.insert_log(&log).unwrap();
        }

        let volumes = db.volume_by_muscle_group_weekly(&user_id, "-365 days").unwrap();
        assert!(!volumes.is_empty());
        // 3 days * 3 sets * 10 reps * 80kg = 7200 total
        let total: f64 = volumes.iter().map(|v| v.total_volume).sum();
        assert!((total - 7200.0).abs() < 0.01);
    }

    #[test]
    fn personal_records_returns_one_per_exercise() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);

        for weight in [60.0, 80.0, 70.0] {
            let mut log = new_exercise_log(&user_id, &ex_id, None);
            log.sets = Some(3);
            log.reps = Some(5);
            log.weight_kg = Some(weight);
            db.insert_log(&log).unwrap();
        }

        let prs = db.personal_records(&user_id).unwrap();
        let pr = prs.iter().find(|p| p.exercise_id == ex_id).unwrap();
        assert!((pr.value - 80.0).abs() < 0.01);
    }

    #[test]
    fn workout_streak_consecutive_days() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();

        let today = chrono::Utc::now().date_naive();
        for offset in 0..3 {
            let date = today - chrono::Duration::days(offset);
            let started = format!("{date} 10:00:00");
            let ended = format!("{date} 11:00:00");
            db.conn()
                .execute(
                    "INSERT INTO sessions (id, user_id, started_at, ended_at) VALUES (?1, ?2, ?3, ?4)",
                    params![uuid::Uuid::new_v4().to_string(), user.id, started, ended],
                )
                .unwrap();
        }

        assert_eq!(db.workout_streak(&user.id).unwrap(), 3);
    }

    #[test]
    fn workout_streak_with_gap_resets() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();

        let today = chrono::Utc::now().date_naive();
        // Today and 2 days ago (gap yesterday)
        for offset in [0, 2] {
            let date = today - chrono::Duration::days(offset);
            let started = format!("{date} 10:00:00");
            let ended = format!("{date} 11:00:00");
            db.conn()
                .execute(
                    "INSERT INTO sessions (id, user_id, started_at, ended_at) VALUES (?1, ?2, ?3, ?4)",
                    params![uuid::Uuid::new_v4().to_string(), user.id, started, ended],
                )
                .unwrap();
        }

        assert_eq!(db.workout_streak(&user.id).unwrap(), 1);
    }

    #[test]
    fn workout_streak_starting_yesterday() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();

        let today = chrono::Utc::now().date_naive();
        // Yesterday and day before, but not today
        for offset in [1, 2] {
            let date = today - chrono::Duration::days(offset);
            let started = format!("{date} 10:00:00");
            let ended = format!("{date} 11:00:00");
            db.conn()
                .execute(
                    "INSERT INTO sessions (id, user_id, started_at, ended_at) VALUES (?1, ?2, ?3, ?4)",
                    params![uuid::Uuid::new_v4().to_string(), user.id, started, ended],
                )
                .unwrap();
        }

        assert_eq!(db.workout_streak(&user.id).unwrap(), 2);
    }

    #[test]
    fn week_summary_counts_correctly() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);

        let session = db.start_session(&user_id, None).unwrap();
        let mut log = new_exercise_log(&user_id, &ex_id, Some(&session.id));
        log.sets = Some(3);
        log.reps = Some(10);
        log.weight_kg = Some(60.0);
        db.insert_log(&log).unwrap();

        let summary = db.week_summary(&user_id).unwrap();
        // The session was just created "now", so it should be in the current week
        assert!(summary.session_count >= 1);
    }

    #[test]
    fn get_logs_paginated_returns_correct_page() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);

        for i in 0..5 {
            let mut log = new_exercise_log(&user_id, &ex_id, None);
            log.logged_at = format!("2025-06-{:02} 10:00:00", i + 1);
            log.sets = Some(3);
            log.reps = Some(10);
            log.weight_kg = Some(60.0 + i as f64);
            db.insert_log(&log).unwrap();
        }

        let (logs, total) = db.get_logs_paginated(&user_id, None, None, None, 2, 0).unwrap();
        assert_eq!(total, 5);
        assert_eq!(logs.len(), 2);

        let (logs, total) = db.get_logs_paginated(&user_id, None, None, None, 2, 3).unwrap();
        assert_eq!(total, 5);
        assert_eq!(logs.len(), 2);
    }

    #[test]
    fn is_group_member_works() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let group = super::super::models::Group {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Test Group".to_string(),
            description: None,
            created_at: "2025-01-01 00:00:00".into(),
        };
        db.insert_group(&group).unwrap();
        db.add_member(&user.id, &group.id, super::super::models::AccessLevel::Read).unwrap();
        assert!(db.is_group_member(&user.id, &group.id).unwrap());
        assert!(!db.is_group_member(&user.id, "nonexistent").unwrap());
    }
}
