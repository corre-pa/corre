use anyhow::Context as _;
use rusqlite::{Row, params};

use super::database::Database;
use super::models::{Difficulty, ExerciseEntry, ExerciseSet, MeasurementType, Session, SessionSummary};

// ── Sessions ───────────────────────────────────────────────────────────────────

fn row_to_session(row: &Row) -> rusqlite::Result<Session> {
    Ok(Session { id: row.get(0)?, user_id: row.get(1)?, started_at: row.get(2)?, ended_at: row.get(3)?, notes: row.get(4)? })
}

const SELECT_SESSION: &str = "SELECT id, user_id, started_at, ended_at, notes FROM sessions";

impl Database {
    pub fn start_session(&self, user_id: i64, notes: Option<&str>) -> anyhow::Result<Session> {
        self.conn().execute("INSERT INTO sessions (user_id, notes) VALUES (?1, ?2)", params![user_id, notes])?;
        let id = self.conn().last_insert_rowid();
        let session =
            self.get_session(id)?.context("Session disappeared immediately after insert")?;
        Ok(session)
    }

    pub fn end_session(&self, session_id: i64) -> anyhow::Result<()> {
        let rows = self
            .conn()
            .execute("UPDATE sessions SET ended_at = datetime('now') WHERE id = ?1 AND ended_at IS NULL", params![session_id])?;
        anyhow::ensure!(rows > 0, "session id {session_id} not found or already ended");
        Ok(())
    }

