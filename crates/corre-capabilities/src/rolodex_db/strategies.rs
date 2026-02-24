//! Outreach strategy management, execution scheduling, and the outreach log.
//!
//! Extends `Database` with methods to insert, query, and replace strategies, determine
//! which are due based on elapsed interval, mark them executed, and generate default
//! strategy sets from a contact's importance level.

use super::db::Database;
use super::models::{Contact, Importance, OutreachLog, OutreachStrategy, StrategyType};
use anyhow::Context;
use rusqlite::params;

impl Database {
    pub fn insert_strategy(&self, strategy: &OutreachStrategy) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO outreach_strategies (id, contact_id, strategy_type, enabled, interval_days, last_executed, config_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                strategy.id,
                strategy.contact_id,
                strategy.strategy_type.as_str(),
                strategy.enabled as i32,
                strategy.interval_days,
                strategy.last_executed,
                strategy.config_json,
                strategy.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_strategies_for_contact(&self, contact_id: &str) -> anyhow::Result<Vec<OutreachStrategy>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, contact_id, strategy_type, enabled, interval_days, last_executed, config_json, created_at \
             FROM outreach_strategies WHERE contact_id = ?1 ORDER BY strategy_type",
        )?;
        let rows = stmt.query_map(params![contact_id], row_to_strategy)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to get strategies for contact")
    }

    /// Replace all strategies for a contact with the given list.
    pub fn set_strategies_for_contact(&self, contact_id: &str, strategies: &[OutreachStrategy]) -> anyhow::Result<()> {
        self.conn().execute("DELETE FROM outreach_strategies WHERE contact_id = ?1", params![contact_id])?;
        strategies.iter().try_for_each(|s| self.insert_strategy(s))
    }

    /// Find all enabled strategies that are due for execution.
    /// A strategy is due when `last_executed` is NULL or when
    /// `interval_days` has elapsed since `last_executed`.
    pub fn strategies_due(&self, now: &chrono::DateTime<chrono::Utc>) -> anyhow::Result<Vec<OutreachStrategy>> {
        let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let mut stmt = self.conn().prepare(
            "SELECT id, contact_id, strategy_type, enabled, interval_days, last_executed, config_json, created_at \
             FROM outreach_strategies \
             WHERE enabled = 1 AND (last_executed IS NULL OR julianday(?1) - julianday(last_executed) >= interval_days)",
        )?;
        let rows = stmt.query_map(params![now_str], row_to_strategy)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to query due strategies")
    }

    /// Find all enabled strategies of a specific type that are due.
    pub fn strategies_due_by_type(
        &self,
        strategy_type: StrategyType,
        now: &chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<OutreachStrategy>> {
        let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let mut stmt = self.conn().prepare(
            "SELECT id, contact_id, strategy_type, enabled, interval_days, last_executed, config_json, created_at \
             FROM outreach_strategies \
             WHERE enabled = 1 AND strategy_type = ?1 \
             AND (last_executed IS NULL OR julianday(?2) - julianday(last_executed) >= interval_days)",
        )?;
        let rows = stmt.query_map(params![strategy_type.as_str(), now_str], row_to_strategy)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to query due strategies by type")
    }

    /// Mark a strategy as executed at the given time.
    pub fn mark_strategy_executed(&self, strategy_id: &str, now: &chrono::DateTime<chrono::Utc>) -> anyhow::Result<()> {
        let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let rows = self.conn().execute("UPDATE outreach_strategies SET last_executed = ?1 WHERE id = ?2", params![now_str, strategy_id])?;
        anyhow::ensure!(rows > 0, "Strategy with id {strategy_id} not found");
        Ok(())
    }

    /// Log an outreach action.
    pub fn log_outreach(&self, log: &OutreachLog) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO outreach_log (id, contact_id, strategy_type, executed_at, result, details) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![log.id, log.contact_id, log.strategy_type.as_str(), log.executed_at, log.result, log.details],
        )?;
        Ok(())
    }

    /// Get outreach logs for a contact, most recent first.
    pub fn get_outreach_logs(&self, contact_id: &str) -> anyhow::Result<Vec<OutreachLog>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, contact_id, strategy_type, executed_at, result, details \
             FROM outreach_log WHERE contact_id = ?1 ORDER BY executed_at DESC",
        )?;
        let rows = stmt.query_map(params![contact_id], row_to_log)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to get outreach logs")
    }

    /// Assign default outreach strategies for a contact based on importance level.
    ///
    /// | Importance | Strategies |
    /// |---|---|
    /// | Low | PeriodicCheckin (90 days) |
    /// | Medium | BirthdayMessage, PeriodicCheckin (60 days) |
    /// | High | BirthdayMessage, NewsSearch, PeriodicCheckin (30 days) |
    /// | VeryHigh | BirthdayMessage, NewsSearch, DraftCongratulations, PeriodicCheckin (14 days) |
    ///
    /// Only adds BirthdayMessage if `birthday` is set on the contact.
    pub fn default_strategies_for(&self, contact: &Contact) -> anyhow::Result<Vec<OutreachStrategy>> {
        let has_birthday = contact.birthday.is_some();
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let mut strategies = Vec::new();
        let mut add = |stype: StrategyType, interval: Option<i64>| {
            strategies.push(OutreachStrategy {
                id: uuid::Uuid::new_v4().to_string(),
                contact_id: contact.id.clone(),
                strategy_type: stype,
                enabled: true,
                interval_days: interval,
                last_executed: None,
                config_json: None,
                created_at: now.clone(),
            });
        };

        match contact.importance {
            Importance::Low => {
                add(StrategyType::PeriodicCheckin, Some(90));
            }
            Importance::Medium => {
                if has_birthday {
                    add(StrategyType::BirthdayMessage, Some(365));
                }
                add(StrategyType::PeriodicCheckin, Some(60));
            }
            Importance::High => {
                if has_birthday {
                    add(StrategyType::BirthdayMessage, Some(365));
                }
                add(StrategyType::NewsSearch, Some(7));
                add(StrategyType::ProfileScrape, Some(30));
                add(StrategyType::PeriodicCheckin, Some(30));
            }
            Importance::VeryHigh => {
                if has_birthday {
                    add(StrategyType::BirthdayMessage, Some(365));
                }
                add(StrategyType::NewsSearch, Some(7));
                add(StrategyType::DraftCongratulations, Some(7));
                add(StrategyType::ProfileScrape, Some(14));
                add(StrategyType::PeriodicCheckin, Some(14));
            }
        }

        Ok(strategies)
    }

    /// Insert default strategies for a contact (convenience wrapper).
    pub fn assign_default_strategies(&self, contact: &Contact) -> anyhow::Result<()> {
        let strategies = self.default_strategies_for(contact)?;
        strategies.iter().try_for_each(|s| self.insert_strategy(s))
    }
}

