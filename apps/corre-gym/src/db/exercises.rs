use anyhow::Context as _;
use rusqlite::params;

use super::database::Database;
use super::models::{Exercise, FullExercise, MeasurementType};

fn row_to_exercise(row: &rusqlite::Row) -> rusqlite::Result<Exercise> {
    Ok(Exercise {
        id: row.get(0)?,
        name: row.get(1)?,
        aliases: row.get(2)?,
        muscle_group_id: row.get(3)?,
        purpose: row.get(4)?,
        measurement_type: MeasurementType::from_str_loose(&row.get::<_, String>(5)?),
        description: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn row_to_full_exercise(row: &rusqlite::Row) -> rusqlite::Result<FullExercise> {
    let exercise = Exercise {
        id: row.get(0)?,
        name: row.get(1)?,
        aliases: row.get(2)?,
        muscle_group_id: row.get(3)?,
        purpose: row.get(4)?,
        measurement_type: MeasurementType::from_str_loose(&row.get::<_, String>(5)?),
        description: row.get(6)?,
        created_at: row.get(7)?,
    };
    Ok(FullExercise { exercise, muscle_group: row.get(8)? })
}

const SELECT_EXERCISE: &str = "\
    SELECT e.id, e.name, e.aliases, e.muscle_group_id, e.purpose, \
           mt.name, e.description, e.created_at \
    FROM exercises e \
    JOIN measurement_types mt ON e.measurement_type_id = mt.id";

const SELECT_FULL_EXERCISE: &str = "\
    SELECT e.id, e.name, e.aliases, e.muscle_group_id, e.purpose, \
           mt.name, e.description, e.created_at, mg.name \
    FROM exercises e \
    JOIN muscle_groups mg ON e.muscle_group_id = mg.id \
    JOIN measurement_types mt ON e.measurement_type_id = mt.id";

impl Database {
    pub fn list_muscle_groups(&self) -> anyhow::Result<Vec<(i32, String)>> {
        let mut stmt = self.conn().prepare("SELECT id, name FROM muscle_groups ORDER BY id")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list muscle groups")
    }

    pub fn list_measurement_types(&self) -> anyhow::Result<Vec<(i32, String)>> {
        let mut stmt = self.conn().prepare("SELECT id, name FROM measurement_types ORDER BY id")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list measurement types")
    }

    pub fn muscle_group_id(&self, name: &str) -> anyhow::Result<Option<i32>> {
        let mut stmt = self.conn().prepare("SELECT id FROM muscle_groups WHERE name = ?1")?;
        let mut rows = stmt.query_map(params![name], |row| row.get(0))?;
        rows.next().transpose().context("Failed to look up muscle group")
    }

    pub fn measurement_type_id(&self, name: &str) -> anyhow::Result<Option<i32>> {
        let mut stmt = self.conn().prepare("SELECT id FROM measurement_types WHERE name = ?1")?;
        let mut rows = stmt.query_map(params![name], |row| row.get(0))?;
        rows.next().transpose().context("Failed to look up measurement type")
    }

    pub fn insert_exercise(&self, exercise: &Exercise) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO exercises (id, name, aliases, muscle_group_id, purpose, measurement_type_id, description, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, \
                 (SELECT id FROM measurement_types WHERE name = ?6), \
                 ?7, ?8)",
            params![
                exercise.id,
                exercise.name,
                exercise.aliases,
                exercise.muscle_group_id,
                exercise.purpose,
                exercise.measurement_type.as_str(),
                exercise.description,
                exercise.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_exercise(&self, id: &str) -> anyhow::Result<Option<Exercise>> {
        let sql = format!("{SELECT_EXERCISE} WHERE e.id = ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![id], row_to_exercise)?;
        rows.next().transpose().context("Failed to read exercise row")
    }

    pub fn get_exercise_by_name(&self, name: &str) -> anyhow::Result<Option<Exercise>> {
        let sql = format!("{SELECT_EXERCISE} WHERE e.name = ?1 COLLATE NOCASE");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![name], row_to_exercise)?;
        rows.next().transpose().context("Failed to read exercise row")
    }

    pub fn search_exercises(&self, query: &str) -> anyhow::Result<Vec<Exercise>> {
        let pattern = format!("%{query}%");
        let sql = format!("{SELECT_EXERCISE} WHERE e.name LIKE ?1 OR e.aliases LIKE ?1 ORDER BY e.name");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![pattern], row_to_exercise)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to search exercises")
    }

    pub fn list_exercises(&self) -> anyhow::Result<Vec<Exercise>> {
        let sql = format!("{SELECT_EXERCISE} ORDER BY e.name");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map([], row_to_exercise)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list exercises")
    }

    pub fn list_exercises_by_muscle_group(&self, muscle_group: &str) -> anyhow::Result<Vec<FullExercise>> {
        let sql = format!("{SELECT_FULL_EXERCISE} WHERE mg.name = ?1 ORDER BY e.name");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![muscle_group], row_to_full_exercise)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list exercises by muscle group")
    }

    pub fn update_exercise(&self, exercise: &Exercise) -> anyhow::Result<()> {
        let rows = self.conn().execute(
            "UPDATE exercises SET name = ?1, aliases = ?2, muscle_group_id = ?3, purpose = ?4, \
             measurement_type_id = (SELECT id FROM measurement_types WHERE name = ?5), \
             description = ?6 WHERE id = ?7",
            params![
                exercise.name,
                exercise.aliases,
                exercise.muscle_group_id,
                exercise.purpose,
                exercise.measurement_type.as_str(),
                exercise.description,
                exercise.id,
            ],
        )?;
        anyhow::ensure!(rows > 0, "Exercise with id {} not found", exercise.id);
        Ok(())
    }

    pub fn delete_exercise(&self, id: &str) -> anyhow::Result<()> {
        let rows = self.conn().execute("DELETE FROM exercises WHERE id = ?1", params![id])?;
        anyhow::ensure!(rows > 0, "Exercise with id {id} not found");
        Ok(())
    }

    pub fn seed_exercises(&self) -> anyhow::Result<usize> {
        super::seed::seed_exercises(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn sample_exercise(db: &Database) -> Exercise {
        let mg_id = db.muscle_group_id("chest").unwrap().unwrap();
        Exercise {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Test Bench Press".into(),
            aliases: Some("flat bench,bench".into()),
            muscle_group_id: mg_id,
            purpose: "strength".into(),
            measurement_type: MeasurementType::WeightReps,
            description: Some("A test exercise".into()),
            created_at: "2025-01-01 00:00:00".into(),
        }
    }

    #[test]
    fn insert_and_get_exercise() {
        let db = test_db();
        let ex = sample_exercise(&db);
        db.insert_exercise(&ex).unwrap();

        let fetched = db.get_exercise(&ex.id).unwrap().unwrap();
        assert_eq!(fetched.name, "Test Bench Press");
        assert_eq!(fetched.measurement_type, MeasurementType::WeightReps);
        assert_eq!(fetched.aliases.as_deref(), Some("flat bench,bench"));
    }

    #[test]
    fn get_by_name_case_insensitive() {
        let db = test_db();
        let ex = sample_exercise(&db);
        db.insert_exercise(&ex).unwrap();

        let fetched = db.get_exercise_by_name("test bench press").unwrap().unwrap();
        assert_eq!(fetched.id, ex.id);

        let fetched = db.get_exercise_by_name("TEST BENCH PRESS").unwrap().unwrap();
        assert_eq!(fetched.id, ex.id);
    }

    #[test]
    fn list_by_muscle_group() {
        let db = test_db();
        let ex = sample_exercise(&db);
        db.insert_exercise(&ex).unwrap();

        let results = db.list_exercises_by_muscle_group("chest").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].muscle_group, "chest");
    }

    #[test]
    fn search_exercises_by_name() {
        let db = test_db();
        let ex = sample_exercise(&db);
        db.insert_exercise(&ex).unwrap();

        let results = db.search_exercises("Bench").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_exercises_by_alias() {
        let db = test_db();
        let ex = sample_exercise(&db);
        db.insert_exercise(&ex).unwrap();

        let results = db.search_exercises("flat bench").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn update_exercise() {
        let db = test_db();
        let mut ex = sample_exercise(&db);
        db.insert_exercise(&ex).unwrap();

        ex.name = "Modified Bench Press".into();
        ex.purpose = "hypertrophy".into();
        db.update_exercise(&ex).unwrap();

        let fetched = db.get_exercise(&ex.id).unwrap().unwrap();
        assert_eq!(fetched.name, "Modified Bench Press");
        assert_eq!(fetched.purpose, "hypertrophy");
    }

    #[test]
    fn delete_exercise() {
        let db = test_db();
        let ex = sample_exercise(&db);
        db.insert_exercise(&ex).unwrap();
        db.delete_exercise(&ex.id).unwrap();
        assert!(db.get_exercise(&ex.id).unwrap().is_none());
    }

    #[test]
    fn delete_exercise_with_logs_fails() {
        let db = test_db();
        let ex = sample_exercise(&db);
        db.insert_exercise(&ex).unwrap();

        let user = super::super::models::new_user("Test", None, "UTC");
        db.insert_user(&user).unwrap();

        let mut log = super::super::models::new_exercise_log(&user.id, &ex.id, None);
        log.sets = Some(3);
        log.reps = Some(10);
        log.weight_kg = Some(60.0);
        db.insert_log(&log).unwrap();

        assert!(db.delete_exercise(&ex.id).is_err());
    }

    #[test]
    fn duplicate_name_fails() {
        let db = test_db();
        let ex1 = sample_exercise(&db);
        db.insert_exercise(&ex1).unwrap();

        let mut ex2 = sample_exercise(&db);
        ex2.id = uuid::Uuid::new_v4().to_string();
        assert!(db.insert_exercise(&ex2).is_err());
    }

    #[test]
    fn seed_exercises_populates_catalogue() {
        let db = test_db();
        let count = db.seed_exercises().unwrap();
        assert!(count >= 30, "Expected at least 30 exercises, got {count}");
    }

    #[test]
    fn seed_exercises_idempotent() {
        let db = test_db();
        db.seed_exercises().unwrap();
        let count_after_first = db.list_exercises().unwrap().len();
        db.seed_exercises().unwrap();
        let count_after_second = db.list_exercises().unwrap().len();
        assert_eq!(count_after_first, count_after_second);
    }

    #[test]
    fn list_muscle_groups_returns_all() {
        let db = test_db();
        let groups = db.list_muscle_groups().unwrap();
        assert_eq!(groups.len(), 16);
    }

    #[test]
    fn list_measurement_types_returns_all() {
        let db = test_db();
        let types = db.list_measurement_types().unwrap();
        assert_eq!(types.len(), 5);
    }

    #[test]
    fn muscle_group_id_lookup() {
        let db = test_db();
        assert_eq!(db.muscle_group_id("chest").unwrap(), Some(1));
        assert_eq!(db.muscle_group_id("back").unwrap(), Some(2));
        assert_eq!(db.muscle_group_id("nonexistent").unwrap(), None);
    }
}
