# Milestone 0: Database + Data Model

## Goal

Create the gym tracker crate with its complete SQLite schema, domain models, CRUD operations,
access control, and seed data. Pure data layer -- no networking, no LLM, no UI. Fully testable
with in-memory SQLite.

This is the foundation that every subsequent milestone builds on.

## Crate setup

Create `crates/corre-gym/` as a new workspace member.

### Cargo.toml

```toml
[package]
name = "corre-gym"
version.workspace = true
edition.workspace = true
description = "Voice-driven gym tracker and personal trainer"

[lib]
path = "src/lib.rs"

[[bin]]
name = "corre-gym"
path = "src/main.rs"

[dependencies]
corre-core = { workspace = true }
rusqlite = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
uuid = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tempfile = "3"
```

### Workspace Cargo.toml change

Add `"crates/corre-gym"` to the `[workspace] members` list.

## File structure

```
crates/corre-gym/
  Cargo.toml
  src/
    lib.rs                  pub mod db;
    main.rs                 Placeholder binary (prints "not yet implemented")
    db/
      mod.rs                Module root: re-exports Database, all model types, enums
      migrations.rs         SQL DDL strings + reference table seeding
      database.rs           Database struct (open, migrate, WAL mode)
      models.rs             All domain types and enums
      exercises.rs          Exercise catalogue CRUD + lookup table helpers
      users.rs              User CRUD + telegram_id lookup
      groups.rs             Group + group_members CRUD
      access.rs             Permission check helpers
      logs.rs               Exercise log + session CRUD
      goals.rs              Exercise goal CRUD
      schedules.rs          Schedule + schedule_exercises CRUD
      health.rs             Health entry CRUD
      conversation.rs       Conversation history CRUD
      progress.rs           Time-series queries + goal progress reporting
      seed.rs               Default exercise catalogue
  tests/
    helpers/
      mod.rs                Test seed helpers (load_seed_data, seed_database, seeded_db)
    fixtures/
      seed_data.json        250-session test dataset (~2 users, ~14 months)
    integration_tests.rs    Integration tests against seeded data
```

## Database schema

Follow the rolodex pattern: `Database` struct wrapping `rusqlite::Connection`, migrations
run on open via `CREATE TABLE IF NOT EXISTS`.

WAL mode and foreign keys are set as separate `PRAGMA` calls in `Database::open()` /
`run_migrations()` (not inside the DDL string), matching the rolodex pattern. WAL is needed
here because the gym tracker has concurrent readers (Axum server) and writers (Telegram bot),
unlike the rolodex which runs single-threaded.

### migrations.rs

The DDL string constant contains only `CREATE TABLE/INDEX` and `INSERT OR IGNORE` statements.
Pragmas are handled separately in `database.rs`.

