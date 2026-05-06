//! Database-state matchers shared by step definitions.
//!
//! Step files describe expectations declaratively (key/value tables in feature files);
//! the matchers in this module translate those tables into typed comparisons against the
//! shared in-process [`Database`].

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use cucumber::gherkin::Table;
use tokio::sync::Mutex;

use corre_gym::db::{Database, Difficulty, ExerciseLevel, ExerciseSet, ExerciseType, HealthEntry, MeasurementType, Session};

// ── Table helpers ─────────────────────────────────────────────────────────────

/// Convert a 2-column gherkin data table into a key/value map. Whitespace is trimmed and
/// keys are lowercased so feature files can be relaxed about casing.
pub fn table_to_map(table: &Table) -> anyhow::Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    for (i, row) in table.rows.iter().enumerate() {
        if row.len() != 2 {
            anyhow::bail!("expected 2-column key/value table; row {i} has {} cells: {row:?}", row.len());
        }
        let key = row[0].trim().to_lowercase();
        let value = row[1].trim().to_string();
        if key.is_empty() {
            anyhow::bail!("row {i} has an empty key");
        }
        if out.insert(key.clone(), value).is_some() {
            anyhow::bail!("duplicate key in table: {key}");
        }
    }
    Ok(out)
}

// ── Exercise type lookup ──────────────────────────────────────────────────────

/// Resolve an exercise name (free-form, e.g. "bench press") to an `ExerciseType`.
///
/// First tries an exact case-insensitive match via `db.get_exercise_type_by_name` (which
/// already handles aliases internally). Falls back to titlecased and normalised forms so
/// "bench press", "Bench Press", and "BENCH PRESS" all resolve.
pub async fn resolve_exercise_type(db: &Arc<Mutex<Database>>, name: &str) -> anyhow::Result<ExerciseType> {
    let candidates =
        [name.to_string(), name.trim().to_string(), titlecase(name.trim()), name.trim().to_lowercase(), name.trim().to_uppercase()];
    let db = db.lock().await;
    for candidate in &candidates {
        if let Some(et) = db.get_exercise_type_by_name(candidate)? {
            return Ok(et);
        }
    }
    anyhow::bail!("no exercise_type named `{name}` (tried: {candidates:?})")
}

