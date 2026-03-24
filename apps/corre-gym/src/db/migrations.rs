pub const MIGRATIONS: &str = r#"
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
    aliases             TEXT,
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
    target_value REAL NOT NULL,
    start_date   TEXT NOT NULL,
    end_date     TEXT,
    achieved     INTEGER NOT NULL DEFAULT 0,
    notes        TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_goals_user ON exercise_goals(user_id, achieved);

-- Workout sessions
CREATE TABLE IF NOT EXISTS sessions (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    ended_at   TEXT,
    notes      TEXT
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id, started_at);

-- Exercise logs
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
    level         INTEGER,
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
    schedule_id      TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    exercise_id      TEXT NOT NULL REFERENCES exercises(id),
    order_idx        INTEGER NOT NULL DEFAULT 0,
    target_sets      INTEGER,
    target_reps      INTEGER,
    target_weight_kg REAL,
    PRIMARY KEY (schedule_id, exercise_id)
) WITHOUT ROWID;

-- Health tracking
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
"#;

/// Incremental migration: add exclude_from_context flag to conversation_history.
/// Runs idempotently — the "duplicate column name" error is suppressed on re-runs.
pub const MIGRATIONS_V2: &str = "ALTER TABLE conversation_history ADD COLUMN exclude_from_context INTEGER NOT NULL DEFAULT 0;";
