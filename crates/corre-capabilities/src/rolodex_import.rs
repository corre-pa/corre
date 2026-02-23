use corre_db::contacts::new_contact;
use corre_db::{Contact, Database, Importance};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateAction {
    Skip,
    Merge,
    Replace,
}

/// Import contacts from a CSV file with standard column names.
/// Expected columns: first_name, last_name, email, phone, birthday, importance
/// (columns are matched case-insensitively, missing columns are treated as empty).
pub fn import_csv(db: &Database, path: &Path, dup_action: DuplicateAction) -> anyhow::Result<ImportResult> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).has_headers(true).from_path(path)?;
    let headers: Vec<String> = reader.headers()?.iter().map(|h| h.to_lowercase().trim().to_string()).collect();

    let col = |name: &str| headers.iter().position(|h| h == name);
    let first_name_idx = col("first_name").or_else(|| col("firstname")).or_else(|| col("first name"));
    let last_name_idx = col("last_name").or_else(|| col("lastname")).or_else(|| col("last name"));
    let email_idx = col("email").or_else(|| col("e-mail"));
    let phone_idx = col("phone").or_else(|| col("telephone")).or_else(|| col("mobile"));
    let birthday_idx = col("birthday").or_else(|| col("date of birth")).or_else(|| col("dob"));
    let importance_idx = col("importance").or_else(|| col("priority"));
    let nickname_idx = col("nickname");
    let notes_idx = col("notes");

    let mut result = ImportResult { imported: 0, skipped: 0, errors: Vec::new() };

    for (row_num, record) in reader.records().enumerate() {
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                result.errors.push(format!("Row {}: {e}", row_num + 2));
                continue;
            }
        };

        let get = |idx: Option<usize>| idx.and_then(|i| record.get(i)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

        let first_name = match get(first_name_idx) {
            Some(n) => n,
            None => {
                result.errors.push(format!("Row {}: missing first_name", row_num + 2));
                continue;
            }
        };
        let last_name = match get(last_name_idx) {
            Some(n) => n,
            None => {
                result.errors.push(format!("Row {}: missing last_name", row_num + 2));
                continue;
            }
        };
        let email = get(email_idx);
        let phone = get(phone_idx);
        let birthday = get(birthday_idx);
        let importance = get(importance_idx).map(|s| Importance::from_str_loose(&s)).unwrap_or(Importance::Medium);

        let mut contact = new_contact(first_name.clone(), last_name.clone(), email.clone(), phone, birthday, importance);
        contact.nickname = get(nickname_idx);
        contact.notes = get(notes_idx);

        match handle_duplicate(db, &contact, dup_action)? {
            DupResult::Insert => {
                db.insert_contact(&contact)?;
                db.assign_default_strategies(&contact)?;
                result.imported += 1;
            }
            DupResult::Updated => {
                result.imported += 1;
            }
            DupResult::Skipped => {
                result.skipped += 1;
            }
        }
    }

    Ok(result)
}

/// Import contacts from Google Contacts CSV export (Google Takeout format).
pub fn import_google(db: &Database, path: &Path, dup_action: DuplicateAction) -> anyhow::Result<ImportResult> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).has_headers(true).from_path(path)?;
    let headers: Vec<String> = reader.headers()?.iter().map(|h| h.to_lowercase().trim().to_string()).collect();

    let col = |name: &str| headers.iter().position(|h| h.contains(name));
    let first_name_idx = col("given name").or_else(|| col("first name"));
    let last_name_idx = col("family name").or_else(|| col("last name"));
    let email_idx = col("e-mail 1 - value").or_else(|| col("email"));
    let phone_idx = col("phone 1 - value").or_else(|| col("phone"));
    let birthday_idx = col("birthday");
    let nickname_idx = col("nickname");
    let notes_idx = col("notes");

    let mut result = ImportResult { imported: 0, skipped: 0, errors: Vec::new() };

    for (row_num, record) in reader.records().enumerate() {
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                result.errors.push(format!("Row {}: {e}", row_num + 2));
                continue;
            }
        };

        let get = |idx: Option<usize>| idx.and_then(|i| record.get(i)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

        let first_name = match get(first_name_idx) {
            Some(n) => n,
            None => continue, // Skip contacts without a name
        };
        let last_name = get(last_name_idx).unwrap_or_default();
        if last_name.is_empty() && first_name.is_empty() {
            continue;
        }

        let mut contact = new_contact(first_name, last_name, get(email_idx), get(phone_idx), get(birthday_idx), Importance::Medium);
        contact.nickname = get(nickname_idx);
        contact.notes = get(notes_idx);

        match handle_duplicate(db, &contact, dup_action)? {
            DupResult::Insert => {
                db.insert_contact(&contact)?;
                db.assign_default_strategies(&contact)?;
                result.imported += 1;
            }
            DupResult::Updated => result.imported += 1,
            DupResult::Skipped => result.skipped += 1,
        }
    }

    Ok(result)
}