    pub fn get_session(&self, id: i64) -> anyhow::Result<Option<Session>> {
        let sql = format!("{SELECT_SESSION} WHERE id = ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![id], row_to_session)?;
        rows.next().transpose().context("Failed to read session row")
    }

    pub fn get_active_session(&self, user_id: i64) -> anyhow::Result<Option<Session>> {
        let sql = format!("{SELECT_SESSION} WHERE user_id = ?1 AND ended_at IS NULL ORDER BY started_at DESC LIMIT 1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![user_id], row_to_session)?;
        rows.next().transpose().context("Failed to read active session")
    }

    pub fn list_sessions(&self, user_id: i64, from: Option<&str>, to: Option<&str>) -> anyhow::Result<Vec<Session>> {
        let sql = format!(
            "{SELECT_SESSION} \
             WHERE user_id = ?1 \
               AND (?2 IS NULL OR started_at >= ?2) \
               AND (?3 IS NULL OR started_at <= ?3) \
             ORDER BY started_at DESC"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, from, to], row_to_session)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list sessions")
    }

    pub fn list_session_summaries(&self, user_id: i64, from: Option<&str>, to: Option<&str>) -> anyhow::Result<Vec<SessionSummary>> {
        let mut stmt = self.conn().prepare(
            "SELECT s.id, s.user_id, s.started_at, s.ended_at, s.notes, \
                    COUNT(DISTINCT ee.id) AS exercise_count, \
                    CASE WHEN s.ended_at IS NULL THEN NULL \
                         ELSE CAST((julianday(s.ended_at) - julianday(s.started_at)) * 24 * 60 AS INTEGER) \
                    END AS duration_mins \
             FROM sessions s \
             LEFT JOIN exercise_entry ee ON ee.session_id = s.id \
             WHERE s.user_id = ?1 \
               AND (?2 IS NULL OR s.started_at >= ?2) \
               AND (?3 IS NULL OR s.started_at <= ?3) \
             GROUP BY s.id \
             ORDER BY s.started_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id, from, to], |row| {
            let session = Session {
                id: row.get(0)?,
                user_id: row.get(1)?,
                started_at: row.get(2)?,
                ended_at: row.get(3)?,
                notes: row.get(4)?,
            };
            Ok(SessionSummary { session, exercise_count: row.get(5)?, duration_mins: row.get(6)? })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list session summaries")
    }

    pub fn session_count_by_week(&self, user_id: i64, weeks: i32) -> anyhow::Result<Vec<(String, i32)>> {
        let mut stmt = self.conn().prepare(
            "SELECT strftime('%G-W%V', started_at) AS week, COUNT(*) \
             FROM sessions \
             WHERE user_id = ?1 \
               AND started_at >= datetime('now', ?2) \
             GROUP BY week \
             ORDER BY week",
        )?;
        let period = format!("-{weeks} days", weeks = weeks * 7);
        let rows = stmt.query_map(params![user_id, period], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to query session count by week")
    }
}

// ── Exercise entries ───────────────────────────────────────────────────────────

fn row_to_entry(row: &Row) -> rusqlite::Result<ExerciseEntry> {
    Ok(ExerciseEntry {
        id: row.get(0)?,
        user_id: row.get(1)?,
        session_id: row.get(2)?,
        start_timestamp: row.get(3)?,
        end_timestamp: row.get(4)?,
        comment: row.get(5)?,
    })
}

const SELECT_ENTRY: &str = "SELECT id, user_id, session_id, start_timestamp, end_timestamp, comment FROM exercise_entry";

impl Database {
    pub fn insert_entry(&self, entry: &ExerciseEntry) -> anyhow::Result<i64> {
        self.conn().execute(
            "INSERT INTO exercise_entry (user_id, session_id, start_timestamp, end_timestamp, comment) \
             VALUES (?1, ?2, COALESCE(?3, datetime('now')), ?4, ?5)",
            params![
                entry.user_id,
                entry.session_id,
                if entry.start_timestamp.is_empty() { None } else { Some(&entry.start_timestamp) },
                entry.end_timestamp,
                entry.comment,
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn end_entry(&self, entry_id: i64) -> anyhow::Result<()> {
        let rows = self.conn().execute(
            "UPDATE exercise_entry SET end_timestamp = datetime('now') WHERE id = ?1 AND end_timestamp IS NULL",
            params![entry_id],
        )?;
        anyhow::ensure!(rows > 0, "exercise_entry id {entry_id} not found or already closed");
        Ok(())
    }

    pub fn get_entry(&self, id: i64) -> anyhow::Result<Option<ExerciseEntry>> {
        let sql = format!("{SELECT_ENTRY} WHERE id = ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![id], row_to_entry)?;
        rows.next().transpose().context("Failed to read exercise_entry")
    }

    pub fn list_entries_for_session(&self, session_id: i64) -> anyhow::Result<Vec<ExerciseEntry>> {
        let sql = format!("{SELECT_ENTRY} WHERE session_id = ?1 ORDER BY start_timestamp");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![session_id], row_to_entry)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list entries for session")
    }

    pub fn list_entries_for_user(&self, user_id: i64, from: Option<&str>, to: Option<&str>) -> anyhow::Result<Vec<ExerciseEntry>> {
        let sql = format!(
            "{SELECT_ENTRY} \
             WHERE user_id = ?1 \
               AND (?2 IS NULL OR start_timestamp >= ?2) \
               AND (?3 IS NULL OR start_timestamp <= ?3) \
             ORDER BY start_timestamp DESC"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, from, to], row_to_entry)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list entries for user")
    }

    /// Most recent open entry for the user (no `end_timestamp`), within the last
    /// `recent_minutes` minutes. Returns `None` if no recent open entry exists.
    pub fn latest_open_entry(&self, user_id: i64, recent_minutes: i64) -> anyhow::Result<Option<ExerciseEntry>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, user_id, session_id, start_timestamp, end_timestamp, comment \
             FROM exercise_entry \
             WHERE user_id = ?1 \
               AND end_timestamp IS NULL \
               AND start_timestamp >= datetime('now', ?2) \
             ORDER BY start_timestamp DESC LIMIT 1",
        )?;
        let cutoff = format!("-{recent_minutes} minutes");
        let mut rows = stmt.query_map(params![user_id, cutoff], row_to_entry)?;
        rows.next().transpose().context("Failed to read latest open entry")
    }

    pub fn delete_entry(&self, entry_id: i64) -> anyhow::Result<()> {
        let rows = self.conn().execute("DELETE FROM exercise_entry WHERE id = ?1", params![entry_id])?;
        anyhow::ensure!(rows > 0, "exercise_entry id {entry_id} not found");
        Ok(())
    }
}

// ── Sets ───────────────────────────────────────────────────────────────────────

fn row_to_set(row: &Row) -> rusqlite::Result<ExerciseSet> {
    Ok(ExerciseSet {
        id: row.get(0)?,
        exercise_entry_id: row.get(1)?,
        exercise_type_id: row.get(2)?,
        order_idx: row.get(3)?,
        measurement_type: MeasurementType::from_id(row.get::<_, i64>(4)?),
        count: row.get(5)?,
        value: row.get(6)?,
        perceived_difficulty: Difficulty::from_str_loose(&row.get::<_, String>(7)?),
        comment: row.get(8)?,
        logged_at: row.get(9)?,
    })
}

const SELECT_SET: &str = "\
    SELECT id, exercise_entry_id, exercise_type_id, order_idx, \
           measurement_type_id, count, value, perceived_difficulty, comment, logged_at \
    FROM sets";

impl Database {
    pub fn insert_set(&self, set: &ExerciseSet) -> anyhow::Result<i64> {
        self.conn().execute(
            "INSERT INTO sets (exercise_entry_id, exercise_type_id, order_idx, measurement_type_id, \
                               count, value, perceived_difficulty, comment, logged_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, COALESCE(?9, datetime('now')))",
            params![
                set.exercise_entry_id,
                set.exercise_type_id,
                set.order_idx,
                set.measurement_type.id(),
                set.count,
                set.value,
                set.perceived_difficulty.as_str(),
                set.comment,
                if set.logged_at.is_empty() { None } else { Some(&set.logged_at) },
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn update_set(&self, set: &ExerciseSet) -> anyhow::Result<()> {
        let rows = self.conn().execute(
            "UPDATE sets SET exercise_entry_id = ?1, exercise_type_id = ?2, order_idx = ?3, \
                              measurement_type_id = ?4, count = ?5, value = ?6, \
                              perceived_difficulty = ?7, comment = ?8, logged_at = ?9 \
             WHERE id = ?10",
            params![
                set.exercise_entry_id,
                set.exercise_type_id,
                set.order_idx,
                set.measurement_type.id(),
                set.count,
                set.value,
                set.perceived_difficulty.as_str(),
                set.comment,
                set.logged_at,
                set.id,
            ],
        )?;
        anyhow::ensure!(rows > 0, "set id {} not found", set.id);
        Ok(())
    }

    pub fn delete_set(&self, id: i64) -> anyhow::Result<()> {
        let rows = self.conn().execute("DELETE FROM sets WHERE id = ?1", params![id])?;
        anyhow::ensure!(rows > 0, "set id {id} not found");
        Ok(())
    }

    pub fn get_set(&self, id: i64) -> anyhow::Result<Option<ExerciseSet>> {
        let sql = format!("{SELECT_SET} WHERE id = ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![id], row_to_set)?;
        rows.next().transpose().context("Failed to read set row")
    }

    pub fn list_sets_for_entry(&self, entry_id: i64) -> anyhow::Result<Vec<ExerciseSet>> {
        let sql = format!("{SELECT_SET} WHERE exercise_entry_id = ?1 ORDER BY order_idx, id");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![entry_id], row_to_set)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list sets for entry")
    }

    pub fn list_sets_for_user(&self, user_id: i64, from: Option<&str>, to: Option<&str>) -> anyhow::Result<Vec<ExerciseSet>> {
        let mut stmt = self.conn().prepare(
            "SELECT s.id, s.exercise_entry_id, s.exercise_type_id, s.order_idx, \
                    s.measurement_type_id, s.count, s.value, s.perceived_difficulty, s.comment, s.logged_at \
             FROM sets s \
             JOIN exercise_entry ee ON s.exercise_entry_id = ee.id \
             WHERE ee.user_id = ?1 \
               AND (?2 IS NULL OR s.logged_at >= ?2) \
               AND (?3 IS NULL OR s.logged_at <= ?3) \
             ORDER BY s.logged_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id, from, to], row_to_set)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list sets for user")
    }

    /// Sets logged against `exercise_type_id`. When `include_descendants` is true,
    /// the query walks the taxonomy and matches any descendant of `exercise_type_id`.
    pub fn list_sets_for_exercise_type(
        &self,
        user_id: i64,
        exercise_type_id: i64,
        limit: usize,
        include_descendants: bool,
    ) -> anyhow::Result<Vec<ExerciseSet>> {
        let sql = if include_descendants {
            "WITH RECURSIVE tree(id) AS ( \
                 SELECT id FROM exercise_types WHERE id = ?1 \
                 UNION ALL \
                 SELECT et.id FROM exercise_types et JOIN tree t ON et.parent_id = t.id \
             ) \
             SELECT s.id, s.exercise_entry_id, s.exercise_type_id, s.order_idx, \
                    s.measurement_type_id, s.count, s.value, s.perceived_difficulty, s.comment, s.logged_at \
             FROM sets s \
             JOIN exercise_entry ee ON s.exercise_entry_id = ee.id \
             WHERE ee.user_id = ?2 AND s.exercise_type_id IN (SELECT id FROM tree) \
             ORDER BY s.logged_at DESC LIMIT ?3"
        } else {
            "SELECT s.id, s.exercise_entry_id, s.exercise_type_id, s.order_idx, \
                    s.measurement_type_id, s.count, s.value, s.perceived_difficulty, s.comment, s.logged_at \
             FROM sets s \
             JOIN exercise_entry ee ON s.exercise_entry_id = ee.id \
             WHERE ee.user_id = ?2 AND s.exercise_type_id = ?1 \
             ORDER BY s.logged_at DESC LIMIT ?3"
        };
        let mut stmt = self.conn().prepare(sql)?;
        let rows = stmt.query_map(params![exercise_type_id, user_id, limit as i64], row_to_set)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list sets for exercise_type")
    }

    pub fn list_recent_sets(&self, user_id: i64, days: i32) -> anyhow::Result<Vec<ExerciseSet>> {
        let cutoff = format!("-{days} days");
        let mut stmt = self.conn().prepare(
            "SELECT s.id, s.exercise_entry_id, s.exercise_type_id, s.order_idx, \
                    s.measurement_type_id, s.count, s.value, s.perceived_difficulty, s.comment, s.logged_at \
             FROM sets s \
             JOIN exercise_entry ee ON s.exercise_entry_id = ee.id \
             WHERE ee.user_id = ?1 AND s.logged_at >= datetime('now', ?2) \
             ORDER BY s.logged_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id, cutoff], row_to_set)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list recent sets")
    }

    /// Personal record (max value) for a given exercise_type, optionally rolling up
    /// descendants. Returns the best `ExerciseSet`.
    pub fn personal_record(&self, user_id: i64, exercise_type_id: i64, include_descendants: bool) -> anyhow::Result<Option<ExerciseSet>> {
        let sql = if include_descendants {
            "WITH RECURSIVE tree(id) AS ( \
                 SELECT id FROM exercise_types WHERE id = ?1 \
                 UNION ALL \
                 SELECT et.id FROM exercise_types et JOIN tree t ON et.parent_id = t.id \
             ) \
             SELECT s.id, s.exercise_entry_id, s.exercise_type_id, s.order_idx, \
                    s.measurement_type_id, s.count, s.value, s.perceived_difficulty, s.comment, s.logged_at \
             FROM sets s \
             JOIN exercise_entry ee ON s.exercise_entry_id = ee.id \
             WHERE ee.user_id = ?2 AND s.exercise_type_id IN (SELECT id FROM tree) \
             ORDER BY s.value DESC LIMIT 1"
        } else {
            "SELECT s.id, s.exercise_entry_id, s.exercise_type_id, s.order_idx, \
                    s.measurement_type_id, s.count, s.value, s.perceived_difficulty, s.comment, s.logged_at \
             FROM sets s \
             JOIN exercise_entry ee ON s.exercise_entry_id = ee.id \
             WHERE ee.user_id = ?2 AND s.exercise_type_id = ?1 \
             ORDER BY s.value DESC LIMIT 1"
        };
        let mut stmt = self.conn().prepare(sql)?;
        let mut rows = stmt.query_map(params![exercise_type_id, user_id], row_to_set)?;
        rows.next().transpose().context("Failed to read PR row")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{new_exercise_entry, new_exercise_set, new_user};

    fn fixture() -> (Database, i64, i64) {
        let db = Database::open_in_memory().unwrap();
        let user = new_user("Tester", None, "UTC");
        let user_id = db.insert_user(&user).unwrap();
        let bp = db.get_exercise_type_by_name("Bench Press").unwrap().unwrap();
        (db, user_id, bp.id)
    }

    #[test]
    fn start_session_returns_session_with_id() {
        let (db, user_id, _) = fixture();
        let s = db.start_session(user_id, Some("warm-up")).unwrap();
        assert!(s.id > 0);
        assert_eq!(s.user_id, user_id);
        assert!(s.ended_at.is_none());
    }

    #[test]
    fn entry_then_sets_then_pr() {
        let (db, user_id, bp_id) = fixture();
        let session = db.start_session(user_id, None).unwrap();
        let entry = new_exercise_entry(user_id, Some(session.id), Some("morning"));
        let entry_id = db.insert_entry(&entry).unwrap();

        for w in [60.0, 80.0, 70.0] {
            let mut s = new_exercise_set(entry_id, bp_id, MeasurementType::WeightReps, w);
            s.count = Some(5);
            db.insert_set(&s).unwrap();
        }

        let sets = db.list_sets_for_entry(entry_id).unwrap();
        assert_eq!(sets.len(), 3);

        let pr = db.personal_record(user_id, bp_id, false).unwrap().unwrap();
        assert!((pr.value - 80.0).abs() < 1e-6);
    }

    #[test]
    fn personal_record_with_descendants_rolls_up() {
        let (db, user_id, bp_id) = fixture();
        let flat = db.get_exercise_type_by_name("Flat Barbell Bench Press").unwrap().unwrap();
        let entry_id = db.insert_entry(&new_exercise_entry(user_id, None, None)).unwrap();
        let mut s = new_exercise_set(entry_id, flat.id, MeasurementType::WeightReps, 100.0);
        s.count = Some(5);
        db.insert_set(&s).unwrap();

        // Querying Bench Press (parent) without descendants → no PR.
        assert!(db.personal_record(user_id, bp_id, false).unwrap().is_none());
        // With descendants → finds the variation's set.
        let pr = db.personal_record(user_id, bp_id, true).unwrap().unwrap();
        assert!((pr.value - 100.0).abs() < 1e-6);
    }

    #[test]
    fn end_entry_sets_end_timestamp() {
        let (db, user_id, _) = fixture();
        let entry_id = db.insert_entry(&new_exercise_entry(user_id, None, None)).unwrap();
        db.end_entry(entry_id).unwrap();
        let entry = db.get_entry(entry_id).unwrap().unwrap();
        assert!(entry.end_timestamp.is_some());
    }

    #[test]
    fn latest_open_entry_returns_recent_only() {
        let (db, user_id, _) = fixture();
        let _id1 = db.insert_entry(&new_exercise_entry(user_id, None, None)).unwrap();
        let id2 = db.insert_entry(&new_exercise_entry(user_id, None, None)).unwrap();
        let recent = db.latest_open_entry(user_id, 60).unwrap().unwrap();
        assert_eq!(recent.id, id2);
    }
}
