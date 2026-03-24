use anyhow::Context as _;
use rusqlite::params;

use super::database::Database;
use super::models::{Difficulty, ExerciseLog, MeasurementType, Session, SessionSummary, new_session};

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<Session> {
    Ok(Session { id: row.get(0)?, user_id: row.get(1)?, started_at: row.get(2)?, ended_at: row.get(3)?, notes: row.get(4)? })
}

fn row_to_log(row: &rusqlite::Row) -> rusqlite::Result<ExerciseLog> {
    Ok(ExerciseLog {
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
        difficulty: Difficulty::from_str_loose(&row.get::<_, String>(11)?),
        notes: row.get(12)?,
    })
}

const SELECT_LOG: &str = "\
    SELECT id, user_id, exercise_id, session_id, logged_at, \
           sets, reps, weight_kg, duration_secs, distance_m, level, difficulty, notes \
    FROM exercise_logs";

impl Database {
    // ── Sessions ───────────────────────────────────────────────────────────────

    pub fn start_session(&self, user_id: &str, notes: Option<&str>) -> anyhow::Result<Session> {
        let session = new_session(user_id, notes);
        self.conn().execute(
            "INSERT INTO sessions (id, user_id, started_at, notes) VALUES (?1, ?2, ?3, ?4)",
            params![session.id, session.user_id, session.started_at, session.notes],
        )?;
        tracing::debug!(id = %session.id, user_id = %user_id, "DB: inserted session");
        Ok(session)
    }

    pub fn end_session(&self, session_id: &str) -> anyhow::Result<()> {
        let rows = self.conn().execute("UPDATE sessions SET ended_at = datetime('now') WHERE id = ?1", params![session_id])?;
        anyhow::ensure!(rows > 0, "Session with id {session_id} not found");
        tracing::debug!(id = %session_id, "DB: ended session");
        Ok(())
    }

    pub fn get_session(&self, id: &str) -> anyhow::Result<Option<Session>> {
        let mut stmt = self.conn().prepare("SELECT id, user_id, started_at, ended_at, notes FROM sessions WHERE id = ?1")?;
        let mut rows = stmt.query_map(params![id], row_to_session)?;
        rows.next().transpose().context("Failed to read session row")
    }