/// Import contacts from Outlook CSV export format.
pub fn import_outlook(db: &Database, path: &Path, dup_action: DuplicateAction) -> anyhow::Result<ImportResult> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).has_headers(true).from_path(path)?;
    let headers: Vec<String> = reader.headers()?.iter().map(|h| h.to_lowercase().trim().to_string()).collect();

    let col = |name: &str| headers.iter().position(|h| h.contains(name));
    let first_name_idx = col("first name");
    let last_name_idx = col("last name");
    let email_idx = col("e-mail address").or_else(|| col("email"));
    let phone_idx = col("mobile phone").or_else(|| col("primary phone"));
    let birthday_idx = col("birthday");
    let notes_idx = col("notes");

    let mut result = ImportResult { imported: 0, skipped: 0, errors: Vec::new() };

    for (row_num, record) in reader.records().enumerate() {
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                result.errors.push(format!("Row {}: {e}", row_num + 2));
                continue;
            }
        };

        let get = |idx: Option<usize>| idx.and_then(|i| record.get(i)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

        let first_name = match get(first_name_idx) {
            Some(n) => n,
            None => continue,
        };
        let last_name = get(last_name_idx).unwrap_or_default();

        let mut contact = new_contact(first_name, last_name, get(email_idx), get(phone_idx), get(birthday_idx), Importance::Medium);
        contact.notes = get(notes_idx);

        match handle_duplicate(db, &contact, dup_action)? {
            DupResult::Insert => {
                db.insert_contact(&contact)?;
                db.assign_default_strategies(&contact)?;
                result.imported += 1;
            }
            DupResult::Updated => result.imported += 1,
            DupResult::Skipped => result.skipped += 1,
        }
    }

    Ok(result)
}

/// Import contacts from Facebook data download JSON.
pub fn import_facebook(db: &Database, path: &Path, dup_action: DuplicateAction) -> anyhow::Result<ImportResult> {
    let content = std::fs::read_to_string(path)?;
    let data: serde_json::Value = serde_json::from_str(&content)?;

    let mut result = ImportResult { imported: 0, skipped: 0, errors: Vec::new() };

    // Facebook exports contacts as an array under "friends_v2" or just a top-level array
    let contacts = data.as_array().or_else(|| data.get("friends_v2").and_then(|v| v.as_array()));

    let Some(contacts) = contacts else {
        anyhow::bail!("Could not find contacts array in Facebook export");
    };

    for entry in contacts {
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let parts: Vec<&str> = name.splitn(2, ' ').collect();
        let first_name = parts.first().unwrap_or(&"").to_string();
        let last_name = parts.get(1).unwrap_or(&"").to_string();

        if first_name.is_empty() {
            continue;
        }

        let mut contact = new_contact(first_name, last_name, None, None, None, Importance::Medium);
        contact.facebook = entry.get("contact_info").and_then(|v| v.as_str()).map(|s| s.to_string());

        match handle_duplicate(db, &contact, dup_action)? {
            DupResult::Insert => {
                db.insert_contact(&contact)?;
                db.assign_default_strategies(&contact)?;
                result.imported += 1;
            }
            DupResult::Updated => result.imported += 1,
            DupResult::Skipped => result.skipped += 1,
        }
    }

    Ok(result)
}

