use anyhow::Context as _;
use rusqlite::{OptionalExtension as _, params};

use super::database::Database;
use super::models::{ExerciseGoal, GoalProgress, GoalStatus, MeasurementType, TimeSeries, TimeSeriesPoint};

impl Database {
    /// Time series for a single exercise. Returns one data point per day,
    /// using the best set from each day.
    /// Defaults: from = 1 year ago, to = today.
    pub fn exercise_time_series(
        &self,
        user_id: &str,
        exercise_id: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> anyhow::Result<Vec<TimeSeriesPoint>> {
        let default_from = (chrono::Utc::now() - chrono::Duration::days(365)).format("%Y-%m-%d").to_string();
        let default_to = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let from = from.unwrap_or(&default_from);
        let to = to.unwrap_or(&default_to);

        // Look up measurement type
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
            None => return Ok(vec![]),
        };

        let (value_expr, not_null_filter) = match mt {
            MeasurementType::WeightReps => ("MAX(el.weight_kg)", "el.weight_kg IS NOT NULL"),
            MeasurementType::TimeBased => ("MAX(el.duration_secs)", "el.duration_secs IS NOT NULL"),
            MeasurementType::DistanceBased => ("MAX(el.distance_m)", "el.distance_m IS NOT NULL"),
            MeasurementType::LevelBased | MeasurementType::ScoreBased => ("MAX(el.level)", "el.level IS NOT NULL"),
        };

        let sql = format!(
            "SELECT date(el.logged_at) AS d, {value_expr} AS value \
             FROM exercise_logs el \
             WHERE el.user_id = ?1 AND el.exercise_id = ?2 \
               AND el.logged_at >= ?3 AND el.logged_at <= ?4 \
               AND {not_null_filter} \
             GROUP BY date(el.logged_at) \
             ORDER BY date(el.logged_at)"
        );

        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, exercise_id, from, to], |row| {
            Ok(TimeSeriesPoint { date: row.get(0)?, value: row.get(1)? })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to query exercise time series")
    }

