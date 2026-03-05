//! Contact profile entry storage and the `new_profile_entry` constructor.
//!
//! Extends `Database` with methods to store and retrieve structured observations
//! about a contact, filterable by category and bounded by a limit.

use super::db::Database;
use super::models::{ProfileCategory, ProfileEntry, ProfileSource};
use anyhow::Context;
use rusqlite::params;

impl Database {
    pub fn insert_profile_entry(&self, entry: &ProfileEntry) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO contact_profiles (id, contact_id, source, category, content, observed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![entry.id, entry.contact_id, entry.source.as_str(), entry.category.as_str(), entry.content, entry.observed_at],
        )?;
        Ok(())
    }

    pub fn get_profile_entries(&self, contact_id: &str, limit: usize) -> anyhow::Result<Vec<ProfileEntry>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, contact_id, source, category, content, observed_at \
             FROM contact_profiles WHERE contact_id = ?1 ORDER BY observed_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![contact_id, limit as i64], row_to_profile_entry)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to get profile entries")
    }

    pub fn get_profile_entries_by_category(
        &self,
        contact_id: &str,
        category: ProfileCategory,
        limit: usize,
    ) -> anyhow::Result<Vec<ProfileEntry>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, contact_id, source, category, content, observed_at \
             FROM contact_profiles WHERE contact_id = ?1 AND category = ?2 ORDER BY observed_at DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![contact_id, category.as_str(), limit as i64], row_to_profile_entry)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to get profile entries by category")
    }
}

fn row_to_profile_entry(row: &rusqlite::Row) -> rusqlite::Result<ProfileEntry> {
    Ok(ProfileEntry {
        id: row.get(0)?,
        contact_id: row.get(1)?,
        source: ProfileSource::from_str_loose(&row.get::<_, String>(2)?),
        category: ProfileCategory::from_str_loose(&row.get::<_, String>(3)?),
        content: row.get(4)?,
        observed_at: row.get(5)?,
    })
}

pub fn new_profile_entry(contact_id: &str, source: ProfileSource, category: ProfileCategory, content: String) -> ProfileEntry {
    ProfileEntry {
        id: uuid::Uuid::new_v4().to_string(),
        contact_id: contact_id.to_string(),
        source,
        category,
        content,
        observed_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::contacts::new_contact;
    use super::super::models::Importance;
    use super::*;

    fn test_db_with_contact() -> (Database, super::super::models::Contact) {
        let db = Database::open_in_memory().unwrap();
        let contact =
            new_contact("Alice".into(), "Smith".into(), Some("alice@test.com".into()), None, Some("1990-03-15".into()), Importance::High);
        db.insert_contact(&contact).unwrap();
        (db, contact)
    }

    #[test]
    fn insert_and_retrieve_profile_entry() {
        let (db, contact) = test_db_with_contact();
        let entry = new_profile_entry(&contact.id, ProfileSource::News, ProfileCategory::Achievement, "Won an award".into());
        db.insert_profile_entry(&entry).unwrap();

        let entries = db.get_profile_entries(&contact.id, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "Won an award");
        assert_eq!(entries[0].source, ProfileSource::News);
        assert_eq!(entries[0].category, ProfileCategory::Achievement);
    }

    #[test]
    fn get_profile_entries_respects_limit() {
        let (db, contact) = test_db_with_contact();
        for i in 0..5 {
            let entry = new_profile_entry(&contact.id, ProfileSource::News, ProfileCategory::News, format!("News item {i}"));
            db.insert_profile_entry(&entry).unwrap();
        }

        let entries = db.get_profile_entries(&contact.id, 3).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn get_profile_entries_by_category_filters_correctly() {
        let (db, contact) = test_db_with_contact();

        let work = new_profile_entry(&contact.id, ProfileSource::LinkedIn, ProfileCategory::WorkHistory, "Joined ACME Corp".into());
        let news = new_profile_entry(&contact.id, ProfileSource::News, ProfileCategory::News, "Featured in article".into());
        let edu = new_profile_entry(&contact.id, ProfileSource::Manual, ProfileCategory::Education, "PhD from MIT".into());
        db.insert_profile_entry(&work).unwrap();
        db.insert_profile_entry(&news).unwrap();
        db.insert_profile_entry(&edu).unwrap();

        let work_entries = db.get_profile_entries_by_category(&contact.id, ProfileCategory::WorkHistory, 10).unwrap();
        assert_eq!(work_entries.len(), 1);
        assert_eq!(work_entries[0].content, "Joined ACME Corp");

        let news_entries = db.get_profile_entries_by_category(&contact.id, ProfileCategory::News, 10).unwrap();
        assert_eq!(news_entries.len(), 1);
    }

    #[test]
    fn cascade_delete_removes_profile_entries() {
        let (db, contact) = test_db_with_contact();
        let entry = new_profile_entry(&contact.id, ProfileSource::Manual, ProfileCategory::Personal, "Loves hiking".into());
        db.insert_profile_entry(&entry).unwrap();

        assert_eq!(db.get_profile_entries(&contact.id, 10).unwrap().len(), 1);
        db.delete_contact(&contact.id).unwrap();
        assert!(db.get_profile_entries(&contact.id, 10).unwrap().is_empty());
    }
}
