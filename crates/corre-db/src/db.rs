use anyhow::Context;
use rusqlite::Connection;
use std::path::Path;

const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS contacts (
    id                       TEXT PRIMARY KEY,
    first_name               TEXT NOT NULL,
    last_name                TEXT NOT NULL,
    nickname                 TEXT,
    email                    TEXT,
    phone                    TEXT,
    telegram                 TEXT,
    whatsapp                 TEXT,
    signal                   TEXT,
    facebook                 TEXT,
    linkedin                 TEXT,
    preferred_contact_method TEXT NOT NULL DEFAULT 'email',
    birthday                 TEXT,
    importance               TEXT NOT NULL DEFAULT 'medium',
    notes                    TEXT,
    created_at               TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at               TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS outreach_strategies (
    id              TEXT PRIMARY KEY,
    contact_id      TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    strategy_type   TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    interval_days   INTEGER,
    last_executed   TEXT,
    config_json     TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS outreach_log (
    id              TEXT PRIMARY KEY,
    contact_id      TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    strategy_type   TEXT NOT NULL,
    executed_at     TEXT NOT NULL DEFAULT (datetime('now')),
    result          TEXT NOT NULL,
    details         TEXT
);

CREATE TABLE IF NOT EXISTS contact_profiles (
    id          TEXT PRIMARY KEY,
    contact_id  TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    source      TEXT NOT NULL,
    category    TEXT NOT NULL,
    content     TEXT NOT NULL,
    observed_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
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
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        self.conn.execute_batch(MIGRATIONS).context("Failed to run database migrations")?;
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
        // Verify tables exist by querying sqlite_master
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('contacts', 'outreach_strategies', 'outreach_log', 'contact_profiles')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 4);
    }

    #[test]
    fn foreign_keys_enabled() {
        let db = Database::open_in_memory().unwrap();
        let fk: i64 = db.conn().query_row("PRAGMA foreign_keys", [], |row| row.get(0)).unwrap();
        assert_eq!(fk, 1);
    }
}