    /// Time series for all exercises in a muscle group.
    pub fn muscle_group_time_series(
        &self,
        user_id: &str,
        muscle_group: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> anyhow::Result<Vec<TimeSeries>> {
        let default_from = (chrono::Utc::now() - chrono::Duration::days(365)).format("%Y-%m-%d").to_string();
        let default_to = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let from_str = from.unwrap_or(&default_from);
        let to_str = to.unwrap_or(&default_to);

        // Discover exercises with data in the period
        let mut stmt = self.conn().prepare(
            "SELECT DISTINCT e.id, e.name, mt.name AS measurement_type \
             FROM exercises e \
             JOIN muscle_groups mg ON e.muscle_group_id = mg.id \
             JOIN measurement_types mt ON e.measurement_type_id = mt.id \
             JOIN exercise_logs el ON el.exercise_id = e.id \
             WHERE el.user_id = ?1 AND mg.name = ?2 \
               AND el.logged_at >= ?3 AND el.logged_at <= ?4",
        )?;

        let exercise_info: Vec<(String, String, String)> = stmt
            .query_map(params![user_id, muscle_group, from_str, to_str], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to discover exercises")?;

        exercise_info
            .into_iter()
            .map(|(ex_id, ex_name, mt_str)| {
                let points = self.exercise_time_series(user_id, &ex_id, Some(from_str), Some(to_str))?;
                Ok(TimeSeries {
                    exercise_id: ex_id,
                    exercise_name: ex_name,
                    measurement_type: MeasurementType::from_str_loose(&mt_str),
                    points,
                })
            })
            .collect()
    }

    /// Time series for all exercises that have an active or recently-completed goal.
    pub fn goal_time_series(
        &self,
        user_id: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> anyhow::Result<Vec<TimeSeries>> {
        let default_from = (chrono::Utc::now() - chrono::Duration::days(365)).format("%Y-%m-%d").to_string();
        let default_to = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let from_str = from.unwrap_or(&default_from);
        let to_str = to.unwrap_or(&default_to);

        let mut stmt = self.conn().prepare(
            "SELECT DISTINCT e.id, e.name, mt.name AS measurement_type \
             FROM exercise_goals g \
             JOIN exercises e ON g.exercise_id = e.id \
             JOIN measurement_types mt ON e.measurement_type_id = mt.id \
             WHERE g.user_id = ?1 \
               AND g.start_date <= ?3 AND (g.end_date IS NULL OR g.end_date >= ?2)",
        )?;

        let exercise_info: Vec<(String, String, String)> = stmt
            .query_map(params![user_id, from_str, to_str], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to discover goal exercises")?;

        exercise_info
            .into_iter()
            .map(|(ex_id, ex_name, mt_str)| {
                let points = self.exercise_time_series(user_id, &ex_id, Some(from_str), Some(to_str))?;
                Ok(TimeSeries {
                    exercise_id: ex_id,
                    exercise_name: ex_name,
                    measurement_type: MeasurementType::from_str_loose(&mt_str),
                    points,
                })
            })
            .collect()
    }

    /// Goal progress report for a period. Lists every goal whose date range
    /// overlaps [from, to], with current progress percentage and status.
    pub fn goal_progress_report(
        &self,
        user_id: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> anyhow::Result<Vec<GoalProgress>> {
        let default_from = (chrono::Utc::now() - chrono::Duration::days(365)).format("%Y-%m-%d").to_string();
        let default_to = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let from_str = from.unwrap_or(&default_from);
        let to_str = to.unwrap_or(&default_to);
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        // Find all goals overlapping the period
        let mut stmt = self.conn().prepare(
            "SELECT g.id, g.user_id, g.exercise_id, g.target_value, g.start_date, g.end_date, \
                    g.achieved, g.notes, g.created_at, g.updated_at, \
                    e.name AS exercise_name, mt.name AS measurement_type \
             FROM exercise_goals g \
             JOIN exercises e ON g.exercise_id = e.id \
             JOIN measurement_types mt ON e.measurement_type_id = mt.id \
             WHERE g.user_id = ?1 \
               AND g.start_date <= ?3 AND (g.end_date IS NULL OR g.end_date >= ?2) \
             ORDER BY g.start_date",
        )?;

        let goals_with_info: Vec<(ExerciseGoal, String, MeasurementType)> = stmt
            .query_map(params![user_id, from_str, to_str], |row| {
                let goal = ExerciseGoal {
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
                };
                let exercise_name: String = row.get(10)?;
                let mt_str: String = row.get(11)?;
                Ok((goal, exercise_name, MeasurementType::from_str_loose(&mt_str)))
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to query goals")?;

        goals_with_info
            .into_iter()
            .map(|(goal, exercise_name, mt)| {
                // Get the current best value within the goal's date range
                let goal_end = goal.end_date.as_deref().unwrap_or(to_str);
                let current_value = self.best_value_for_exercise(user_id, &goal.exercise_id, &goal.start_date, goal_end, mt)?;

                let percentage = match (current_value, goal.target_value) {
                    (_, tv) if tv == 0.0 => 0.0,
                    (Some(cv), tv) => (cv / tv) * 100.0,
                    (None, _) => 0.0,
                };

                let status = if goal.achieved || percentage >= 100.0 {
                    GoalStatus::Achieved
                } else if goal.end_date.as_deref().is_some_and(|ed| ed < today.as_str()) {
                    GoalStatus::Failed
                } else {
                    GoalStatus::Active
                };

                Ok(GoalProgress { goal, exercise_name, status, current_value, percentage })
            })
            .collect()
    }

    fn best_value_for_exercise(
        &self,
        user_id: &str,
        exercise_id: &str,
        from: &str,
        to: &str,
        mt: MeasurementType,
    ) -> anyhow::Result<Option<f64>> {
        let value_expr = match mt {
            MeasurementType::WeightReps => "MAX(el.weight_kg)",
            MeasurementType::TimeBased => "MAX(el.duration_secs)",
            MeasurementType::DistanceBased => "MAX(el.distance_m)",
            MeasurementType::LevelBased | MeasurementType::ScoreBased => "MAX(el.level)",
        };

        let sql = format!(
            "SELECT {value_expr} FROM exercise_logs el \
             WHERE el.user_id = ?1 AND el.exercise_id = ?2 \
               AND el.logged_at >= ?3 AND el.logged_at <= ?4"
        );

        self.conn()
            .query_row(&sql, params![user_id, exercise_id, from, to], |row| row.get(0))
            .optional()
            .context("Failed to query best value")
            .map(|v| v.flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::models::{new_exercise_goal, new_exercise_log, new_user};

    fn test_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.seed_exercises().unwrap();
        db
    }

    #[test]
    fn exercise_time_series_returns_daily_points() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises().unwrap();
        let bench = exercises.iter().find(|e| e.name.contains("Bench")).unwrap();

        // Insert logs on different days
        for (day, weight) in [(1, 60.0), (2, 65.0), (3, 70.0)] {
            let mut log = new_exercise_log(&user.id, &bench.id, None);
            log.logged_at = format!("2025-06-{day:02} 10:00:00");
            log.sets = Some(3);
            log.reps = Some(8);
            log.weight_kg = Some(weight);
            db.insert_log(&log).unwrap();
        }

        let points = db.exercise_time_series(&user.id, &bench.id, Some("2025-06-01"), Some("2025-06-30")).unwrap();
        assert_eq!(points.len(), 3);
        assert!(points[0].value < points[2].value);
    }

    #[test]
    fn exercise_time_series_branches_by_measurement_type() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises().unwrap();

        // Find a time_based exercise (Plank)
        let plank = exercises.iter().find(|e| e.measurement_type == MeasurementType::TimeBased);
        if let Some(plank) = plank {
            let mut log = new_exercise_log(&user.id, &plank.id, None);
            log.logged_at = "2025-06-01 10:00:00".into();
            log.duration_secs = Some(120);
            db.insert_log(&log).unwrap();

            let points = db.exercise_time_series(&user.id, &plank.id, Some("2025-06-01"), Some("2025-06-30")).unwrap();
            assert_eq!(points.len(), 1);
            assert_eq!(points[0].value, 120.0);
        }
    }

    #[test]
    fn muscle_group_time_series_groups_exercises() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises_by_muscle_group("chest").unwrap();

        // Insert data for 2 chest exercises
        for ex in exercises.iter().take(2) {
            let mut log = new_exercise_log(&user.id, &ex.exercise.id, None);
            log.logged_at = "2025-06-01 10:00:00".into();
            log.sets = Some(3);
            log.reps = Some(10);
            log.weight_kg = Some(60.0);
            db.insert_log(&log).unwrap();
        }

        let series = db.muscle_group_time_series(&user.id, "chest", Some("2025-06-01"), Some("2025-06-30")).unwrap();
        assert!(series.len() >= 2, "Expected at least 2 exercise series, got {}", series.len());
    }

    #[test]
    fn goal_time_series_includes_goal_exercises() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises().unwrap();

        let mut goal = new_exercise_goal(&user.id, &exercises[0].id, 100.0);
        goal.start_date = "2025-01-01".into();
        db.insert_goal(&goal).unwrap();

        // Insert some data
        let mut log = new_exercise_log(&user.id, &exercises[0].id, None);
        log.logged_at = "2025-06-01 10:00:00".into();
        log.weight_kg = Some(80.0);
        db.insert_log(&log).unwrap();

        let series = db.goal_time_series(&user.id, Some("2025-01-01"), Some("2025-12-31")).unwrap();
        assert_eq!(series.len(), 1);
    }

    #[test]
    fn goal_progress_report_computes_percentages() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises().unwrap();

        let mut goal = new_exercise_goal(&user.id, &exercises[0].id, 100.0);
        goal.start_date = "2025-01-01".into();
        goal.end_date = Some("2025-12-31".into());
        db.insert_goal(&goal).unwrap();

        let mut log = new_exercise_log(&user.id, &exercises[0].id, None);
        log.logged_at = "2025-06-01 10:00:00".into();
        log.weight_kg = Some(80.0);
        db.insert_log(&log).unwrap();

        let report = db.goal_progress_report(&user.id, Some("2025-01-01"), Some("2025-12-31")).unwrap();
        assert_eq!(report.len(), 1);
        assert!((report[0].percentage - 80.0).abs() < 0.01);
    }

    #[test]
    fn goal_progress_report_derives_status() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises().unwrap();

        // Achieved goal
        let mut achieved_goal = new_exercise_goal(&user.id, &exercises[0].id, 100.0);
        achieved_goal.start_date = "2025-01-01".into();
        achieved_goal.end_date = Some("2025-12-31".into());
        achieved_goal.achieved = true;
        db.insert_goal(&achieved_goal).unwrap();

        // Failed goal (past end date, not achieved)
        let mut failed_goal = new_exercise_goal(&user.id, &exercises[1].id, 200.0);
        failed_goal.start_date = "2024-01-01".into();
        failed_goal.end_date = Some("2024-06-01".into());
        db.insert_goal(&failed_goal).unwrap();

        let report = db.goal_progress_report(&user.id, Some("2024-01-01"), Some("2025-12-31")).unwrap();
        assert!(report.iter().any(|r| r.status == GoalStatus::Achieved));
        assert!(report.iter().any(|r| r.status == GoalStatus::Failed));
    }

    #[test]
    fn goal_progress_zero_target_returns_zero_percent() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();
        let exercises = db.list_exercises().unwrap();

        let mut goal = new_exercise_goal(&user.id, &exercises[0].id, 0.0);
        goal.start_date = "2025-01-01".into();
        db.insert_goal(&goal).unwrap();

        let report = db.goal_progress_report(&user.id, Some("2025-01-01"), Some("2025-12-31")).unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].percentage, 0.0);
    }
}
