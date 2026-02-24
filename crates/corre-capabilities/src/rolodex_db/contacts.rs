//! Contact CRUD operations and the `new_contact` constructor.
//!
//! Extends `Database` with methods for inserting, updating, deleting, and querying
//! contacts, including birthday lookup, importance filtering, and free-text search.

use super::db::Database;
use super::models::{Contact, ContactMethod, Importance};
use anyhow::Context;
use rusqlite::params;

impl Database {
    pub fn insert_contact(&self, contact: &Contact) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO contacts (id, first_name, last_name, nickname, email, phone, telegram, whatsapp, signal, facebook, linkedin, \
             preferred_contact_method, birthday, importance, notes, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                contact.id,
                contact.first_name,
                contact.last_name,
                contact.nickname,
                contact.email,
                contact.phone,
                contact.telegram,
                contact.whatsapp,
                contact.signal,
                contact.facebook,
                contact.linkedin,
                contact.preferred_contact_method.as_str(),
                contact.birthday,
                contact.importance.as_str(),
                contact.notes,
                contact.created_at,
                contact.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_contact(&self, contact: &Contact) -> anyhow::Result<()> {
        let rows = self.conn().execute(
            "UPDATE contacts SET first_name = ?1, last_name = ?2, nickname = ?3, email = ?4, phone = ?5, telegram = ?6, whatsapp = ?7, \
             signal = ?8, facebook = ?9, linkedin = ?10, preferred_contact_method = ?11, birthday = ?12, importance = ?13, notes = ?14, \
             updated_at = datetime('now') WHERE id = ?15",
            params![
                contact.first_name,
                contact.last_name,
                contact.nickname,
                contact.email,
                contact.phone,
                contact.telegram,
                contact.whatsapp,
                contact.signal,
                contact.facebook,
                contact.linkedin,
                contact.preferred_contact_method.as_str(),
                contact.birthday,
                contact.importance.as_str(),
                contact.notes,
                contact.id,
            ],
        )?;
        anyhow::ensure!(rows > 0, "Contact with id {} not found", contact.id);
        Ok(())
    }

    pub fn delete_contact(&self, id: &str) -> anyhow::Result<()> {
        let rows = self.conn().execute("DELETE FROM contacts WHERE id = ?1", params![id])?;
        anyhow::ensure!(rows > 0, "Contact with id {id} not found");
        Ok(())
    }

    pub fn get_contact(&self, id: &str) -> anyhow::Result<Option<Contact>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, first_name, last_name, nickname, email, phone, telegram, whatsapp, signal, facebook, linkedin, \
             preferred_contact_method, birthday, importance, notes, created_at, updated_at FROM contacts WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], row_to_contact)?;
        rows.next().transpose().context("Failed to read contact row")
    }

    pub fn list_contacts(&self) -> anyhow::Result<Vec<Contact>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, first_name, last_name, nickname, email, phone, telegram, whatsapp, signal, facebook, linkedin, \
             preferred_contact_method, birthday, importance, notes, created_at, updated_at FROM contacts ORDER BY last_name, first_name",
        )?;
        let rows = stmt.query_map([], row_to_contact)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list contacts")
    }

    /// Find contacts whose birthday matches the given month and day (MM-DD format from date).
    pub fn birthdays_on(&self, date: &chrono::NaiveDate) -> anyhow::Result<Vec<Contact>> {
        let mm_dd = date.format("%m-%d").to_string();
        let mut stmt = self.conn().prepare(
            "SELECT id, first_name, last_name, nickname, email, phone, telegram, whatsapp, signal, facebook, linkedin, \
             preferred_contact_method, birthday, importance, notes, created_at, updated_at \
             FROM contacts WHERE substr(birthday, 6) = ?1",
        )?;
        let rows = stmt.query_map(params![mm_dd], row_to_contact)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to query birthdays")
    }

    /// Find contacts at or above the given importance level.
    pub fn contacts_by_importance(&self, min_importance: Importance) -> anyhow::Result<Vec<Contact>> {
        let all = self.list_contacts()?;
        Ok(all.into_iter().filter(|c| c.importance >= min_importance).collect())
    }

    /// Search contacts by name or email (case-insensitive LIKE).
    pub fn search_contacts(&self, query: &str) -> anyhow::Result<Vec<Contact>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn().prepare(
            "SELECT id, first_name, last_name, nickname, email, phone, telegram, whatsapp, signal, facebook, linkedin, \
             preferred_contact_method, birthday, importance, notes, created_at, updated_at \
             FROM contacts WHERE first_name LIKE ?1 OR last_name LIKE ?1 OR email LIKE ?1 ORDER BY last_name, first_name",
        )?;
        let rows = stmt.query_map(params![pattern], row_to_contact)?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to search contacts")
    }

    /// Find a contact by first_name + last_name (case-insensitive), for dedup during import.
    pub fn find_by_name(&self, first_name: &str, last_name: &str) -> anyhow::Result<Option<Contact>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, first_name, last_name, nickname, email, phone, telegram, whatsapp, signal, facebook, linkedin, \
             preferred_contact_method, birthday, importance, notes, created_at, updated_at \
             FROM contacts WHERE lower(first_name) = lower(?1) AND lower(last_name) = lower(?2) LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![first_name, last_name], row_to_contact)?;
        rows.next().transpose().context("Failed to find contact by name")
    }

    /// Find a contact by email (case-insensitive), for dedup during import.
    pub fn find_by_email(&self, email: &str) -> anyhow::Result<Option<Contact>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, first_name, last_name, nickname, email, phone, telegram, whatsapp, signal, facebook, linkedin, \
             preferred_contact_method, birthday, importance, notes, created_at, updated_at \
             FROM contacts WHERE lower(email) = lower(?1) LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![email], row_to_contact)?;
        rows.next().transpose().context("Failed to find contact by email")
    }
}