```sql
-- Reference table: muscle groups
CREATE TABLE IF NOT EXISTS muscle_groups (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

INSERT OR IGNORE INTO muscle_groups (id, name) VALUES
    (1, 'chest'), (2, 'back'), (3, 'shoulders'), (4, 'biceps'),
    (5, 'triceps'), (6, 'forearms'), (7, 'quads'), (8, 'hamstrings'),
    (9, 'glutes'), (10, 'calves'), (11, 'core'), (12, 'full_body'),
    (13, 'cardio'), (14, 'other'), (15, 'traps'), (16, 'hip_flexors');

-- Reference table: measurement types
CREATE TABLE IF NOT EXISTS measurement_types (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

INSERT OR IGNORE INTO measurement_types (id, name) VALUES
    (1, 'weight_reps'), (2, 'time_based'), (3, 'distance_based'),
    (4, 'level_based'), (5, 'score_based');

-- Global exercise catalogue
CREATE TABLE IF NOT EXISTS exercises (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL UNIQUE COLLATE NOCASE,
    aliases             TEXT,          -- comma-separated alternative names for voice matching
    muscle_group_id     INTEGER NOT NULL REFERENCES muscle_groups(id),
    purpose             TEXT NOT NULL DEFAULT 'strength',
    measurement_type_id INTEGER NOT NULL DEFAULT 1 REFERENCES measurement_types(id),
    description         TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now'))
) WITHOUT ROWID;

-- Users identified by messaging platform IDs
CREATE TABLE IF NOT EXISTS users (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    telegram_id  TEXT UNIQUE,
    signal_id    TEXT UNIQUE,
    timezone     TEXT NOT NULL DEFAULT 'UTC',
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
) WITHOUT ROWID;

-- Groups for access control
CREATE TABLE IF NOT EXISTS groups (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
) WITHOUT ROWID;

-- Group membership with access levels
CREATE TABLE IF NOT EXISTS group_members (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    group_id   TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    level      TEXT NOT NULL DEFAULT 'read' CHECK (level IN ('read', 'write', 'admin')),
    granted_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, group_id)
) WITHOUT ROWID;

-- Per-user exercise goals
CREATE TABLE IF NOT EXISTS exercise_goals (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    exercise_id  TEXT NOT NULL REFERENCES exercises(id),
    target_value REAL NOT NULL,      -- always the primary metric for the exercise's measurement type
    start_date   TEXT NOT NULL,
    end_date     TEXT,
    achieved     INTEGER NOT NULL DEFAULT 0,
    notes        TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_goals_user ON exercise_goals(user_id, achieved);

-- Workout sessions (groups exercise logs within a single gym visit)
CREATE TABLE IF NOT EXISTS sessions (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    ended_at   TEXT,
    notes      TEXT
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id, started_at);

-- Exercise logs (the core tracking data)
CREATE TABLE IF NOT EXISTS exercise_logs (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    exercise_id   TEXT NOT NULL REFERENCES exercises(id),
    session_id    TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    logged_at     TEXT NOT NULL DEFAULT (datetime('now')),
    sets          INTEGER,
    reps          INTEGER,
    weight_kg     REAL,
    duration_secs INTEGER,
    distance_m    REAL,
    level         INTEGER,           -- also used for score_based measurements
    difficulty    TEXT NOT NULL DEFAULT 'medium' CHECK (difficulty IN ('easy', 'medium', 'hard', 'failure')),
    notes         TEXT
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_exercise_logs_user_date ON exercise_logs(user_id, logged_at);
CREATE INDEX IF NOT EXISTS idx_exercise_logs_session ON exercise_logs(session_id);
CREATE INDEX IF NOT EXISTS idx_exercise_logs_exercise ON exercise_logs(exercise_id, logged_at);

-- Workout schedules
CREATE TABLE IF NOT EXISTS schedules (
    id                   TEXT PRIMARY KEY,
    user_id              TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                 TEXT NOT NULL,
    cron_expr            TEXT NOT NULL,
    reminder_type        TEXT NOT NULL DEFAULT 'text' CHECK (reminder_type IN ('text', 'voice')),
    reminder_notice_mins INTEGER NOT NULL DEFAULT 30,
    enabled              INTEGER NOT NULL DEFAULT 1,
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at           TEXT NOT NULL DEFAULT (datetime('now'))
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_schedules_user ON schedules(user_id);

-- Exercises assigned to a schedule
CREATE TABLE IF NOT EXISTS schedule_exercises (
    schedule_id     TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    exercise_id     TEXT NOT NULL REFERENCES exercises(id),
    order_idx       INTEGER NOT NULL DEFAULT 0,
    target_sets     INTEGER,
    target_reps     INTEGER,
    target_weight_kg REAL,
    PRIMARY KEY (schedule_id, exercise_id)
) WITHOUT ROWID;

-- Health tracking (injuries, illness, wellbeing)
CREATE TABLE IF NOT EXISTS health_entries (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    entry_type  TEXT NOT NULL CHECK (entry_type IN ('injury', 'illness', 'wellbeing')),
    body_part   TEXT,
    severity    TEXT NOT NULL DEFAULT 'mild' CHECK (severity IN ('mild', 'moderate', 'severe')),
    description TEXT NOT NULL,
    started_at  TEXT NOT NULL DEFAULT (datetime('now')),
    resolved_at TEXT,
    notes       TEXT,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_health_user_date ON health_entries(user_id, started_at);

-- Conversation history for LLM context
CREATE TABLE IF NOT EXISTS conversation_history (
    id        TEXT PRIMARY KEY,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    platform  TEXT NOT NULL DEFAULT 'telegram' CHECK (platform IN ('telegram', 'signal', 'web')),
    role      TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content   TEXT NOT NULL,
    timestamp TEXT NOT NULL DEFAULT (datetime('now'))
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_conversation_user_time ON conversation_history(user_id, timestamp);
```

### Goal value interpretation

`exercise_goals.target_value` is `REAL`. Its meaning depends on the exercise's measurement type:

| MeasurementType | target_value represents | Example |
|-----------------|------------------------|---------|
| `weight_reps`   | weight in kg           | 100.0 (bench 100kg) |
| `time_based`    | duration in seconds    | 120.0 (2-min plank) |
| `distance_based`| distance in metres     | 5000.0 (5k run) |
| `level_based`   | level integer          | 10.0 |
| `score_based`   | score integer          | 50.0 |

The `goal_progress_report` computes `percentage = (current_value / target_value) * 100.0`,
guarded against zero division (returns 0.0 if target_value == 0.0).

### Schema versioning

For Milestone 0 (greenfield), `CREATE TABLE IF NOT EXISTS` is sufficient. Before Milestone 3
(dashboard with live data), introduce a `schema_version` table with numbered migration scripts
to support `ALTER TABLE` operations on deployed databases. Use sqlx for migrations.

## Domain models (models.rs)

### Enums