/// Import contacts from vCard (.vcf) format.
/// Parses the basic vCard properties: FN, N, EMAIL, TEL, BDAY, NOTE.
pub fn import_vcard(db: &Database, path: &Path, dup_action: DuplicateAction) -> anyhow::Result<ImportResult> {
    let content = std::fs::read_to_string(path)?;
    let mut result = ImportResult { imported: 0, skipped: 0, errors: Vec::new() };

    // Simple vCard parser: split by BEGIN:VCARD / END:VCARD blocks
    let blocks: Vec<&str> = content.split("BEGIN:VCARD").filter(|b| b.contains("END:VCARD")).collect();

    for block in blocks {
        let lines: Vec<&str> = block.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();

        let get_prop = |prefix: &str| -> Option<String> {
            lines
                .iter()
                .find(|l| l.to_uppercase().starts_with(prefix))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
                .filter(|s| !s.is_empty())
        };

        // N property format: LastName;FirstName;MiddleName;Prefix;Suffix
        let (first_name, last_name) = if let Some(n_val) = get_prop("N") {
            let parts: Vec<&str> = n_val.split(';').collect();
            let last = parts.first().unwrap_or(&"").trim().to_string();
            let first = parts.get(1).unwrap_or(&"").trim().to_string();
            if first.is_empty() && last.is_empty() {
                // Fall back to FN (full name)
                if let Some(fn_val) = get_prop("FN") {
                    let parts: Vec<&str> = fn_val.splitn(2, ' ').collect();
                    (parts.first().unwrap_or(&"").to_string(), parts.get(1).unwrap_or(&"").to_string())
                } else {
                    continue;
                }
            } else {
                (first, last)
            }
        } else if let Some(fn_val) = get_prop("FN") {
            let parts: Vec<&str> = fn_val.splitn(2, ' ').collect();
            (parts.first().unwrap_or(&"").to_string(), parts.get(1).unwrap_or(&"").to_string())
        } else {
            continue;
        };

        if first_name.is_empty() {
            continue;
        }

        let email = get_prop("EMAIL");
        let phone = get_prop("TEL");
        let birthday = get_prop("BDAY");
        let notes = get_prop("NOTE");

        let mut contact = new_contact(first_name, last_name, email, phone, birthday, Importance::Medium);
        contact.notes = notes;

        match handle_duplicate(db, &contact, dup_action)? {
            DupResult::Insert => {
                db.insert_contact(&contact)?;
                db.assign_default_strategies(&contact)?;
                result.imported += 1;
            }
            DupResult::Updated => result.imported += 1,
            DupResult::Skipped => result.skipped += 1,
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Duplicate handling
// ---------------------------------------------------------------------------

enum DupResult {
    Insert,
    Updated,
    Skipped,
}

fn handle_duplicate(db: &Database, contact: &Contact, action: DuplicateAction) -> anyhow::Result<DupResult> {
    // Check for existing contact by name or email
    let existing = db
        .find_by_name(&contact.first_name, &contact.last_name)?
        .or(contact.email.as_ref().and_then(|e| db.find_by_email(e).ok().flatten()));

    let Some(existing) = existing else {
        return Ok(DupResult::Insert);
    };

    match action {
        DuplicateAction::Skip => Ok(DupResult::Skipped),
        DuplicateAction::Replace => {
            let mut replacement = contact.clone();
            replacement.id = existing.id;
            db.update_contact(&replacement)?;
            Ok(DupResult::Updated)
        }
        DuplicateAction::Merge => {
            let mut merged = existing.clone();
            // Only overwrite fields if the incoming contact has a non-empty value
            if contact.email.is_some() {
                merged.email = contact.email.clone();
            }
            if contact.phone.is_some() {
                merged.phone = contact.phone.clone();
            }
            if contact.birthday.is_some() {
                merged.birthday = contact.birthday.clone();
            }
            if contact.nickname.is_some() {
                merged.nickname = contact.nickname.clone();
            }
            if contact.notes.is_some() {
                merged.notes = contact.notes.clone();
            }
            if contact.telegram.is_some() {
                merged.telegram = contact.telegram.clone();
            }
            if contact.whatsapp.is_some() {
                merged.whatsapp = contact.whatsapp.clone();
            }
            if contact.signal.is_some() {
                merged.signal = contact.signal.clone();
            }
            if contact.facebook.is_some() {
                merged.facebook = contact.facebook.clone();
            }
            if contact.linkedin.is_some() {
                merged.linkedin = contact.linkedin.clone();
            }
            db.update_contact(&merged)?;
            Ok(DupResult::Updated)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn import_csv_basic() {
        let db = test_db();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "first_name,last_name,email,phone,birthday,importance").unwrap();
        writeln!(file, "Alice,Smith,alice@test.com,+1234567890,1990-03-15,high").unwrap();
        writeln!(file, "Bob,Jones,bob@test.com,,1985-06-20,low").unwrap();

        let result = import_csv(&db, file.path(), DuplicateAction::Skip).unwrap();
        assert_eq!(result.imported, 2);
        assert_eq!(result.skipped, 0);
        assert!(result.errors.is_empty());

        let contacts = db.list_contacts().unwrap();
        assert_eq!(contacts.len(), 2);

        let alice = db.search_contacts("alice").unwrap();
        assert_eq!(alice[0].importance, Importance::High);
    }

    #[test]
    fn import_csv_skip_duplicates() {
        let db = test_db();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "first_name,last_name,email").unwrap();
        writeln!(file, "Alice,Smith,alice@test.com").unwrap();

        import_csv(&db, file.path(), DuplicateAction::Skip).unwrap();
        let result = import_csv(&db, file.path(), DuplicateAction::Skip).unwrap();
        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 1);
    }

    #[test]
    fn import_csv_merge_duplicates() {
        let db = test_db();
        let mut file1 = NamedTempFile::new().unwrap();
        writeln!(file1, "first_name,last_name,email").unwrap();
        writeln!(file1, "Alice,Smith,alice@test.com").unwrap();
        import_csv(&db, file1.path(), DuplicateAction::Skip).unwrap();

        let mut file2 = NamedTempFile::new().unwrap();
        writeln!(file2, "first_name,last_name,phone,birthday").unwrap();
        writeln!(file2, "Alice,Smith,+9876543210,1990-01-01").unwrap();

        let result = import_csv(&db, file2.path(), DuplicateAction::Merge).unwrap();
        assert_eq!(result.imported, 1);

        let alice = db.search_contacts("alice").unwrap();
        assert_eq!(alice[0].email.as_deref(), Some("alice@test.com")); // preserved
        assert_eq!(alice[0].phone.as_deref(), Some("+9876543210")); // merged
        assert_eq!(alice[0].birthday.as_deref(), Some("1990-01-01")); // merged
    }

    #[test]
    fn import_csv_missing_required_fields() {
        let db = test_db();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "first_name,last_name,email").unwrap();
        writeln!(file, ",Smith,missing@test.com").unwrap();
        writeln!(file, "Bob,,bob@test.com").unwrap();

        let result = import_csv(&db, file.path(), DuplicateAction::Skip).unwrap();
        assert_eq!(result.imported, 0);
        assert_eq!(result.errors.len(), 2);
    }

    #[test]
    fn import_vcard_basic() {
        let db = test_db();
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            "BEGIN:VCARD\nVERSION:3.0\nN:Smith;Alice;;;\nFN:Alice Smith\nEMAIL:alice@test.com\nTEL:+1234567890\nBDAY:1990-03-15\nEND:VCARD\n\
             BEGIN:VCARD\nVERSION:3.0\nN:Jones;Bob;;;\nFN:Bob Jones\nEMAIL:bob@test.com\nEND:VCARD\n"
        )
        .unwrap();

        let result = import_vcard(&db, file.path(), DuplicateAction::Skip).unwrap();
        assert_eq!(result.imported, 2);
        assert!(result.errors.is_empty());

        let contacts = db.list_contacts().unwrap();
        assert_eq!(contacts.len(), 2);
    }

    #[test]
    fn import_assigns_default_strategies() {
        let db = test_db();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "first_name,last_name,email,birthday,importance").unwrap();
        writeln!(file, "Alice,Smith,alice@test.com,1990-03-15,high").unwrap();

        import_csv(&db, file.path(), DuplicateAction::Skip).unwrap();
        let contacts = db.list_contacts().unwrap();
        let strategies = db.get_strategies_for_contact(&contacts[0].id).unwrap();
        assert!(!strategies.is_empty(), "Default strategies should be assigned on import");
    }
}
