use anyhow::Context as _;
use rusqlite::params;

use super::database::Database;
use super::models::User;

pub(super) fn row_to_user(row: &rusqlite::Row) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        name: row.get(1)?,
        telegram_id: row.get(2)?,
        signal_id: row.get(3)?,
        timezone: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

const SELECT_USER: &str = "SELECT id, name, telegram_id, signal_id, timezone, created_at, updated_at FROM users";

impl Database {
    pub fn insert_user(&self, user: &User) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO users (id, name, telegram_id, signal_id, timezone, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![user.id, user.name, user.telegram_id, user.signal_id, user.timezone, user.created_at, user.updated_at],
        )?;
        Ok(())
    }

    pub fn get_user(&self, id: &str) -> anyhow::Result<Option<User>> {
        let sql = format!("{SELECT_USER} WHERE id = ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![id], row_to_user)?;
        rows.next().transpose().context("Failed to read user row")
    }

    pub fn get_user_by_telegram_id(&self, telegram_id: &str) -> anyhow::Result<Option<User>> {
        let sql = format!("{SELECT_USER} WHERE telegram_id = ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![telegram_id], row_to_user)?;
        rows.next().transpose().context("Failed to read user row")
    }

    pub fn get_user_by_signal_id(&self, signal_id: &str) -> anyhow::Result<Option<User>> {
        let sql = format!("{SELECT_USER} WHERE signal_id = ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![signal_id], row_to_user)?;
        rows.next().transpose().context("Failed to read user row")
    }

    pub fn update_user(&self, user: &User) -> anyhow::Result<()> {
        let rows = self.conn().execute(
            "UPDATE users SET name = ?1, telegram_id = ?2, signal_id = ?3, timezone = ?4, \
             updated_at = datetime('now') WHERE id = ?5",
            params![user.name, user.telegram_id, user.signal_id, user.timezone, user.id],
        )?;
        anyhow::ensure!(rows > 0, "User with id {} not found", user.id);
        Ok(())
    }

    pub fn delete_user(&self, id: &str) -> anyhow::Result<()> {
        let rows = self.conn().execute("DELETE FROM users WHERE id = ?1", params![id])?;
        anyhow::ensure!(rows > 0, "User with id {id} not found");
        Ok(())
    }

    pub fn list_users(&self) -> anyhow::Result<Vec<User>> {
        let sql = format!("{SELECT_USER} ORDER BY name");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map([], row_to_user)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list users")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::models::new_user;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn insert_and_get_user() {
        let db = test_db();
        let user = new_user("Alice", Some("alice_tg"), "Europe/London");
        db.insert_user(&user).unwrap();

        let fetched = db.get_user(&user.id).unwrap().unwrap();
        assert_eq!(fetched.name, "Alice");
        assert_eq!(fetched.telegram_id.as_deref(), Some("alice_tg"));
        assert_eq!(fetched.timezone, "Europe/London");
    }

    #[test]
    fn get_by_telegram_id() {
        let db = test_db();
        let user = new_user("Bob", Some("bob_tg"), "UTC");
        db.insert_user(&user).unwrap();

        let fetched = db.get_user_by_telegram_id("bob_tg").unwrap().unwrap();
        assert_eq!(fetched.id, user.id);
    }

    #[test]
    fn duplicate_telegram_id_fails() {
        let db = test_db();
        let u1 = new_user("Alice", Some("same_tg"), "UTC");
        let u2 = new_user("Bob", Some("same_tg"), "UTC");
        db.insert_user(&u1).unwrap();
        assert!(db.insert_user(&u2).is_err());
    }

    #[test]
    fn update_user() {
        let db = test_db();
        let mut user = new_user("Alice", None, "UTC");
        db.insert_user(&user).unwrap();

        user.name = "Alicia".into();
        user.telegram_id = Some("alicia_tg".into());
        db.update_user(&user).unwrap();

        let fetched = db.get_user(&user.id).unwrap().unwrap();
        assert_eq!(fetched.name, "Alicia");
        assert_eq!(fetched.telegram_id.as_deref(), Some("alicia_tg"));
    }

    #[test]
    fn delete_user_cascades() {
        let db = test_db();
        let user = new_user("Alice", None, "UTC");
        db.insert_user(&user).unwrap();

        // Insert an exercise and a log for the user
        db.seed_exercises().unwrap();
        let exercises = db.list_exercises().unwrap();
        let ex = &exercises[0];

        let session = db.start_session(&user.id, None).unwrap();
        let mut log = super::super::models::new_exercise_log(&user.id, &ex.id, Some(&session.id));
        log.sets = Some(3);
        log.reps = Some(10);
        log.weight_kg = Some(60.0);
        db.insert_log(&log).unwrap();

        // Delete user — cascades should remove session + log
        db.delete_user(&user.id).unwrap();
        assert!(db.get_user(&user.id).unwrap().is_none());
        assert!(db.get_session(&session.id).unwrap().is_none());
    }
}
