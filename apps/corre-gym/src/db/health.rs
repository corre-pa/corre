use anyhow::Context as _;
use rusqlite::params;

use super::database::Database;
use super::models::{HealthEntry, HealthEntryType};

fn row_to_health_entry(row: &rusqlite::Row) -> rusqlite::Result<HealthEntry> {
    Ok(HealthEntry {
        id: row.get(0)?,
        user_id: row.get(1)?,
        entry_type: HealthEntryType::from_str_loose(&row.get::<_, String>(2)?),
        body_part: row.get(3)?,
        severity: row.get(4)?,
        description: row.get(5)?,
        started_at: row.get(6)?,
        resolved_at: row.get(7)?,
        notes: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

const SELECT_HEALTH: &str = "\
    SELECT id, user_id, entry_type, body_part, severity, description, \
           started_at, resolved_at, notes, updated_at \
    FROM health_entries";

impl Database {
    pub fn insert_health_entry(&self, entry: &HealthEntry) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO health_entries (id, user_id, entry_type, body_part, severity, description, \
             started_at, resolved_at, notes, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                entry.id, entry.user_id, entry.entry_type.as_str(), entry.body_part,
                entry.severity, entry.description, entry.started_at, entry.resolved_at,
                entry.notes, entry.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_health_entry(&self, id: &str) -> anyhow::Result<Option<HealthEntry>> {
        let sql = format!("{SELECT_HEALTH} WHERE id = ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map(params![id], row_to_health_entry)?;
        rows.next().transpose().context("Failed to read health entry row")
    }

    pub fn list_active_health_entries(&self, user_id: &str) -> anyhow::Result<Vec<HealthEntry>> {
        let sql = format!("{SELECT_HEALTH} WHERE user_id = ?1 AND resolved_at IS NULL ORDER BY started_at DESC");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id], row_to_health_entry)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list active health entries")
    }

    pub fn list_health_entries_by_type(
        &self,
        user_id: &str,
        entry_type: HealthEntryType,
        limit: usize,
    ) -> anyhow::Result<Vec<HealthEntry>> {
        let sql = format!("{SELECT_HEALTH} WHERE user_id = ?1 AND entry_type = ?2 ORDER BY started_at DESC LIMIT ?3");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, entry_type.as_str(), limit as i64], row_to_health_entry)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list health entries by type")
    }

    pub fn list_health_history(&self, user_id: &str, limit: usize) -> anyhow::Result<Vec<HealthEntry>> {
        let sql = format!("{SELECT_HEALTH} WHERE user_id = ?1 ORDER BY started_at DESC LIMIT ?2");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![user_id, limit as i64], row_to_health_entry)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list health history")
    }

    pub fn resolve_health_entry(&self, id: &str) -> anyhow::Result<()> {
        let rows = self.conn().execute(
            "UPDATE health_entries SET resolved_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1",
            params![id],
        )?;
        anyhow::ensure!(rows > 0, "Health entry with id {id} not found");
        Ok(())
    }

    pub fn update_health_entry(&self, entry: &HealthEntry) -> anyhow::Result<()> {
        let rows = self.conn().execute(
            "UPDATE health_entries SET entry_type = ?1, body_part = ?2, severity = ?3, \
             description = ?4, resolved_at = ?5, notes = ?6, updated_at = datetime('now') WHERE id = ?7",
            params![
                entry.entry_type.as_str(), entry.body_part, entry.severity,
                entry.description, entry.resolved_at, entry.notes, entry.id,
            ],
        )?;
        anyhow::ensure!(rows > 0, "Health entry with id {} not found", entry.id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::models::{new_health_entry, new_user};

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn insert_and_list_active() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();

        let mut entry = new_health_entry(&user.id, HealthEntryType::Injury, "Shoulder pain");
        entry.body_part = Some("shoulder".into());
        entry.severity = "moderate".into();
        db.insert_health_entry(&entry).unwrap();

        let active = db.list_active_health_entries(&user.id).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].entry_type, HealthEntryType::Injury);
    }

    #[test]
    fn list_by_type() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();

        let injury = new_health_entry(&user.id, HealthEntryType::Injury, "Bad knee");
        let illness = new_health_entry(&user.id, HealthEntryType::Illness, "Cold");
        db.insert_health_entry(&injury).unwrap();
        db.insert_health_entry(&illness).unwrap();

        let injuries = db.list_health_entries_by_type(&user.id, HealthEntryType::Injury, 10).unwrap();
        assert_eq!(injuries.len(), 1);
    }

    #[test]
    fn resolve_health_entry() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();

        let entry = new_health_entry(&user.id, HealthEntryType::Injury, "Sprained ankle");
        db.insert_health_entry(&entry).unwrap();

        db.resolve_health_entry(&entry.id).unwrap();
        let fetched = db.get_health_entry(&entry.id).unwrap().unwrap();
        assert!(fetched.resolved_at.is_some());

        let active = db.list_active_health_entries(&user.id).unwrap();
        assert!(active.is_empty());
    }

    #[test]
    fn list_history_ordered_by_date() {
        let db = test_db();
        let user = new_user("Tester", None, "UTC");
        db.insert_user(&user).unwrap();

        let e1 = new_health_entry(&user.id, HealthEntryType::Injury, "First");
        let e2 = new_health_entry(&user.id, HealthEntryType::Illness, "Second");
        db.insert_health_entry(&e1).unwrap();
        db.insert_health_entry(&e2).unwrap();

        let history = db.list_health_history(&user.id, 10).unwrap();
        assert_eq!(history.len(), 2);
    }
}