    pub fn get_active_session(&self, user_id: &str) -> anyhow::Result<Option<Session>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, user_id, started_at, ended_at, notes FROM sessions \
             WHERE user_id = ?1 AND ended_at IS NULL ORDER BY started_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![user_id], row_to_session)?;
        rows.next().transpose().context("Failed to read session row")
    }

    pub fn list_sessions(&self, user_id: &str, from: Option<&str>, to: Option<&str>) -> anyhow::Result<Vec<Session>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, user_id, started_at, ended_at, notes FROM sessions \
             WHERE user_id = ?1 AND (?2 IS NULL OR started_at >= ?2) AND (?3 IS NULL OR started_at <= ?3) \
             ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id, from, to], row_to_session)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list sessions")
    }

    pub fn list_session_summaries(&self, user_id: &str, from: Option<&str>, to: Option<&str>) -> anyhow::Result<Vec<SessionSummary>> {
        let mut stmt = self.conn().prepare(
            "SELECT s.id, s.user_id, s.started_at, s.ended_at, s.notes, \
                    COUNT(DISTINCT el.id) AS exercise_count, \
                    CAST((julianday(s.ended_at) - julianday(s.started_at)) * 24 * 60 AS INTEGER) AS duration_mins \
             FROM sessions s \
             LEFT JOIN exercise_logs el ON el.session_id = s.id \
             WHERE s.user_id = ?1 \
               AND (?2 IS NULL OR s.started_at >= ?2) \
               AND (?3 IS NULL OR s.started_at <= ?3) \
             GROUP BY s.id \
             ORDER BY s.started_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id, from, to], |row| {
            let session = row_to_session(row)?;
            Ok(SessionSummary { session, exercise_count: row.get(5)?, duration_mins: row.get(6)? })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list session summaries")
    }

    // ── Exercise logs ──────────────────────────────────────────────────────────

    pub fn insert_log(&self, log: &ExerciseLog) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO exercise_logs (id, user_id, exercise_id, session_id, logged_at, \
             sets, reps, weight_kg, duration_secs, distance_m, level, difficulty, notes) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                log.id,
                log.user_id,
                log.exercise_id,
                log.session_id,
                log.logged_at,
                log.sets,
                log.reps,
                log.weight_kg,
                log.duration_secs,
                log.distance_m,
                log.level,
                log.difficulty.as_str(),
                log.notes,
            ],
        )?;
        tracing::debug!(id = %log.id, exercise_id = %log.exercise_id, session_id = ?log.session_id, "DB: inserted exercise log");
        Ok(())
    }

    pub fn update_log(&self, log: &ExerciseLog) -> anyhow::Result<()> {
        let rows = self.conn().execute(
            "UPDATE exercise_logs SET exercise_id = ?1, session_id = ?2, logged_at = ?3, \
             sets = ?4, reps = ?5, weight_kg = ?6, duration_secs = ?7, distance_m = ?8, \
             level = ?9, difficulty = ?10, notes = ?11 WHERE id = ?12",
            params![
                log.exercise_id,
                log.session_id,
                log.logged_at,
                log.sets,
                log.reps,
                log.weight_kg,
                log.duration_secs,
                log.distance_m,
                log.level,
                log.difficulty.as_str(),
                log.notes,
                log.id,
            ],
        )?;
        anyhow::ensure!(rows > 0, "Exercise log with id {} not found", log.id);
        tracing::debug!(id = %log.id, "DB: updated exercise log");
        Ok(())
    }

    pub fn get_logs_for_session(&self, session_id: &str) -> anyhow::Result<Vec<ExerciseLog>> {
        let sql = format!("{SELECT_LOG} WHERE session_id = ?1 ORDER BY logged_at");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![session_id], row_to_log)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to get logs for session")
    }

    pub fn get_logs_for_user(&self, user_id: &str, from: Option<&str>, to: Option<&str>) -> anyhow::Result<Vec<ExerciseLog>> {
        let sql = format!(
            "{SELECT_LOG} WHERE user_id = ?1 AND (?2 IS NULL OR logged_at >= ?2) AND (?3 IS NULL OR logged_at <= ?3) \
             ORDER BY logged_at DESC"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, from, to], row_to_log)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to get logs for user")
    }

    pub fn get_logs_for_exercise(&self, user_id: &str, exercise_id: &str, limit: usize) -> anyhow::Result<Vec<ExerciseLog>> {
        let sql = format!("{SELECT_LOG} WHERE user_id = ?1 AND exercise_id = ?2 ORDER BY logged_at DESC LIMIT ?3");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, exercise_id, limit as i64], row_to_log)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to get logs for exercise")
    }

    pub fn get_recent_logs(&self, user_id: &str, days: i32) -> anyhow::Result<Vec<ExerciseLog>> {
        let sql = format!("{SELECT_LOG} WHERE user_id = ?1 AND logged_at >= datetime('now', ?2) ORDER BY logged_at DESC");
        let modifier = format!("-{days} days");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, modifier], row_to_log)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to get recent logs")
    }

    pub fn delete_log(&self, id: &str) -> anyhow::Result<()> {
        let rows = self.conn().execute("DELETE FROM exercise_logs WHERE id = ?1", params![id])?;
        anyhow::ensure!(rows > 0, "Exercise log with id {id} not found");
        Ok(())
    }

    // ── Aggregations ───────────────────────────────────────────────────────────

    pub fn personal_record(&self, user_id: &str, exercise_id: &str) -> anyhow::Result<Option<ExerciseLog>> {
        // Determine measurement type for this exercise
        let mt: Option<String> = self
            .conn()
            .query_row(
                "SELECT mt.name FROM exercises e JOIN measurement_types mt ON e.measurement_type_id = mt.id WHERE e.id = ?1",
                params![exercise_id],
                |row| row.get(0),
            )
            .optional()
            .context("Failed to look up exercise measurement type")?;

        let mt = match mt {
            Some(m) => MeasurementType::from_str_loose(&m),
            None => return Ok(None),
        };

        let order_clause = match mt {
            MeasurementType::WeightReps => "weight_kg DESC NULLS LAST",
            MeasurementType::TimeBased => "duration_secs DESC NULLS LAST",
            MeasurementType::DistanceBased => "distance_m DESC NULLS LAST",
            MeasurementType::LevelBased | MeasurementType::ScoreBased => "level DESC NULLS LAST",
        };

        let sql = format!("{SELECT_LOG} WHERE user_id = ?1 AND exercise_id = ?2 ORDER BY {order_clause} LIMIT 1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![user_id, exercise_id], row_to_log)?;
        rows.next().transpose().context("Failed to read personal record")
    }

    pub fn session_count_by_week(&self, user_id: &str, weeks: i32) -> anyhow::Result<Vec<(String, i32)>> {
        let modifier = format!("-{} days", weeks * 7);
        let mut stmt = self.conn().prepare(
            "SELECT strftime('%Y-W%W', started_at) AS week, COUNT(*) AS cnt \
             FROM sessions \
             WHERE user_id = ?1 AND started_at >= datetime('now', ?2) \
             GROUP BY week ORDER BY week",
        )?;
        let rows = stmt.query_map(params![user_id, modifier], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to count sessions by week")
    }
}

use rusqlite::OptionalExtension as _;

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
    fn start_and_end_session() {
        let db = test_db();
        let (user_id, _) = setup_user_and_exercise(&db);

        let session = db.start_session(&user_id, Some("Leg day")).unwrap();
        assert!(session.ended_at.is_none());

        db.end_session(&session.id).unwrap();
        let fetched = db.get_session(&session.id).unwrap().unwrap();
        assert!(fetched.ended_at.is_some());
    }

    #[test]
    fn get_session_by_id() {
        let db = test_db();
        let (user_id, _) = setup_user_and_exercise(&db);
        let session = db.start_session(&user_id, None).unwrap();

        let fetched = db.get_session(&session.id).unwrap().unwrap();
        assert_eq!(fetched.user_id, user_id);
    }

    #[test]
    fn insert_log_in_session() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);
        let session = db.start_session(&user_id, None).unwrap();

        let mut log = new_exercise_log(&user_id, &ex_id, Some(&session.id));
        log.sets = Some(4);
        log.reps = Some(8);
        log.weight_kg = Some(80.0);
        db.insert_log(&log).unwrap();

        let logs = db.get_logs_for_session(&session.id).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].weight_kg, Some(80.0));
    }

    #[test]
    fn update_log() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);

        let mut log = new_exercise_log(&user_id, &ex_id, None);
        log.sets = Some(3);
        log.reps = Some(10);
        log.weight_kg = Some(60.0);
        db.insert_log(&log).unwrap();

        log.weight_kg = Some(65.0);
        log.difficulty = Difficulty::Hard;
        db.update_log(&log).unwrap();

        let logs = db.get_logs_for_exercise(&user_id, &ex_id, 1).unwrap();
        assert_eq!(logs[0].weight_kg, Some(65.0));
        assert_eq!(logs[0].difficulty, Difficulty::Hard);
    }

    #[test]
    fn get_logs_by_date_range() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);

        let mut log = new_exercise_log(&user_id, &ex_id, None);
        log.sets = Some(3);
        log.reps = Some(10);
        log.weight_kg = Some(60.0);
        db.insert_log(&log).unwrap();

        let logs = db.get_logs_for_user(&user_id, Some("2020-01-01"), None).unwrap();
        assert_eq!(logs.len(), 1);

        let logs = db.get_logs_for_user(&user_id, Some("2099-01-01"), None).unwrap();
        assert!(logs.is_empty());
    }

    #[test]
    fn get_logs_by_exercise() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);

        for w in [60.0, 65.0, 70.0] {
            let mut log = new_exercise_log(&user_id, &ex_id, None);
            log.sets = Some(3);
            log.reps = Some(10);
            log.weight_kg = Some(w);
            db.insert_log(&log).unwrap();
        }

        let logs = db.get_logs_for_exercise(&user_id, &ex_id, 2).unwrap();
        assert_eq!(logs.len(), 2);
    }

    #[test]
    fn personal_record() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);

        for w in [60.0, 80.0, 70.0] {
            let mut log = new_exercise_log(&user_id, &ex_id, None);
            log.sets = Some(3);
            log.reps = Some(5);
            log.weight_kg = Some(w);
            db.insert_log(&log).unwrap();
        }

        let pr = db.personal_record(&user_id, &ex_id).unwrap().unwrap();
        assert_eq!(pr.weight_kg, Some(80.0));
    }

    #[test]
    fn session_count_by_week() {
        let db = test_db();
        let (user_id, _) = setup_user_and_exercise(&db);

        db.start_session(&user_id, None).unwrap();
        db.start_session(&user_id, None).unwrap();

        // Use a wide window to catch sessions regardless of timezone differences
        let counts = db.session_count_by_week(&user_id, 52).unwrap();
        assert!(!counts.is_empty());
        let total: i32 = counts.iter().map(|(_, c)| c).sum();
        assert_eq!(total, 2);
    }

    #[test]
    fn list_session_summaries() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);

        let session = db.start_session(&user_id, None).unwrap();
        let mut log = new_exercise_log(&user_id, &ex_id, Some(&session.id));
        log.sets = Some(3);
        log.reps = Some(10);
        log.weight_kg = Some(60.0);
        db.insert_log(&log).unwrap();

        let summaries = db.list_session_summaries(&user_id, None, None).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].exercise_count, 1);
    }

    #[test]
    fn session_delete_cascades_logs() {
        let db = test_db();
        let (user_id, ex_id) = setup_user_and_exercise(&db);

        let session = db.start_session(&user_id, None).unwrap();
        let mut log = new_exercise_log(&user_id, &ex_id, Some(&session.id));
        log.sets = Some(3);
        log.reps = Some(10);
        log.weight_kg = Some(60.0);
        db.insert_log(&log).unwrap();

        // Delete session by deleting directly
        self::super::super::database::Database::conn(&db).execute("DELETE FROM sessions WHERE id = ?1", params![session.id]).unwrap();

        // Log should be cascade-deleted
        let logs = db.get_logs_for_exercise(&user_id, &ex_id, 10).unwrap();
        assert!(logs.is_empty());
    }
}