fn row_to_contact(row: &rusqlite::Row) -> rusqlite::Result<Contact> {
    Ok(Contact {
        id: row.get(0)?,
        first_name: row.get(1)?,
        last_name: row.get(2)?,
        nickname: row.get(3)?,
        email: row.get(4)?,
        phone: row.get(5)?,
        telegram: row.get(6)?,
        whatsapp: row.get(7)?,
        signal: row.get(8)?,
        facebook: row.get(9)?,
        linkedin: row.get(10)?,
        preferred_contact_method: ContactMethod::from_str_loose(&row.get::<_, String>(11)?),
        birthday: row.get(12)?,
        importance: Importance::from_str_loose(&row.get::<_, String>(13)?),
        notes: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
    })
}

/// Helper to create a new Contact with a fresh UUID and timestamps.
pub fn new_contact(
    first_name: String,
    last_name: String,
    email: Option<String>,
    phone: Option<String>,
    birthday: Option<String>,
    importance: Importance,
) -> Contact {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    Contact {
        id: uuid::Uuid::new_v4().to_string(),
        first_name,
        last_name,
        nickname: None,
        email,
        phone,
        telegram: None,
        whatsapp: None,
        signal: None,
        facebook: None,
        linkedin: None,
        preferred_contact_method: ContactMethod::Email,
        birthday,
        importance,
        notes: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn sample_contact() -> Contact {
        new_contact("Alice".into(), "Smith".into(), Some("alice@example.com".into()), None, Some("1990-03-15".into()), Importance::High)
    }

    #[test]
    fn insert_and_get_contact() {
        let db = test_db();
        let contact = sample_contact();
        db.insert_contact(&contact).unwrap();

        let fetched = db.get_contact(&contact.id).unwrap().unwrap();
        assert_eq!(fetched.first_name, "Alice");
        assert_eq!(fetched.last_name, "Smith");
        assert_eq!(fetched.email.as_deref(), Some("alice@example.com"));
        assert_eq!(fetched.importance, Importance::High);
    }

    #[test]
    fn update_contact() {
        let db = test_db();
        let mut contact = sample_contact();
        db.insert_contact(&contact).unwrap();

        contact.first_name = "Alicia".into();
        contact.importance = Importance::VeryHigh;
        db.update_contact(&contact).unwrap();

        let fetched = db.get_contact(&contact.id).unwrap().unwrap();
        assert_eq!(fetched.first_name, "Alicia");
        assert_eq!(fetched.importance, Importance::VeryHigh);
    }

    #[test]
    fn delete_contact() {
        let db = test_db();
        let contact = sample_contact();
        db.insert_contact(&contact).unwrap();
        db.delete_contact(&contact.id).unwrap();
        assert!(db.get_contact(&contact.id).unwrap().is_none());
    }

    #[test]
    fn delete_nonexistent_contact_errors() {
        let db = test_db();
        assert!(db.delete_contact("nonexistent").is_err());
    }

    #[test]
    fn list_contacts_ordered() {
        let db = test_db();
        let mut c1 = new_contact("Zara".into(), "Bee".into(), None, None, None, Importance::Low);
        let mut c2 = new_contact("Alice".into(), "Ant".into(), None, None, None, Importance::Medium);
        c1.id = "1".into();
        c2.id = "2".into();
        db.insert_contact(&c1).unwrap();
        db.insert_contact(&c2).unwrap();

        let list = db.list_contacts().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].last_name, "Ant"); // sorted by last_name
        assert_eq!(list[1].last_name, "Bee");
    }

    #[test]
    fn birthdays_on_matches_month_day() {
        let db = test_db();
        let c1 = new_contact("Alice".into(), "A".into(), None, None, Some("1990-03-15".into()), Importance::Medium);
        let c2 = new_contact("Bob".into(), "B".into(), None, None, Some("1985-03-15".into()), Importance::Low);
        let c3 = new_contact("Charlie".into(), "C".into(), None, None, Some("1990-06-20".into()), Importance::High);
        db.insert_contact(&c1).unwrap();
        db.insert_contact(&c2).unwrap();
        db.insert_contact(&c3).unwrap();

        let march_15 = chrono::NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let bdays = db.birthdays_on(&march_15).unwrap();
        assert_eq!(bdays.len(), 2);
        assert!(bdays.iter().all(|c| c.birthday.as_deref().unwrap().ends_with("03-15")));
    }

    #[test]
    fn contacts_by_importance() {
        let db = test_db();
        db.insert_contact(&new_contact("Low".into(), "L".into(), None, None, None, Importance::Low)).unwrap();
        db.insert_contact(&new_contact("Med".into(), "M".into(), None, None, None, Importance::Medium)).unwrap();
        db.insert_contact(&new_contact("High".into(), "H".into(), None, None, None, Importance::High)).unwrap();

        let high_plus = db.contacts_by_importance(Importance::High).unwrap();
        assert_eq!(high_plus.len(), 1);
        assert_eq!(high_plus[0].first_name, "High");
    }

    #[test]
    fn search_contacts_by_name() {
        let db = test_db();
        db.insert_contact(&new_contact("Alice".into(), "Smith".into(), Some("a@x.com".into()), None, None, Importance::Medium)).unwrap();
        db.insert_contact(&new_contact("Bob".into(), "Jones".into(), Some("b@x.com".into()), None, None, Importance::Medium)).unwrap();

        let results = db.search_contacts("alice").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].first_name, "Alice");

        let results = db.search_contacts("x.com").unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn find_by_name_and_email() {
        let db = test_db();
        let contact = new_contact("Alice".into(), "Smith".into(), Some("alice@test.com".into()), None, None, Importance::Medium);
        db.insert_contact(&contact).unwrap();

        assert!(db.find_by_name("alice", "smith").unwrap().is_some());
        assert!(db.find_by_name("Alice", "Smith").unwrap().is_some());
        assert!(db.find_by_name("Bob", "Smith").unwrap().is_none());

        assert!(db.find_by_email("Alice@Test.com").unwrap().is_some());
        assert!(db.find_by_email("nobody@test.com").unwrap().is_none());
    }
}
