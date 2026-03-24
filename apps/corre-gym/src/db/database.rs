use std::path::Path;

use anyhow::Context as _;
use rusqlite::Connection;

use super::migrations::MIGRATIONS;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("Failed to create database directory {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("Failed to open database at {}", path.display()))?;
        let db = Self { conn };
        db.run_migrations()?;
        Ok(db)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;
        let db = Self { conn };
        db.run_migrations()?;
        Ok(db)
    }

    fn run_migrations(&self) -> anyhow::Result<()> {
        self.conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        self.conn.execute_batch(MIGRATIONS).context("Failed to run database migrations")?;
        super::seed::seed_exercises(self).context("Failed to seed exercises")?;
        tracing::debug!("Database migrations applied");
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_succeeds() {
        let db = Database::open_in_memory().unwrap();
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN \
                 ('exercises', 'users', 'groups', 'group_members', 'exercise_goals', \
                  'sessions', 'exercise_logs', 'schedules', 'schedule_exercises', \
                  'health_entries', 'conversation_history', 'muscle_groups', 'measurement_types')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 13);
    }

    #[test]
    fn foreign_keys_enabled() {
        let db = Database::open_in_memory().unwrap();
        let fk: i64 = db.conn().query_row("PRAGMA foreign_keys", [], |row| row.get(0)).unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn wal_mode_enabled() {
        let db = Database::open_in_memory().unwrap();
        let mode: String = db.conn().query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
        // In-memory databases report "memory" for journal_mode, WAL only applies to file-backed DBs
        assert!(mode == "wal" || mode == "memory");
    }

    #[test]
    fn muscle_groups_seeded() {
        let db = Database::open_in_memory().unwrap();
        let count: i64 = db.conn().query_row("SELECT COUNT(*) FROM muscle_groups", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 16);
    }

    #[test]
    fn measurement_types_seeded() {
        let db = Database::open_in_memory().unwrap();
        let count: i64 = db.conn().query_row("SELECT COUNT(*) FROM measurement_types", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 5);
    }
}