fn titlecase(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str().to_lowercase().as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Set matchers ──────────────────────────────────────────────────────────────

/// Fetch the most-recently logged set for a user, or `None` if the user has no sets.
pub async fn last_set(db: &Arc<Mutex<Database>>, user_id: i64) -> anyhow::Result<Option<ExerciseSet>> {
    let db = db.lock().await;
    let sets = db.list_sets_for_user(user_id, None, None).context("listing sets for user")?;
    Ok(sets.into_iter().next())
}

/// Total set count across the user's currently active session, or 0 if there is none.
pub async fn active_session_set_count(db: &Arc<Mutex<Database>>, user_id: i64) -> anyhow::Result<usize> {
    let db = db.lock().await;
    let Some(session) = db.get_active_session(user_id)? else {
        return Ok(0);
    };
    let entries = db.list_entries_for_session(session.id)?;
    let mut total = 0usize;
    for entry in &entries {
        total += db.list_sets_for_entry(entry.id)?.len();
    }
    Ok(total)
}

/// Active session for a user (or `None`).
pub async fn active_session(db: &Arc<Mutex<Database>>, user_id: i64) -> anyhow::Result<Option<Session>> {
    let db = db.lock().await;
    Ok(db.get_active_session(user_id)?)
}

/// Apply a single feature-table cell against an `ExerciseSet`.
///
/// Recognised field names (case-insensitive):
///
/// | Field                  | Comparison                                                                        |
/// |------------------------|-----------------------------------------------------------------------------------|
/// | `exercise_type`        | Resolve name → `ExerciseType`; assert set's type id matches or descends from it.  |
/// | `measurement_type`     | Parse via `MeasurementType::from_str_loose`; `assert_eq!`.                        |
/// | `count` / `reps`       | Parse `i32`; `assert_eq!(set.count.unwrap(), expected)`.                          |
/// | `value` / `weight_kg`  | Parse `f64`; `assert!((set.value - expected).abs() < 0.01)`.                      |
/// | `perceived_difficulty` / `difficulty` | Parse via `Difficulty::from_str_loose`; `assert_eq!`.                           |
/// | `comment`              | Substring match (case-insensitive).                                               |
async fn match_set_field(db: &Arc<Mutex<Database>>, set: &ExerciseSet, field: &str, value: &str) -> anyhow::Result<()> {
    match field {
        "exercise_type" => {
            let expected = resolve_exercise_type(db, value).await?;
            let descendant_match = if expected.level != ExerciseLevel::Variation {
                let db_guard = db.lock().await;
                db_guard.list_descendants(expected.id)?.iter().any(|d| d.id == set.exercise_type_id)
            } else {
                false
            };
            let direct_match = set.exercise_type_id == expected.id;
            if !direct_match && !descendant_match {
                let actual_name = {
                    let db_guard = db.lock().await;
                    db_guard
                        .get_exercise_type(set.exercise_type_id)?
                        .map(|et| et.name)
                        .unwrap_or_else(|| format!("id={}", set.exercise_type_id))
                };
                anyhow::bail!(
                    "exercise_type mismatch: feature wanted `{}` (id {}, level {}); set has `{}` (id {})",
                    expected.name,
                    expected.id,
                    expected.level,
                    actual_name,
                    set.exercise_type_id,
                );
            }
        }
        "measurement_type" => {
            let expected = MeasurementType::from_str_loose(value);
            anyhow::ensure!(
                set.measurement_type == expected,
                "measurement_type mismatch: expected `{value}`, got `{}`",
                set.measurement_type
            );
        }
        "count" | "reps" => {
            let expected: i32 = value.parse().with_context(|| format!("parsing {field} `{value}` as i32"))?;
            let actual = set.count.ok_or_else(|| anyhow!("set has no count, but feature expected {expected}"))?;
            anyhow::ensure!(actual == expected, "{field} mismatch: expected {expected}, got {actual}");
        }
        "value" | "weight_kg" => {
            let expected: f64 = value.parse().with_context(|| format!("parsing {field} `{value}` as f64"))?;
            anyhow::ensure!((set.value - expected).abs() < 0.01, "{field} mismatch: expected {expected:.2}, got {:.2}", set.value);
        }
        "perceived_difficulty" | "difficulty" => {
            let expected = Difficulty::from_str_loose(value);
            anyhow::ensure!(
                set.perceived_difficulty == expected,
                "perceived_difficulty mismatch: expected `{}`, got `{}`",
                expected,
                set.perceived_difficulty,
            );
        }
        "comment" => {
            let actual = set.comment.as_deref().unwrap_or("").to_lowercase();
            let needle = value.to_lowercase();
            anyhow::ensure!(
                actual.contains(&needle),
                "comment mismatch: expected substring `{value}`, got `{}`",
                set.comment.as_deref().unwrap_or(""),
            );
        }
        other => anyhow::bail!("unknown set field `{other}` in feature data table"),
    }
    Ok(())
}

/// Apply a 2-column key/value table to the most-recently logged set for a user.
pub async fn assert_last_set_matches(db: &Arc<Mutex<Database>>, user_id: i64, table: &Table) -> anyhow::Result<()> {
    let map = table_to_map(table)?;
    let set =
        last_set(db, user_id).await?.ok_or_else(|| anyhow!("expected a logged set for user {user_id}, but the sets table is empty"))?;
    for (key, value) in &map {
        match_set_field(db, &set, key, value.trim()).await.with_context(|| format!("checking field `{key}` on last set"))?;
    }
    Ok(())
}

/// Collect every set in the user's active session, ordered chronologically (oldest first).
/// Within a single second, falls back to insertion order via `id`. Returns an error if
/// there is no active session.
pub async fn active_session_sets(db: &Arc<Mutex<Database>>, user_id: i64) -> anyhow::Result<Vec<ExerciseSet>> {
    let db = db.lock().await;
    let session = db.get_active_session(user_id)?.ok_or_else(|| anyhow!("no active session for user {user_id}"))?;
    let entries = db.list_entries_for_session(session.id)?;
    let mut sets: Vec<ExerciseSet> = Vec::new();
    for entry in &entries {
        sets.extend(db.list_sets_for_entry(entry.id)?);
    }
    sets.sort_by(|a, b| a.logged_at.cmp(&b.logged_at).then(a.id.cmp(&b.id)));
    Ok(sets)
}

/// Apply a multi-row table where row 0 is the header and rows 1..N are sets to assert in
/// chronological order. Fails if the active-session set count differs from `N`.
pub async fn assert_active_session_sets_match(db: &Arc<Mutex<Database>>, user_id: i64, table: &Table) -> anyhow::Result<()> {
    let header = table.rows.first().ok_or_else(|| anyhow!("empty table"))?;
    let headers: Vec<String> = header.iter().map(|s| s.trim().to_lowercase()).collect();
    let expected_rows = &table.rows[1..];
    if expected_rows.is_empty() {
        anyhow::bail!("multi-row sets table needs at least one data row beneath the header");
    }

    let sets = active_session_sets(db, user_id).await?;
    anyhow::ensure!(
        sets.len() == expected_rows.len(),
        "expected exactly {} set(s) in active session, found {}",
        expected_rows.len(),
        sets.len(),
    );

    for (idx, row) in expected_rows.iter().enumerate() {
        anyhow::ensure!(
            row.len() == headers.len(),
            "set row {} has {} cells but header has {}: {row:?}",
            idx + 1,
            row.len(),
            headers.len(),
        );
        let set = &sets[idx];
        for (col_idx, header_name) in headers.iter().enumerate() {
            let cell = row[col_idx].trim();
            match_set_field(db, set, header_name, cell)
                .await
                .with_context(|| format!("set #{} (logged_at {}) field `{}`", idx + 1, set.logged_at, header_name))?;
        }
    }
    Ok(())
}

// ── Entry-state matchers (open / closed entries in the active session) ───────

/// Three-valued result for "what's the state of this user's entry for X?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryState {
    /// User has no entry at all for this exercise type or any descendant.
    None,
    /// Most-recent matching entry is still open (`end_timestamp IS NULL`).
    Open,
    /// Most-recent matching entry has been closed.
    Closed,
}

