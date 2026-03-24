use anyhow::Context as _;
use rusqlite::params;

use super::database::Database;
use super::models::ExerciseGoal;

fn row_to_goal(row: &rusqlite::Row) -> rusqlite::Result<ExerciseGoal> {
    Ok(ExerciseGoal {
        id: row.get(0)?,
        user_id: row.get(1)?,
        exercise_id: row.get(2)?,
        target_value: row.get(3)?,
        start_date: row.get(4)?,
        end_date: row.get(5)?,
        achieved: row.get::<_, i32>(6)? != 0,
        notes: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

const SELECT_GOAL: &str = "\
    SELECT id, user_id, exercise_id, target_value, start_date, end_date, \
           achieved, notes, created_at, updated_at \
    FROM exercise_goals";

impl Database {
    pub fn insert_goal(&self, goal: &ExerciseGoal) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO exercise_goals (id, user_id, exercise_id, target_value, start_date, end_date, \
             achieved, notes, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                goal.id,
                goal.user_id,
                goal.exercise_id,
                goal.target_value,
                goal.start_date,
                goal.end_date,
                goal.achieved as i32,
                goal.notes,
                goal.created_at,
                goal.updated_at,
            ],
        )?;
        tracing::debug!(id = %goal.id, exercise_id = %goal.exercise_id, target = %goal.target_value, "DB: inserted goal");
        Ok(())
    }

    pub fn get_goal(&self, id: &str) -> anyhow::Result<Option<ExerciseGoal>> {
        let sql = format!("{SELECT_GOAL} WHERE id = ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![id], row_to_goal)?;
        rows.next().transpose().context("Failed to read goal row")
    }

    pub fn list_active_goals(&self, user_id: &str) -> anyhow::Result<Vec<ExerciseGoal>> {
        let sql = format!("{SELECT_GOAL} WHERE user_id = ?1 AND achieved = 0 ORDER BY start_date");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id], row_to_goal)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list active goals")
    }

    pub fn list_goals_in_period(&self, user_id: &str, from: &str, to: &str) -> anyhow::Result<Vec<ExerciseGoal>> {
        let sql =
            format!("{SELECT_GOAL} WHERE user_id = ?1 AND start_date <= ?3 AND (end_date IS NULL OR end_date >= ?2) ORDER BY start_date");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, from, to], row_to_goal)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list goals in period")
    }

    pub fn mark_goal_achieved(&self, id: &str) -> anyhow::Result<()> {
        let rows =
            self.conn().execute("UPDATE exercise_goals SET achieved = 1, updated_at = datetime('now') WHERE id = ?1", params![id])?;
        anyhow::ensure!(rows > 0, "Goal with id {id} not found");
        Ok(())
    }

    pub fn delete_goal(&self, id: &str) -> anyhow::Result<()> {
        let rows = self.conn().execute("DELETE FROM exercise_goals WHERE id = ?1", params![id])?;
        anyhow::ensure!(rows > 0, "Goal with id {id} not found");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::models::{new_exercise_goal, new_user};
    use super::*;

    fn test_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.seed_exercises().unwrap();
        db
    }

    #[test]
    fn insert_and_list_active_goals() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises().unwrap();

        let mut goal = new_exercise_goal(&user.id, &exercises[0].id, 100.0);
        goal.start_date = "2025-01-01".into();
        db.insert_goal(&goal).unwrap();

        let goals = db.list_active_goals(&user.id).unwrap();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].target_value, 100.0);
    }

    #[test]
    fn list_goals_in_period() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises().unwrap();

        let mut g1 = new_exercise_goal(&user.id, &exercises[0].id, 100.0);
        g1.start_date = "2025-01-01".into();
        g1.end_date = Some("2025-06-01".into());
        db.insert_goal(&g1).unwrap();

        let mut g2 = new_exercise_goal(&user.id, &exercises[1].id, 50.0);
        g2.start_date = "2025-07-01".into();
        g2.end_date = Some("2025-12-01".into());
        db.insert_goal(&g2).unwrap();

        // Query for first half of 2025
        let goals = db.list_goals_in_period(&user.id, "2025-01-01", "2025-06-30").unwrap();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].target_value, 100.0);
    }

    #[test]
    fn mark_goal_achieved() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises().unwrap();

        let goal = new_exercise_goal(&user.id, &exercises[0].id, 100.0);
        db.insert_goal(&goal).unwrap();
        db.mark_goal_achieved(&goal.id).unwrap();

        let fetched = db.get_goal(&goal.id).unwrap().unwrap();
        assert!(fetched.achieved);

        // Should no longer appear in active goals
        let active = db.list_active_goals(&user.id).unwrap();
        assert!(active.is_empty());
    }
}