Only types that have code branches based on their value are modelled as enums.
Informational types (muscle group, purpose, severity) are stored as plain `String` fields —
they're validated by CHECK constraints in the DB and don't drive conditional logic in Rust.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementType {
    WeightReps,     // sets, reps, weight_kg
    TimeBased,      // duration_secs
    DistanceBased,  // distance_m, optionally duration_secs
    LevelBased,     // level (integer)
    ScoreBased,     // level used as score
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    Easy, Medium, Hard, Failure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthEntryType {
    Injury, Illness, Wellbeing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    Read, Write, Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderType {
    Text, Voice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationRole {
    User, Assistant, System,
}
```

Each enum implements `as_str() -> &'static str` and `from_str_loose(s: &str) -> Self`
following the rolodex pattern. Display impl delegates to `as_str()`. Defaults for
`from_str_loose`: `MeasurementType::WeightReps`, `Difficulty::Medium`,
`HealthEntryType::Wellbeing`, `AccessLevel::Read`, `ReminderType::Text`,
`ConversationRole::User`.

### Time-series and goal progress types

```rust
/// A single data point in a time series, suitable for charting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint {
    pub date: String,            // ISO 8601 date
    pub value: f64,              // primary metric (weight, duration, distance, level)
}

/// A labelled time series for one exercise within a grouped query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeries {
    pub exercise_id: String,
    pub exercise_name: String,
    pub measurement_type: MeasurementType,
    pub points: Vec<TimeSeriesPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Achieved,
    Failed,     // end_date passed without achievement
}

/// Progress toward a single exercise goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalProgress {
    pub goal: ExerciseGoal,
    pub exercise_name: String,
    pub status: GoalStatus,
    pub current_value: Option<f64>,
    pub percentage: f64,             // 0.0–100.0+
}

/// Summary view of a session for list displays.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session: Session,
    pub exercise_count: i32,
    pub duration_mins: Option<i32>,
}
```

### Structs

```rust
/// Lightweight exercise record — stores the FK id, no JOINed fields.
/// Used by most CRUD operations.
pub struct Exercise {
    pub id: String,
    pub name: String,
    pub aliases: Option<String>,         // comma-separated alternative names
    pub muscle_group_id: i32,            // FK to muscle_groups(id)
    pub purpose: String,                 // plain string (strength, hypertrophy, etc.)
    pub measurement_type: MeasurementType,
    pub description: Option<String>,
    pub created_at: String,
}

/// Exercise with resolved muscle group name. Returned by queries that JOIN
/// to the muscle_groups table (e.g. search, list_by_muscle_group, progress).
pub struct FullExercise {
    pub exercise: Exercise,
    pub muscle_group: String,            // resolved from muscle_groups.name
}

pub struct User {
    pub id: String,
    pub name: String,
    pub telegram_id: Option<String>,
    pub signal_id: Option<String>,
    pub timezone: String,
    pub created_at: String,
    pub updated_at: String,
}

pub struct Group {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

pub struct GroupMember {
    pub user_id: String,
    pub group_id: String,
    pub level: AccessLevel,
    pub granted_at: String,
}

pub struct ExerciseGoal {
    pub id: String,
    pub user_id: String,
    pub exercise_id: String,
    pub target_value: f64,
    pub start_date: String,
    pub end_date: Option<String>,
    pub achieved: bool,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct Session {
    pub id: String,
    pub user_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub notes: Option<String>,
}

pub struct ExerciseLog {
    pub id: String,
    pub user_id: String,
    pub exercise_id: String,
    pub session_id: Option<String>,
    pub logged_at: String,
    pub sets: Option<i32>,
    pub reps: Option<i32>,
    pub weight_kg: Option<f64>,
    pub duration_secs: Option<i32>,
    pub distance_m: Option<f64>,
    pub level: Option<i32>,
    pub difficulty: Difficulty,
    pub notes: Option<String>,
}

pub struct Schedule {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub cron_expr: String,
    pub reminder_type: ReminderType,
    pub reminder_notice_mins: i32,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub struct ScheduleExercise {
    pub schedule_id: String,
    pub exercise_id: String,
    pub order_idx: i32,
    pub target_sets: Option<i32>,
    pub target_reps: Option<i32>,
    pub target_weight_kg: Option<f64>,
}

pub struct HealthEntry {
    pub id: String,
    pub user_id: String,
    pub entry_type: HealthEntryType,
    pub body_part: Option<String>,
    pub severity: String,                // plain string (mild, moderate, severe)
    pub description: String,
    pub started_at: String,
    pub resolved_at: Option<String>,
    pub notes: Option<String>,
    pub updated_at: String,
}

pub struct ConversationMessage {
    pub id: String,
    pub user_id: String,
    pub platform: String,
    pub role: ConversationRole,
    pub content: String,
    pub timestamp: String,
}
```

All structs derive `Debug, Clone, Serialize, Deserialize`.

### Constructor functions

Following the rolodex pattern, each major type has a `new_*` constructor that generates
a UUID and sets timestamps:

```rust
pub fn new_user(name: &str, telegram_id: Option<&str>, timezone: &str) -> User;
pub fn new_session(user_id: &str, notes: Option<&str>) -> Session;
pub fn new_exercise_log(user_id: &str, exercise_id: &str, session_id: Option<&str>) -> ExerciseLog;
pub fn new_exercise_goal(user_id: &str, exercise_id: &str, target_value: f64) -> ExerciseGoal;
pub fn new_health_entry(user_id: &str, entry_type: HealthEntryType, description: &str) -> HealthEntry;
pub fn new_conversation_message(user_id: &str, platform: &str, role: ConversationRole, content: &str) -> ConversationMessage;
```

## CRUD operations

Each module defines `row_to_*` free functions (e.g. `row_to_exercise`, `row_to_user`) for
reusable row-to-struct mapping, following the rolodex pattern.

### database.rs

```rust
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> anyhow::Result<Self>;
    pub fn open_in_memory() -> anyhow::Result<Self>;
    fn run_migrations(&self) -> anyhow::Result<()>;  // sets PRAGMA WAL + FK, then DDL batch
    pub fn conn(&self) -> &Connection;
}
```

`run_migrations()` executes pragmas first, then the DDL batch:
```rust
fn run_migrations(&self) -> anyhow::Result<()> {
    self.conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    self.conn.execute_batch(MIGRATIONS)?;  // the DDL string constant
    Ok(())
}
```

### exercises.rs

```rust
impl Database {
    // Lookup table helpers
    pub fn list_muscle_groups(&self) -> Result<Vec<(i32, String)>>;
    pub fn list_measurement_types(&self) -> Result<Vec<(i32, String)>>;
    pub fn muscle_group_id(&self, name: &str) -> Result<Option<i32>>;
    pub fn measurement_type_id(&self, name: &str) -> Result<Option<i32>>;

    // Exercise CRUD
    pub fn insert_exercise(&self, exercise: &Exercise) -> Result<()>;
    pub fn get_exercise(&self, id: &str) -> Result<Option<Exercise>>;
    pub fn get_exercise_by_name(&self, name: &str) -> Result<Option<Exercise>>;
    pub fn search_exercises(&self, query: &str) -> Result<Vec<Exercise>>;  // LIKE %query% on name + aliases
    pub fn list_exercises(&self) -> Result<Vec<Exercise>>;
    pub fn list_exercises_by_muscle_group(&self, muscle_group: &str) -> Result<Vec<FullExercise>>;
    pub fn update_exercise(&self, exercise: &Exercise) -> Result<()>;
    pub fn delete_exercise(&self, id: &str) -> Result<()>;
    pub fn seed_exercises(&self) -> Result<usize>;  // returns count inserted
}
```

Basic queries return `Exercise` (no JOIN needed). Queries that filter or display by
muscle group return `FullExercise` (JOINs to resolve the name):

```sql
-- Exercise (no JOIN)
SELECT e.id, e.name, e.aliases, e.muscle_group_id, e.purpose,
       mt.name, e.description, e.created_at
FROM exercises e
JOIN measurement_types mt ON e.measurement_type_id = mt.id

-- FullExercise (with muscle group)
SELECT e.id, e.name, e.aliases, e.muscle_group_id, e.purpose,
       mt.name, e.description, e.created_at, mg.name
FROM exercises e
JOIN muscle_groups mg ON e.muscle_group_id = mg.id
JOIN measurement_types mt ON e.measurement_type_id = mt.id

-- search_exercises: LIKE on name and aliases
WHERE e.name LIKE '%' || ?1 || '%' OR e.aliases LIKE '%' || ?1 || '%'
```

INSERT queries use subqueries to resolve FK ids from names:

```sql
INSERT INTO exercises (id, name, aliases, muscle_group_id, purpose, measurement_type_id, description, created_at)
VALUES (?1, ?2, ?3,
    (SELECT id FROM muscle_groups WHERE name = ?4),
    ?5,
    (SELECT id FROM measurement_types WHERE name = ?6),
    ?7, ?8)
```

### users.rs

```rust
impl Database {
    pub fn insert_user(&self, user: &User) -> Result<()>;
    pub fn get_user(&self, id: &str) -> Result<Option<User>>;
    pub fn get_user_by_telegram_id(&self, telegram_id: &str) -> Result<Option<User>>;
    pub fn get_user_by_signal_id(&self, signal_id: &str) -> Result<Option<User>>;
    pub fn update_user(&self, user: &User) -> Result<()>;
    pub fn delete_user(&self, id: &str) -> Result<()>;
    pub fn list_users(&self) -> Result<Vec<User>>;
}
```

### groups.rs

```rust
impl Database {
    pub fn insert_group(&self, group: &Group) -> Result<()>;
    pub fn get_group(&self, id: &str) -> Result<Option<Group>>;
    pub fn list_groups(&self) -> Result<Vec<Group>>;
    pub fn update_group(&self, group: &Group) -> Result<()>;
    pub fn delete_group(&self, id: &str) -> Result<()>;

    pub fn add_member(&self, user_id: &str, group_id: &str, level: AccessLevel) -> Result<()>;
    pub fn remove_member(&self, user_id: &str, group_id: &str) -> Result<()>;
    pub fn set_member_level(&self, user_id: &str, group_id: &str, level: AccessLevel) -> Result<()>;
    pub fn list_group_members(&self, group_id: &str) -> Result<Vec<(User, AccessLevel)>>;
    pub fn list_user_groups(&self, user_id: &str) -> Result<Vec<(Group, AccessLevel)>>;
}
```

### access.rs

```rust
impl Database {
    /// True if actor == target, or actor has read/write/admin on any group containing target.
    pub fn can_read(&self, actor_id: &str, target_id: &str) -> Result<bool>;

    /// True if actor == target, or actor has write/admin on any group containing target.
    pub fn can_write(&self, actor_id: &str, target_id: &str) -> Result<bool>;

    /// True if actor has admin level on the specified group.
    pub fn can_admin_group(&self, actor_id: &str, group_id: &str) -> Result<bool>;
}
```

All three access queries:

```sql
-- can_read: actor == target OR actor has read/write/admin in a shared group
SELECT 1 FROM group_members gm1
JOIN group_members gm2 ON gm1.group_id = gm2.group_id
WHERE gm1.user_id = ?actor AND gm2.user_id = ?target
  AND gm1.level IN ('read', 'write', 'admin')
LIMIT 1

-- can_write: actor == target OR actor has write/admin in a shared group
SELECT 1 FROM group_members gm1
JOIN group_members gm2 ON gm1.group_id = gm2.group_id
WHERE gm1.user_id = ?actor AND gm2.user_id = ?target
  AND gm1.level IN ('write', 'admin')
LIMIT 1

-- can_admin_group: actor has admin on the specific group
SELECT 1 FROM group_members
WHERE user_id = ?actor AND group_id = ?group AND level = 'admin'
LIMIT 1
```

### logs.rs

```rust
impl Database {
    // Sessions
    pub fn start_session(&self, user_id: &str, notes: Option<&str>) -> Result<Session>;
    pub fn end_session(&self, session_id: &str) -> Result<()>;
    pub fn get_session(&self, id: &str) -> Result<Option<Session>>;
    pub fn get_active_session(&self, user_id: &str) -> Result<Option<Session>>;
    pub fn list_sessions(&self, user_id: &str, from: Option<&str>, to: Option<&str>) -> Result<Vec<Session>>;
    pub fn list_session_summaries(&self, user_id: &str, from: Option<&str>, to: Option<&str>) -> Result<Vec<SessionSummary>>;

    // Exercise logs
    pub fn insert_log(&self, log: &ExerciseLog) -> Result<()>;
    pub fn update_log(&self, log: &ExerciseLog) -> Result<()>;
    pub fn get_logs_for_session(&self, session_id: &str) -> Result<Vec<ExerciseLog>>;
    pub fn get_logs_for_user(&self, user_id: &str, from: Option<&str>, to: Option<&str>) -> Result<Vec<ExerciseLog>>;
    pub fn get_logs_for_exercise(&self, user_id: &str, exercise_id: &str, limit: usize) -> Result<Vec<ExerciseLog>>;
    pub fn get_recent_logs(&self, user_id: &str, days: i32) -> Result<Vec<ExerciseLog>>;
    pub fn delete_log(&self, id: &str) -> Result<()>;

    // Aggregations
    pub fn personal_record(&self, user_id: &str, exercise_id: &str) -> Result<Option<ExerciseLog>>;
    pub fn session_count_by_week(&self, user_id: &str, weeks: i32) -> Result<Vec<(String, i32)>>;
}
```

The `list_session_summaries` query JOINs to exercise_logs to avoid N+1:

```sql
SELECT s.id, s.user_id, s.started_at, s.ended_at, s.notes,
       COUNT(DISTINCT el.id) AS exercise_count,
       CAST((julianday(s.ended_at) - julianday(s.started_at)) * 24 * 60 AS INTEGER) AS duration_mins
FROM sessions s
LEFT JOIN exercise_logs el ON el.session_id = s.id
WHERE s.user_id = ?1
  AND (?2 IS NULL OR s.started_at >= ?2)
  AND (?3 IS NULL OR s.started_at <= ?3)
GROUP BY s.id
ORDER BY s.started_at DESC
```

The `personal_record` query branches on the exercise's measurement type:
- `weight_reps`: row with MAX `weight_kg`
- `time_based`: row with MAX `duration_secs`
- `distance_based`: row with MAX `distance_m`
- `level_based`/`score_based`: row with MAX `level`

### goals.rs

```rust
impl Database {
    pub fn insert_goal(&self, goal: &ExerciseGoal) -> Result<()>;
    pub fn get_goal(&self, id: &str) -> Result<Option<ExerciseGoal>>;
    pub fn list_active_goals(&self, user_id: &str) -> Result<Vec<ExerciseGoal>>;
    pub fn list_goals_in_period(&self, user_id: &str, from: &str, to: &str) -> Result<Vec<ExerciseGoal>>;
    pub fn mark_goal_achieved(&self, id: &str) -> Result<()>;
    pub fn delete_goal(&self, id: &str) -> Result<()>;
}
```

### schedules.rs

```rust
impl Database {
    pub fn insert_schedule(&self, schedule: &Schedule) -> Result<()>;
    pub fn get_schedule(&self, id: &str) -> Result<Option<Schedule>>;
    pub fn list_schedules(&self, user_id: &str) -> Result<Vec<Schedule>>;
    pub fn update_schedule(&self, schedule: &Schedule) -> Result<()>;
    pub fn delete_schedule(&self, id: &str) -> Result<()>;
    pub fn toggle_schedule(&self, id: &str, enabled: bool) -> Result<()>;

    pub fn add_schedule_exercise(&self, entry: &ScheduleExercise) -> Result<()>;
    pub fn list_schedule_exercises(&self, schedule_id: &str) -> Result<Vec<ScheduleExercise>>;
    pub fn remove_schedule_exercise(&self, schedule_id: &str, exercise_id: &str) -> Result<()>;
    pub fn clear_schedule_exercises(&self, schedule_id: &str) -> Result<()>;
}
```

### health.rs

```rust
impl Database {
    pub fn insert_health_entry(&self, entry: &HealthEntry) -> Result<()>;
    pub fn get_health_entry(&self, id: &str) -> Result<Option<HealthEntry>>;
    pub fn list_active_health_entries(&self, user_id: &str) -> Result<Vec<HealthEntry>>;
    pub fn list_health_entries_by_type(&self, user_id: &str, entry_type: HealthEntryType, limit: usize) -> Result<Vec<HealthEntry>>;
    pub fn list_health_history(&self, user_id: &str, limit: usize) -> Result<Vec<HealthEntry>>;
    pub fn resolve_health_entry(&self, id: &str) -> Result<()>;
    pub fn update_health_entry(&self, entry: &HealthEntry) -> Result<()>;
}
```

### conversation.rs

```rust
impl Database {
    pub fn insert_message(&self, msg: &ConversationMessage) -> Result<()>;
    pub fn get_recent_messages(&self, user_id: &str, limit: usize) -> Result<Vec<ConversationMessage>>;
    pub fn get_recent_messages_for_platform(&self, user_id: &str, platform: &str, limit: usize) -> Result<Vec<ConversationMessage>>;
    pub fn prune_old_messages(&self, user_id: &str, keep_last: usize) -> Result<usize>;
}
```

### progress.rs

Time-series and goal progress queries for the dashboard charts. Default date ranges are
computed in Rust using chrono (respecting the user's timezone), not in SQL.

```rust
impl Database {
    /// Time series for a single exercise. Returns one data point per day,
    /// using the best set from each day (MAX weight for weight_reps, MAX duration
    /// for time_based, MAX distance for distance_based).
    /// Defaults: from = 1 year ago, to = today (computed via chrono).
    pub fn exercise_time_series(
        &self,
        user_id: &str,
        exercise_id: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Vec<TimeSeriesPoint>>;

    /// Time series for all exercises in a muscle group. Returns one TimeSeries
    /// per exercise that has data in the period.
    pub fn muscle_group_time_series(
        &self,
        user_id: &str,
        muscle_group: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Vec<TimeSeries>>;

    /// Time series for all exercises that have an active or recently-completed goal.
    pub fn goal_time_series(
        &self,
        user_id: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Vec<TimeSeries>>;

    /// Goal progress report for a period. Lists every goal whose date range
    /// overlaps [from, to], with current progress percentage and status.
    /// Default period: last 1 year (computed via chrono).
    pub fn goal_progress_report(
        &self,
        user_id: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Vec<GoalProgress>>;
}
```

The `exercise_time_series` SQL branches on the exercise's measurement type. For `weight_reps`:

```sql
SELECT date(el.logged_at) AS date,
       MAX(el.weight_kg) AS value
FROM exercise_logs el
WHERE el.user_id = ?1 AND el.exercise_id = ?2
  AND el.logged_at >= ?3 AND el.logged_at <= ?4
  AND el.weight_kg IS NOT NULL
GROUP BY date(el.logged_at)
ORDER BY date(el.logged_at)
```

For `time_based`, substitute `MAX(el.duration_secs)`. For `distance_based`, `MAX(el.distance_m)`.

The `muscle_group_time_series` function first discovers which exercises have data in the
period via a discovery query, then calls `exercise_time_series` for each one:

```sql
SELECT DISTINCT e.id, e.name, mt.name AS measurement_type
FROM exercises e
JOIN muscle_groups mg ON e.muscle_group_id = mg.id
JOIN measurement_types mt ON e.measurement_type_id = mt.id
JOIN exercise_logs el ON el.exercise_id = e.id
WHERE el.user_id = ?1 AND mg.name = ?2
  AND el.logged_at >= ?3 AND el.logged_at <= ?4
```

The `goal_progress_report` function queries all overlapping goals, then for each goal,
looks up the latest measurement value and computes percentage:

```sql
-- Find all goals overlapping the period
SELECT g.*, e.name AS exercise_name, mt.name AS measurement_type
FROM exercise_goals g
JOIN exercises e ON g.exercise_id = e.id
JOIN measurement_types mt ON e.measurement_type_id = mt.id
WHERE g.user_id = ?1
  AND g.start_date <= ?3 AND (g.end_date IS NULL OR g.end_date >= ?2)
ORDER BY g.start_date

-- For each goal, get the latest best value (example for weight_reps)
SELECT MAX(el.weight_kg) FROM exercise_logs el
WHERE el.user_id = ?1 AND el.exercise_id = ?2
  AND el.logged_at >= ?3 AND el.logged_at <= ?4
```

Status derivation:
- `percentage = if target_value == 0.0 { 0.0 } else { (current_value / target_value) * 100.0 }`
- `GoalStatus::Achieved` if `achieved == true` or `percentage >= 100.0`
- `GoalStatus::Failed` if `end_date < today && !achieved`
- `GoalStatus::Active` otherwise

## Seed data (seed.rs)

~60 exercises across all muscle groups. Example subset:

| Name | Muscle Group | Purpose | Measurement |
|------|-------------|---------|-------------|
| Barbell Bench Press | chest | strength | weight_reps |
| Incline Dumbbell Press | chest | hypertrophy | weight_reps |
| Cable Fly | chest | hypertrophy | weight_reps |
| Push-up | chest | endurance | weight_reps |
| Barbell Back Squat | quads | strength | weight_reps |
| Leg Press | quads | strength | weight_reps |
| Leg Extension | quads | hypertrophy | weight_reps |
| Romanian Deadlift | hamstrings | strength | weight_reps |
| Leg Curl | hamstrings | hypertrophy | weight_reps |
| Conventional Deadlift | back | strength | weight_reps |
| Barbell Row | back | strength | weight_reps |
| Pull-up | back | strength | weight_reps |
| Lat Pulldown | back | hypertrophy | weight_reps |
| Seated Cable Row | back | hypertrophy | weight_reps |
| Overhead Press | shoulders | strength | weight_reps |
| Lateral Raise | shoulders | hypertrophy | weight_reps |
| Face Pull | shoulders | hypertrophy | weight_reps |
| Barbell Shrug | traps | strength | weight_reps |
| Barbell Curl | biceps | hypertrophy | weight_reps |
| Hammer Curl | biceps | hypertrophy | weight_reps |
| Tricep Pushdown | triceps | hypertrophy | weight_reps |
| Overhead Tricep Extension | triceps | hypertrophy | weight_reps |
| Dip | triceps | strength | weight_reps |
| Calf Raise | calves | hypertrophy | weight_reps |
| Plank | core | endurance | time_based |
| Hanging Leg Raise | core | strength | weight_reps |
| Ab Wheel Rollout | core | strength | weight_reps |
| Running | cardio | cardio | distance_based |
| Cycling | cardio | cardio | distance_based |
| Rowing Machine | cardio | cardio | distance_based |
| Hip Thrust | glutes | strength | weight_reps |
| Bulgarian Split Squat | quads | strength | weight_reps |
| ... | ... | ... | ... |

The `seed_exercises()` method uses `INSERT OR IGNORE` with subqueries to resolve
`muscle_group_id` and `measurement_type_id` from the lookup tables, so it's safe to call
repeatedly. Seed data includes aliases, e.g. `"flat bench,bench"` for Barbell Bench Press.

## Test data and integration tests

### Test data generation

A sub-agent generates a JSON fixture file at `tests/fixtures/seed_data.json` containing
a realistic dataset of ~250 sessions across 2 users over ~14 months. The dataset encodes:

- Progressive overload (weights increasing ~2.5kg/week on compounds)
- A plateau period (weeks 12–15, no weight increase)
- A deload week (week 16, all weights reduced 20%)
- An injury gap (~2 weeks, shoulder injury, with matching health entry)
- Goals met (bench press 100kg by month 6, achieved) and missed (5k run in 22 min by month 12, failed)
- Varied muscle groups and exercise types (weight_reps, time_based, distance_based)
- Sufficient time_based data (Plank: ~50 data points) and distance_based data (Running: ~40 data points) for time-series query coverage
- 2–4 conversation messages per session

### Test data structure

```json
{
  "users": [...],
  "exercises": [...],
  "groups": [...],
  "group_members": [...],
  "sessions": [
    {
      "id": "sess-001",
      "user_id": "user-1",
      "started_at": "2025-04-01 06:30:00",
      "ended_at": "2025-04-01 07:45:00",
      "notes": "Chest and triceps day",
      "logs": [
        {
          "id": "log-001",
          "exercise_id": "ex-bench-press",
          "sets": 4, "reps": 8, "weight_kg": 80.0,
          "difficulty": "medium", "notes": null
        }
      ],
      "conversation": [
        { "role": "user", "content": "Starting chest day, bench press 80kg" },
        { "role": "assistant", "content": "Got it, logging 4x8 at 80kg. How did it feel?" }
      ]
    }
  ],
  "goals": [...],
  "health_entries": [...]
}
```

### Test helpers (tests/helpers/mod.rs)

```rust
pub struct SeedData {
    pub users: Vec<User>,
    pub exercises: Vec<Exercise>,
    pub sessions: Vec<SeedSession>,
    pub goals: Vec<ExerciseGoal>,
    pub health_entries: Vec<HealthEntry>,
    pub groups: Vec<Group>,
    pub group_members: Vec<GroupMember>,
}

/// Load the fixture file from tests/fixtures/seed_data.json.
pub fn load_seed_data() -> SeedData;

/// Insert all seed data into the database in dependency order:
/// users -> groups -> group_members -> exercises -> sessions + logs -> conversation -> goals -> health
pub fn seed_database(db: &Database, data: &SeedData) -> anyhow::Result<()>;

/// Create an in-memory DB and seed it with the fixture data.
pub fn seeded_db() -> (Database, SeedData);
```

### Integration tests (tests/integration_tests.rs)

```rust
#[test] fn seed_data_loads_completely();            // verify row counts match fixture

#[test] fn exercise_time_series_shows_progression();  // bench press: first < last value
#[test] fn exercise_time_series_time_based();         // plank: has data points
#[test] fn exercise_time_series_distance_based();     // running: has data points
#[test] fn muscle_group_time_series_returns_multiple_exercises();  // chest: >1 exercise
#[test] fn goal_progress_shows_achieved_and_failed(); // at least one of each status
#[test] fn goal_progress_percentages_are_reasonable(); // achieved >= 100%, failed < 100%

#[test] fn injury_gap_visible_in_sessions();          // fewer sessions during injury period
#[test] fn health_entries_have_resolved_dates();       // past injuries have resolved_at

#[test] fn access_control_across_seeded_groups();      // group permissions work end-to-end
#[test] fn conversation_history_proportional();        // messages >= 2 * session count

#[test] fn personal_records();                         // bench PR >= 100kg (achieved goal target)
```

## Tests

### Unit tests per module

**database.rs tests:**
- `open_in_memory_succeeds` -- all tables and indices created
- `foreign_keys_enabled` -- PRAGMA foreign_keys returns 1
- `wal_mode_enabled` -- PRAGMA journal_mode returns "wal"
- `muscle_groups_seeded` -- 16 rows in muscle_groups table
- `measurement_types_seeded` -- 5 rows in measurement_types table

**exercises.rs tests:**
- `insert_and_get_exercise`
- `get_by_name_case_insensitive`
- `list_by_muscle_group`
- `search_exercises_by_name`
- `search_exercises_by_alias`
- `update_exercise`
- `delete_exercise`
- `delete_exercise_with_logs_fails` -- referential integrity: FK prevents deletion when logs exist
- `duplicate_name_fails`
- `seed_exercises_populates_catalogue`
- `seed_exercises_idempotent` -- calling twice yields same count
- `list_muscle_groups_returns_all`
- `list_measurement_types_returns_all`
- `muscle_group_id_lookup`

**users.rs tests:**
- `insert_and_get_user`
- `get_by_telegram_id`
- `duplicate_telegram_id_fails`
- `update_user`
- `delete_user_cascades` -- verify logs, sessions, etc. are deleted

**groups.rs tests:**
- `create_group_and_add_members`
- `remove_member`
- `set_member_level`
- `delete_group_removes_memberships`
- `list_user_groups`

**access.rs tests:**
- `user_can_read_own_data`
- `user_cannot_read_other_data`
- `group_read_access_works`
- `group_write_access_works`
- `group_admin_access_works`
- `write_implies_read`
- `admin_implies_write_and_read`
- `non_member_cannot_read`
- `nonexistent_actor_returns_false`
- `nonexistent_target_returns_false`
- `deleted_group_revokes_access`

**logs.rs tests:**
- `start_and_end_session`
- `get_session_by_id`
- `insert_log_in_session`
- `update_log`
- `get_logs_by_date_range`
- `get_logs_by_exercise`
- `personal_record`
- `session_count_by_week`
- `list_session_summaries`
- `session_delete_cascades_logs` -- ON DELETE CASCADE verified

**goals.rs tests:**
- `insert_and_list_active_goals`
- `list_goals_in_period`
- `mark_goal_achieved`

**schedules.rs tests:**
- `create_schedule_with_exercises`
- `toggle_schedule`
- `delete_schedule_cascades_exercises`

**health.rs tests:**
- `insert_and_list_active`
- `list_by_type`
- `resolve_health_entry`
- `list_history_ordered_by_date`

**conversation.rs tests:**
- `insert_and_get_recent`
- `get_recent_by_platform`
- `prune_old_messages`

**progress.rs tests:**
- `exercise_time_series_returns_daily_points`
- `exercise_time_series_branches_by_measurement_type`
- `muscle_group_time_series_groups_exercises`
- `goal_time_series_includes_goal_exercises`
- `goal_progress_report_computes_percentages`
- `goal_progress_report_derives_status`
- `goal_progress_zero_target_returns_zero_percent`

**concurrent access test (file-backed, using tempfile):**
- `concurrent_read_write_wal` -- open two Database handles to same tempfile, interleave reads and writes

## Verification

```sh
# Run all tests
cargo test -p corre-gym

# Run specific test module
cargo test -p corre-gym -- db::access
cargo test -p corre-gym -- db::logs
cargo test -p corre-gym -- db::progress

# Run integration tests
cargo test -p corre-gym --test integration_tests

# Build the binary (should compile even though main.rs is a placeholder)
cargo build -p corre-gym
```