/// Number of open (unended) entries in the user's active session. Returns 0 when
/// there is no active session, which lets callers write `there are 0 open entries`
/// as a post-condition for end_session without special-casing the no-session case.
pub async fn open_entry_count(db: &Arc<Mutex<Database>>, user_id: i64) -> anyhow::Result<usize> {
    let db = db.lock().await;
    let Some(session) = db.get_active_session(user_id)? else {
        return Ok(0);
    };
    Ok(db.list_open_entries_for_session(session.id)?.len())
}

/// State of the most-recent entry in the user's active session whose sets reference
/// `exercise` (resolving the name with the same hierarchy rules `match_set_field`
/// uses — direct match, plus descendants when the resolved type is not a Variation).
pub async fn entry_state_for_exercise(db: &Arc<Mutex<Database>>, user_id: i64, exercise: &str) -> anyhow::Result<EntryState> {
    let target = resolve_exercise_type(db, exercise).await?;
    let db = db.lock().await;
    let Some(session) = db.get_active_session(user_id)? else {
        return Ok(EntryState::None);
    };
    let mut allowed = vec![target.id];
    if target.level != ExerciseLevel::Variation {
        allowed.extend(db.list_descendants(target.id)?.into_iter().map(|d| d.id));
    }
    let mut entries = db.list_entries_for_session(session.id)?;
    entries.sort_by(|a, b| b.start_timestamp.cmp(&a.start_timestamp).then(b.id.cmp(&a.id)));
    for entry in &entries {
        let sets = db.list_sets_for_entry(entry.id)?;
        if sets.iter().any(|s| allowed.contains(&s.exercise_type_id)) {
            return Ok(if entry.end_timestamp.is_none() { EntryState::Open } else { EntryState::Closed });
        }
    }
    Ok(EntryState::None)
}

/// Back-date every session, exercise_entry, and set belonging to `user_id` by
/// `hours`. Used by the session-continuity scenarios to simulate a long break
/// without actually sleeping.
pub async fn rewind_user_activity(db: &Arc<Mutex<Database>>, user_id: i64, hours: i64) -> anyhow::Result<()> {
    anyhow::ensure!(hours >= 0, "rewind hours must be non-negative, got {hours}");
    let db = db.lock().await;
    // String-formatting `hours` and `user_id` is safe here: both are i64 from the test
    // step regex and never come from user input. Avoids dragging rusqlite::params into
    // the e2e crate's surface area.
    let modifier = format!("'-{hours} hours'");
    let sessions_sql = format!(
        "UPDATE sessions \
         SET started_at = datetime(started_at, {modifier}), \
             ended_at   = CASE WHEN ended_at IS NULL THEN NULL ELSE datetime(ended_at, {modifier}) END \
         WHERE user_id = {user_id}"
    );
    let entries_sql = format!(
        "UPDATE exercise_entry \
         SET start_timestamp = datetime(start_timestamp, {modifier}), \
             end_timestamp   = CASE WHEN end_timestamp IS NULL THEN NULL ELSE datetime(end_timestamp, {modifier}) END \
         WHERE user_id = {user_id}"
    );
    let sets_sql = format!(
        "UPDATE sets SET logged_at = datetime(logged_at, {modifier}) \
         WHERE exercise_entry_id IN (SELECT id FROM exercise_entry WHERE user_id = {user_id})"
    );
    db.conn().execute(&sessions_sql, []).context("rewinding sessions")?;
    db.conn().execute(&entries_sql, []).context("rewinding exercise_entry rows")?;
    db.conn().execute(&sets_sql, []).context("rewinding sets")?;
    Ok(())
}

/// Loose match against the assistant's last reply for a "is this a new workout?"
/// style question. Intentionally permissive so prompt wording can drift; if a
/// genuinely-asking reply doesn't match, broaden the regex rather than tightening
/// the assistant prompt.
pub fn reply_asks_about_new_session(reply: &str) -> bool {
    let lower = reply.to_lowercase();
    let needles = [
        "new session",
        "new workout",
        "same session",
        "same workout",
        "picking up",
        "pick up where",
        "start a new",
        "is this a new",
        "are we continuing",
    ];
    needles.iter().any(|n| lower.contains(n))
}

// ── Health & goal matchers (used by health_logging.feature, goals.feature) ────

/// Most-recent active (unresolved) health entry for a user.
pub async fn last_active_health_entry(db: &Arc<Mutex<Database>>, user_id: i64) -> anyhow::Result<Option<HealthEntry>> {
    let db = db.lock().await;
    let mut entries = db.list_active_health_entries(user_id)?;
    entries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(entries.into_iter().next())
}