fn row_to_strategy(row: &rusqlite::Row) -> rusqlite::Result<OutreachStrategy> {
    Ok(OutreachStrategy {
        id: row.get(0)?,
        contact_id: row.get(1)?,
        strategy_type: StrategyType::from_str_loose(&row.get::<_, String>(2)?).unwrap_or(StrategyType::PeriodicCheckin),
        enabled: row.get::<_, i32>(3)? != 0,
        interval_days: row.get(4)?,
        last_executed: row.get(5)?,
        config_json: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn row_to_log(row: &rusqlite::Row) -> rusqlite::Result<OutreachLog> {
    Ok(OutreachLog {
        id: row.get(0)?,
        contact_id: row.get(1)?,
        strategy_type: StrategyType::from_str_loose(&row.get::<_, String>(2)?).unwrap_or(StrategyType::PeriodicCheckin),
        executed_at: row.get(3)?,
        result: row.get(4)?,
        details: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::super::contacts::new_contact;
    use super::*;

    fn test_db_with_contact() -> (Database, Contact) {
        let db = Database::open_in_memory().unwrap();
        let contact =
            new_contact("Alice".into(), "Smith".into(), Some("alice@test.com".into()), None, Some("1990-03-15".into()), Importance::High);
        db.insert_contact(&contact).unwrap();
        (db, contact)
    }

    #[test]
    fn default_strategies_for_high_importance_with_birthday() {
        let (db, contact) = test_db_with_contact();
        let strategies = db.default_strategies_for(&contact).unwrap();

        let types: Vec<_> = strategies.iter().map(|s| s.strategy_type).collect();
        assert!(types.contains(&StrategyType::BirthdayMessage));
        assert!(types.contains(&StrategyType::NewsSearch));
        assert!(types.contains(&StrategyType::ProfileScrape));
        assert!(types.contains(&StrategyType::PeriodicCheckin));
        assert!(!types.contains(&StrategyType::DraftCongratulations));

        let checkin = strategies.iter().find(|s| s.strategy_type == StrategyType::PeriodicCheckin).unwrap();
        assert_eq!(checkin.interval_days, Some(30));
    }

    #[test]
    fn default_strategies_for_low_importance() {
        let db = Database::open_in_memory().unwrap();
        let contact = new_contact("Bob".into(), "Low".into(), None, None, None, Importance::Low);
        db.insert_contact(&contact).unwrap();

        let strategies = db.default_strategies_for(&contact).unwrap();
        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].strategy_type, StrategyType::PeriodicCheckin);
        assert_eq!(strategies[0].interval_days, Some(90));
    }

    #[test]
    fn default_strategies_no_birthday_skips_birthday_message() {
        let db = Database::open_in_memory().unwrap();
        let contact = new_contact("Charlie".into(), "NoBday".into(), None, None, None, Importance::VeryHigh);
        db.insert_contact(&contact).unwrap();

        let strategies = db.default_strategies_for(&contact).unwrap();
        let types: Vec<_> = strategies.iter().map(|s| s.strategy_type).collect();
        assert!(!types.contains(&StrategyType::BirthdayMessage));
        assert!(types.contains(&StrategyType::NewsSearch));
        assert!(types.contains(&StrategyType::DraftCongratulations));
        assert!(types.contains(&StrategyType::ProfileScrape));
        assert!(types.contains(&StrategyType::PeriodicCheckin));
    }

    #[test]
    fn assign_and_get_strategies() {
        let (db, contact) = test_db_with_contact();
        db.assign_default_strategies(&contact).unwrap();

        let strategies = db.get_strategies_for_contact(&contact.id).unwrap();
        assert!(!strategies.is_empty());
        assert!(strategies.iter().all(|s| s.contact_id == contact.id));
    }

    #[test]
    fn strategies_due_finds_unexecuted() {
        let (db, contact) = test_db_with_contact();
        db.assign_default_strategies(&contact).unwrap();

        let now = chrono::Utc::now();
        let due = db.strategies_due(&now).unwrap();
        assert!(!due.is_empty(), "Unexecuted strategies should be due");
    }

    #[test]
    fn mark_strategy_executed_and_due_check() {
        let (db, contact) = test_db_with_contact();
        db.assign_default_strategies(&contact).unwrap();

        let now = chrono::Utc::now();
        let due = db.strategies_due(&now).unwrap();
        let first = &due[0];

        db.mark_strategy_executed(&first.id, &now).unwrap();

        // Immediately after execution, the strategy should not be due
        let due_again = db.strategies_due(&now).unwrap();
        assert!(!due_again.iter().any(|s| s.id == first.id));
    }

    #[test]
    fn strategies_due_by_type() {
        let (db, contact) = test_db_with_contact();
        db.assign_default_strategies(&contact).unwrap();

        let now = chrono::Utc::now();
        let checkins = db.strategies_due_by_type(StrategyType::PeriodicCheckin, &now).unwrap();
        assert_eq!(checkins.len(), 1);
        assert_eq!(checkins[0].strategy_type, StrategyType::PeriodicCheckin);
    }

    #[test]
    fn set_strategies_replaces_existing() {
        let (db, contact) = test_db_with_contact();
        db.assign_default_strategies(&contact).unwrap();
        let before = db.get_strategies_for_contact(&contact.id).unwrap().len();
        assert!(before > 0);

        // Replace with a single strategy
        let replacement = OutreachStrategy {
            id: uuid::Uuid::new_v4().to_string(),
            contact_id: contact.id.clone(),
            strategy_type: StrategyType::PeriodicCheckin,
            enabled: true,
            interval_days: Some(7),
            last_executed: None,
            config_json: None,
            created_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        };
        db.set_strategies_for_contact(&contact.id, &[replacement]).unwrap();

        let after = db.get_strategies_for_contact(&contact.id).unwrap();
        assert_eq!(after.len(), 1);
    }

    #[test]
    fn outreach_logging() {
        let (db, contact) = test_db_with_contact();
        let log = OutreachLog {
            id: uuid::Uuid::new_v4().to_string(),
            contact_id: contact.id.clone(),
            strategy_type: StrategyType::BirthdayMessage,
            executed_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            result: "sent".into(),
            details: Some("Happy birthday message sent via email".into()),
        };
        db.log_outreach(&log).unwrap();

        let logs = db.get_outreach_logs(&contact.id).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].result, "sent");
    }

    #[test]
    fn cascade_delete_removes_strategies_and_logs() {
        let (db, contact) = test_db_with_contact();
        db.assign_default_strategies(&contact).unwrap();
        let log = OutreachLog {
            id: uuid::Uuid::new_v4().to_string(),
            contact_id: contact.id.clone(),
            strategy_type: StrategyType::PeriodicCheckin,
            executed_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            result: "sent".into(),
            details: None,
        };
        db.log_outreach(&log).unwrap();

        db.delete_contact(&contact.id).unwrap();
        assert!(db.get_strategies_for_contact(&contact.id).unwrap().is_empty());
        assert!(db.get_outreach_logs(&contact.id).unwrap().is_empty());
    }
}
