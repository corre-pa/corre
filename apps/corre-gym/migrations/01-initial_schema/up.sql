-- =============================================================================
-- corre-gym — initial schema (v1)
--
-- All primary keys are auto-incrementing INTEGERs (rowid alias). Foreign keys
-- are INTEGERs throughout. The exercise taxonomy is seeded inline at the
-- bottom of this file using stable ID ranges:
--
--     1 .. 99       muscle_group
--     100 .. 999    specific_muscle
--     1000 .. 9999  exercise
--     10000 ..      variation
-- =============================================================================

-- -----------------------------------------------------------------------------
-- Reference: measurement types (how a set is recorded)
-- -----------------------------------------------------------------------------
CREATE TABLE measurement_types (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

INSERT INTO measurement_types (id, name) VALUES
    (1, 'weight_reps'),
    (2, 'time_based'),
    (3, 'distance_based'),
    (4, 'level_based'),
    (5, 'score_based');

-- -----------------------------------------------------------------------------
-- Hierarchical exercise taxonomy
-- -----------------------------------------------------------------------------
CREATE TABLE exercise_types (
    id                  INTEGER PRIMARY KEY,
    name                TEXT NOT NULL COLLATE NOCASE,
    parent_id           INTEGER REFERENCES exercise_types(id) ON DELETE RESTRICT,
    level               TEXT NOT NULL CHECK (level IN
                            ('muscle_group','specific_muscle','exercise','variation')),
    aliases             TEXT,
    purpose             TEXT,
    measurement_type_id INTEGER REFERENCES measurement_types(id),
    description         TEXT,
    url                 TEXT,  -- relative path to an illustrative image/video; populated for muscle_group / specific_muscle, NULL for exercises and variations until per-exercise media is sourced
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (parent_id, name),
    CHECK ((level = 'muscle_group' AND parent_id IS NULL)
        OR (level <> 'muscle_group' AND parent_id IS NOT NULL))
);

CREATE INDEX idx_exercise_types_parent ON exercise_types(parent_id);
CREATE INDEX idx_exercise_types_level  ON exercise_types(level);

-- -----------------------------------------------------------------------------
-- Users (telegram_id / signal_id are external identifiers; users.id is internal)
-- -----------------------------------------------------------------------------
CREATE TABLE users (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    telegram_id TEXT UNIQUE,
    signal_id   TEXT UNIQUE,
    timezone    TEXT NOT NULL DEFAULT 'UTC',
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- -----------------------------------------------------------------------------
-- Access groups
-- -----------------------------------------------------------------------------
CREATE TABLE groups (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE group_members (
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    group_id   INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    level      TEXT NOT NULL DEFAULT 'read'
                   CHECK (level IN ('read','write','admin')),
    granted_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, group_id)
);

-- -----------------------------------------------------------------------------
-- Per-user goals (target a specific exercise_type at any level)
-- -----------------------------------------------------------------------------
CREATE TABLE exercise_goals (
    id               INTEGER PRIMARY KEY,
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    exercise_type_id INTEGER NOT NULL REFERENCES exercise_types(id),
    target_value     REAL NOT NULL,
    start_date       TEXT NOT NULL,
    end_date         TEXT,
    achieved         INTEGER NOT NULL DEFAULT 0,
    notes            TEXT,
    created_at       TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_goals_user ON exercise_goals(user_id, achieved);

-- -----------------------------------------------------------------------------
-- Workout sessions (a whole training session)
-- -----------------------------------------------------------------------------
CREATE TABLE sessions (
    id         INTEGER PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    ended_at   TEXT,
    notes      TEXT
);

CREATE INDEX idx_sessions_user ON sessions(user_id, started_at);

-- -----------------------------------------------------------------------------
-- Exercise entries (a block of related sets within a session)
-- -----------------------------------------------------------------------------
CREATE TABLE exercise_entry (
    id              INTEGER PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id      INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
    start_timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    end_timestamp   TEXT,
    comment         TEXT
);

CREATE INDEX idx_exercise_entry_user    ON exercise_entry(user_id, start_timestamp);
CREATE INDEX idx_exercise_entry_session ON exercise_entry(session_id);

-- -----------------------------------------------------------------------------
-- Sets (individual recorded efforts)
--
-- The (count, value) pair is interpreted via measurement_type_id:
--     weight_reps    -> count = reps, value = weight_kg
--     time_based     -> count = NULL,  value = duration_secs
--     distance_based -> count = NULL,  value = distance_m
--     level_based    -> count = NULL,  value = level
--     score_based    -> count = NULL,  value = score
-- -----------------------------------------------------------------------------
CREATE TABLE sets (
    id                   INTEGER PRIMARY KEY,
    exercise_entry_id    INTEGER NOT NULL REFERENCES exercise_entry(id) ON DELETE CASCADE,
    exercise_type_id     INTEGER NOT NULL REFERENCES exercise_types(id),
    order_idx            INTEGER NOT NULL DEFAULT 0,
    measurement_type_id  INTEGER NOT NULL REFERENCES measurement_types(id),
    count                INTEGER,
    value                REAL NOT NULL,
    perceived_difficulty TEXT NOT NULL DEFAULT 'medium'
                             CHECK (perceived_difficulty IN ('easy','medium','hard','failure')),
    comment              TEXT,
    logged_at            TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_sets_entry        ON sets(exercise_entry_id);
CREATE INDEX idx_sets_type_logged  ON sets(exercise_type_id, logged_at);

-- -----------------------------------------------------------------------------
-- Schedules (cron-driven workout reminders)
-- -----------------------------------------------------------------------------
CREATE TABLE schedules (
    id                   INTEGER PRIMARY KEY,
    user_id              INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                 TEXT NOT NULL,
    cron_expr            TEXT NOT NULL,
    reminder_type        TEXT NOT NULL DEFAULT 'text'
                             CHECK (reminder_type IN ('text','voice')),
    reminder_notice_mins INTEGER NOT NULL DEFAULT 30,
    enabled              INTEGER NOT NULL DEFAULT 1,
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at           TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_schedules_user ON schedules(user_id);

CREATE TABLE schedule_exercises (
    schedule_id      INTEGER NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    exercise_type_id INTEGER NOT NULL REFERENCES exercise_types(id),
    order_idx        INTEGER NOT NULL DEFAULT 0,
    target_sets      INTEGER,
    target_reps      INTEGER,
    target_weight_kg REAL,
    PRIMARY KEY (schedule_id, exercise_type_id)
);

-- -----------------------------------------------------------------------------
-- Health tracking (injuries, illnesses, wellbeing notes)
-- -----------------------------------------------------------------------------
CREATE TABLE health_entries (
    id          INTEGER PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    entry_type  TEXT NOT NULL CHECK (entry_type IN ('injury','illness','wellbeing')),
    body_part   TEXT,
    severity    TEXT NOT NULL DEFAULT 'mild'
                    CHECK (severity IN ('mild','moderate','severe')),
    description TEXT NOT NULL,
    started_at  TEXT NOT NULL DEFAULT (datetime('now')),
    resolved_at TEXT,
    notes       TEXT,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_health_user_date ON health_entries(user_id, started_at);

-- -----------------------------------------------------------------------------
-- Conversation history (LLM context per platform)
-- -----------------------------------------------------------------------------
CREATE TABLE conversation_history (
    id                   INTEGER PRIMARY KEY,
    user_id              INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    platform             TEXT NOT NULL DEFAULT 'telegram'
                             CHECK (platform IN ('telegram','signal','web')),
    role                 TEXT NOT NULL CHECK (role IN ('user','assistant','system')),
    content              TEXT NOT NULL,
    timestamp            TEXT NOT NULL DEFAULT (datetime('now')),
    exclude_from_context INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_conversation_user_time ON conversation_history(user_id, timestamp);


-- =============================================================================
-- Initial taxonomy seed
-- =============================================================================

-- ============================================
-- Level 1: muscle groups
-- ============================================
INSERT INTO exercise_types (id, name, parent_id, level, url) VALUES
    (1, 'Chest',     NULL, 'muscle_group', '/static/muscles/chest.png'),
    (2, 'Back',      NULL, 'muscle_group', '/static/muscles/back.png'),
    (3, 'Shoulders', NULL, 'muscle_group', '/static/muscles/shoulders.png'),
    (4, 'Arms',      NULL, 'muscle_group', '/static/muscles/arms.png'),
    (5, 'Legs',      NULL, 'muscle_group', NULL),
    (6, 'Core',      NULL, 'muscle_group', '/static/muscles/core.png'),
    (7, 'Cardio',    NULL, 'muscle_group', NULL);


-- ============================================
-- CHEST  (id 1)
-- ============================================
INSERT INTO exercise_types (id, name, parent_id, level, url) VALUES
    (100, 'Pectoral',          1, 'specific_muscle', '/static/muscles/pectoral.png'),
    (101, 'Serratus Anterior', 1, 'specific_muscle', '/static/muscles/serratus-anterior.png');

-- Pectoral exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (1000, 'Bench Press', 100, 'exercise', 1, 'strength',    'bench,bench press'),
    (1001, 'Chest Fly',   100, 'exercise', 1, 'hypertrophy', 'fly,flyes'),
    (1002, 'Push-Up',     100, 'exercise', 1, 'endurance',   'pushup,press-up'),
    (1003, 'Chest Dip',   100, 'exercise', 1, 'strength',    NULL);

-- Bench Press variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (10000, 'Flat Barbell Bench Press',     1000, 'variation', 1, 'flat bench,barbell bench'),
    (10001, 'Flat Dumbbell Bench Press',    1000, 'variation', 1, 'db bench,flat db press'),
    (10002, 'Incline Barbell Bench Press',  1000, 'variation', 1, 'incline bench'),
    (10003, 'Incline Dumbbell Bench Press', 1000, 'variation', 1, 'incline press,incline db press'),
    (10004, 'Decline Barbell Bench Press',  1000, 'variation', 1, NULL),
    (10005, 'Decline Dumbbell Bench Press', 1000, 'variation', 1, NULL);

-- Chest Fly variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (10010, 'Flat Dumbbell Fly',    1001, 'variation', 1, NULL),
    (10011, 'Incline Dumbbell Fly', 1001, 'variation', 1, NULL),
    (10012, 'Cable Crossover',      1001, 'variation', 1, 'cable flyes,cable crossover'),
    (10013, 'Pec Deck Machine',     1001, 'variation', 1, 'pec deck');

-- Push-Up variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (10020, 'Standard Push-Up', 1002, 'variation', 1, NULL),
    (10021, 'Diamond Push-Up',  1002, 'variation', 1, NULL),
    (10022, 'Decline Push-Up',  1002, 'variation', 1, NULL),
    (10023, 'Wide Push-Up',     1002, 'variation', 1, NULL);

-- Serratus Anterior exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (1010, 'Scapular Push-Up', 101, 'exercise', 1, 'endurance', NULL);


-- ============================================
-- BACK  (id 2)
-- ============================================
INSERT INTO exercise_types (id, name, parent_id, level, url) VALUES
    (200, 'Latissimus Dorsi', 2, 'specific_muscle', '/static/muscles/latissimus-dorsi.png'),
    (201, 'Rhomboid',         2, 'specific_muscle', '/static/muscles/rhomboid.png'),
    (202, 'Trapezius',        2, 'specific_muscle', '/static/muscles/trapezius.png');

-- Latissimus Dorsi exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (2000, 'Pull-Up',      200, 'exercise', 1, 'strength',    'pull up,pullup'),
    (2001, 'Lat Pulldown', 200, 'exercise', 1, 'hypertrophy', 'pulldown,lat pull'),
    (2002, 'Seated Row',   200, 'exercise', 1, 'hypertrophy', 'row');

-- Pull-Up variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (20000, 'Overhand Pull-Up',  2000, 'variation', 1, NULL),
    (20001, 'Underhand Pull-Up', 2000, 'variation', 1, NULL),
    (20002, 'Wide Grip Pull-Up', 2000, 'variation', 1, NULL),
    (20003, 'Assisted Pull-Up',  2000, 'variation', 1, NULL);

-- Lat Pulldown variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (20010, 'Wide Grip Lat Pulldown',  2001, 'variation', 1, NULL),
    (20011, 'Close Grip Lat Pulldown', 2001, 'variation', 1, NULL),
    (20012, 'Underhand Lat Pulldown',  2001, 'variation', 1, NULL);

-- Seated Row variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (20020, 'Cable Seated Row',   2002, 'variation', 1, NULL),
    (20021, 'Machine Seated Row', 2002, 'variation', 1, NULL);

-- Rhomboid exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (2010, 'Reverse Fly', 201, 'exercise', 1, 'hypertrophy', NULL);

-- Reverse Fly variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (20030, 'Dumbbell Reverse Fly', 2010, 'variation', 1, NULL),
    (20031, 'Cable Reverse Fly',    2010, 'variation', 1, NULL);

-- Trapezius exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (2020, 'Shrug',     202, 'exercise', 1, 'hypertrophy', NULL),
    (2021, 'Face Pull', 202, 'exercise', 1, 'hypertrophy', NULL);

-- Shrug variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (20040, 'Barbell Shrug',  2020, 'variation', 1, NULL),
    (20041, 'Dumbbell Shrug', 2020, 'variation', 1, NULL),
    (20042, 'Trap Bar Shrug', 2020, 'variation', 1, NULL);


-- ============================================
-- SHOULDERS  (id 3)
-- ============================================
INSERT INTO exercise_types (id, name, parent_id, level, url) VALUES
    (300, 'Anterior Deltoid',  3, 'specific_muscle', '/static/muscles/anterior-deltoid.png'),
    (301, 'Lateral Deltoid',   3, 'specific_muscle', '/static/muscles/lateral-deltoid.png'),
    (302, 'Posterior Deltoid', 3, 'specific_muscle', '/static/muscles/posterior-deltoid.png');

-- Anterior Deltoid exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (3000, 'Overhead Press', 300, 'exercise', 1, 'strength',    'ohp,shoulder press'),
    (3001, 'Front Raise',    300, 'exercise', 1, 'hypertrophy', NULL);

-- Overhead Press variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (30000, 'Barbell Overhead Press',  3000, 'variation', 1, NULL),
    (30001, 'Dumbbell Overhead Press', 3000, 'variation', 1, NULL),
    (30002, 'Military Press',          3000, 'variation', 1, NULL),
    (30003, 'Push Press',              3000, 'variation', 1, NULL);

-- Front Raise variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (30010, 'Dumbbell Front Raise', 3001, 'variation', 1, NULL),
    (30011, 'Cable Front Raise',    3001, 'variation', 1, NULL);

-- Lateral Deltoid exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (3010, 'Lateral Raise', 301, 'exercise', 1, 'hypertrophy', 'side raise');

-- Lateral Raise variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (30020, 'Dumbbell Lateral Raise', 3010, 'variation', 1, NULL),
    (30021, 'Cable Lateral Raise',    3010, 'variation', 1, NULL),
    (30022, 'Machine Lateral Raise',  3010, 'variation', 1, NULL);

-- Posterior Deltoid exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (3020, 'Rear Delt Fly', 302, 'exercise', 1, 'hypertrophy', 'rear delt');

-- Rear Delt Fly variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (30030, 'Dumbbell Rear Delt Fly', 3020, 'variation', 1, NULL),
    (30031, 'Cable Rear Delt Fly',    3020, 'variation', 1, NULL),
    (30032, 'Machine Rear Delt Fly',  3020, 'variation', 1, NULL);


-- ============================================
-- ARMS  (id 4)
-- ============================================
INSERT INTO exercise_types (id, name, parent_id, level, url) VALUES
    (400, 'Biceps',  4, 'specific_muscle', '/static/muscles/biceps.png'),
    (401, 'Triceps', 4, 'specific_muscle', '/static/muscles/triceps.png'),
    (402, 'Forearm', 4, 'specific_muscle', NULL);

-- Biceps exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (4000, 'Bicep Curl', 400, 'exercise', 1, 'hypertrophy', 'curl'),
    (4001, 'Chin-Up',    400, 'exercise', 1, 'strength',    'chinup');

-- Bicep Curl variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (40000, 'Barbell Bicep Curl',  4000, 'variation', 1, NULL),
    (40001, 'Dumbbell Bicep Curl', 4000, 'variation', 1, NULL),
    (40002, 'Hammer Curl',         4000, 'variation', 1, 'hammer'),
    (40003, 'Preacher Curl',       4000, 'variation', 1, NULL),
    (40004, 'Cable Curl',          4000, 'variation', 1, NULL);

-- Triceps exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (4010, 'Tricep Extension',       401, 'exercise', 1, 'hypertrophy', NULL),
    (4011, 'Close-Grip Bench Press', 401, 'exercise', 1, 'strength',    'cgbp'),
    (4012, 'Tricep Dip',             401, 'exercise', 1, 'strength',    NULL);

-- Tricep Extension variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (40010, 'Skull Crusher',             4010, 'variation', 1, 'skullcrusher,lying tricep ext'),
    (40011, 'Cable Tricep Pushdown',     4010, 'variation', 1, 'pushdown,tricep pressdown'),
    (40012, 'Overhead Tricep Extension', 4010, 'variation', 1, NULL),
    (40013, 'Dumbbell Tricep Kickback',  4010, 'variation', 1, 'kickback');

-- Forearm exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (4020, 'Wrist Curl',         402, 'exercise', 1, 'hypertrophy', NULL),
    (4021, 'Reverse Wrist Curl', 402, 'exercise', 1, 'hypertrophy', NULL);


-- ============================================
-- LEGS  (id 5)
-- ============================================
INSERT INTO exercise_types (id, name, parent_id, level, url) VALUES
    (500, 'Quadriceps', 5, 'specific_muscle', NULL),
    (501, 'Hamstrings', 5, 'specific_muscle', NULL),
    (502, 'Glutes',     5, 'specific_muscle', '/static/muscles/glutes.png'),
    (503, 'Calves',     5, 'specific_muscle', '/static/muscles/calves.png');

-- Quadriceps exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (5000, 'Squat',          500, 'exercise', 1, 'strength',    'sq'),
    (5001, 'Leg Press',      500, 'exercise', 1, 'hypertrophy', NULL),
    (5002, 'Leg Extension',  500, 'exercise', 1, 'hypertrophy', 'quad extension'),
    (5003, 'Lunge',          500, 'exercise', 1, 'strength',    NULL);

-- Squat variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (50000, 'Back Squat',    5000, 'variation', 1, 'back sq'),
    (50001, 'Front Squat',   5000, 'variation', 1, 'front sq'),
    (50002, 'Goblet Squat',  5000, 'variation', 1, 'goblet,kb squat'),
    (50003, 'Hack Squat',    5000, 'variation', 1, NULL),
    (50004, 'Split Squat',   5000, 'variation', 1, NULL);

-- Lunge variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (50010, 'Walking Lunge',          5003, 'variation', 1, NULL),
    (50011, 'Reverse Lunge',          5003, 'variation', 1, NULL),
    (50012, 'Bulgarian Split Squat',  5003, 'variation', 1, 'bss,split squat');

-- Hamstrings exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (5010, 'Deadlift',     501, 'exercise', 1, 'strength',    'dl'),
    (5011, 'Leg Curl',     501, 'exercise', 1, 'hypertrophy', 'hamstring curl'),
    (5012, 'Good Morning', 501, 'exercise', 1, 'strength',    NULL);

-- Deadlift variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (50020, 'Conventional Deadlift', 5010, 'variation', 1, NULL),
    (50021, 'Sumo Deadlift',         5010, 'variation', 1, NULL),
    (50022, 'Romanian Deadlift',     5010, 'variation', 1, 'rdl,stiff-leg deadlift');

-- Leg Curl variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (50030, 'Lying Leg Curl',    5011, 'variation', 1, 'lying leg curl'),
    (50031, 'Seated Leg Curl',   5011, 'variation', 1, NULL),
    (50032, 'Standing Leg Curl', 5011, 'variation', 1, NULL);

-- Glutes exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (5020, 'Hip Thrust',     502, 'exercise', 1, 'strength',    NULL),
    (5021, 'Glute Bridge',   502, 'exercise', 1, 'hypertrophy', 'bridge'),
    (5022, 'Cable Kickback', 502, 'exercise', 1, 'hypertrophy', 'glute kickback');

-- Hip Thrust variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (50040, 'Barbell Hip Thrust',     5020, 'variation', 1, 'barbell hip thrust'),
    (50041, 'Single-Leg Hip Thrust',  5020, 'variation', 1, NULL);

-- Calves exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (5030, 'Calf Raise',      503, 'exercise', 1, 'hypertrophy', 'calf press'),
    (5031, 'Tibialis Raise',  503, 'exercise', 1, 'hypertrophy', NULL);

-- Calf Raise variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (50050, 'Standing Calf Raise', 5030, 'variation', 1, 'standing calf'),
    (50051, 'Seated Calf Raise',   5030, 'variation', 1, 'seated calf'),
    (50052, 'Donkey Calf Raise',   5030, 'variation', 1, NULL);


-- ============================================
-- CORE  (id 6)
-- ============================================
INSERT INTO exercise_types (id, name, parent_id, level, url) VALUES
    (600, 'Rectus Abdominis',     6, 'specific_muscle', '/static/muscles/rectus-abdominis.png'),
    (601, 'Oblique',              6, 'specific_muscle', NULL),
    (602, 'Transverse Abdominis', 6, 'specific_muscle', NULL);

-- Rectus Abdominis exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (6000, 'Crunch',           600, 'exercise', 1, 'hypertrophy', NULL),
    (6001, 'Sit-Up',           600, 'exercise', 1, 'endurance',   'situp'),
    (6002, 'Leg Raise',        600, 'exercise', 1, 'strength',    'leg raise'),
    (6003, 'Ab Wheel Rollout', 600, 'exercise', 1, 'strength',    'ab wheel,rollout');

-- Crunch variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (60000, 'Bodyweight Crunch', 6000, 'variation', 1, NULL),
    (60001, 'Cable Crunch',      6000, 'variation', 1, 'cable ab crunch'),
    (60002, 'Machine Crunch',    6000, 'variation', 1, NULL);

-- Leg Raise variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (60010, 'Hanging Leg Raise',         6002, 'variation', 1, 'hanging raise,knee raise'),
    (60011, 'Lying Leg Raise',           6002, 'variation', 1, NULL),
    (60012, 'Captain''s Chair Leg Raise', 6002, 'variation', 1, 'captain chair');

-- Oblique exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (6010, 'Russian Twist', 601, 'exercise', 1, 'endurance',   NULL),
    (6011, 'Side Bend',     601, 'exercise', 1, 'hypertrophy', NULL),
    (6012, 'Woodchopper',   601, 'exercise', 1, 'strength',    NULL);

-- Side Bend variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (60020, 'Dumbbell Side Bend', 6011, 'variation', 1, NULL),
    (60021, 'Cable Side Bend',    6011, 'variation', 1, NULL);

-- Transverse Abdominis exercises
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (6020, 'Plank',    602, 'exercise', 2, 'endurance', 'forearm plank'),
    (6021, 'Dead Bug', 602, 'exercise', 1, 'endurance', NULL);

-- Plank variations
INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, aliases) VALUES
    (60030, 'Front Plank', 6020, 'variation', 2, NULL),
    (60031, 'Side Plank',  6020, 'variation', 2, 'lateral plank');


-- ============================================
-- CARDIO  (id 7)
-- ============================================
INSERT INTO exercise_types (id, name, parent_id, level, url) VALUES
    (700, 'Cardiovascular', 7, 'specific_muscle', NULL);

INSERT INTO exercise_types (id, name, parent_id, level, measurement_type_id, purpose, aliases) VALUES
    (7000, 'Running',   700, 'exercise', 3, 'cardio', 'run,jog,jogging'),
    (7001, 'Cycling',   700, 'exercise', 3, 'cardio', 'bike,biking,cycle'),
    (7002, 'Rowing',    700, 'exercise', 3, 'cardio', 'erg,rower,rowing'),
    (7003, 'Swimming',  700, 'exercise', 3, 'cardio', 'swim,laps'),
    (7004, 'Jump Rope', 700, 'exercise', 2, 'cardio', 'skipping,skip rope'),
    (7005, 'Padel',     700, 'exercise', 3, 'cardio', 'padel');
