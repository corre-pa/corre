use corre_gym::db::*;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct SeedData {
    pub users: Vec<SeedUser>,
    pub exercises: Vec<SeedExercise>,
    pub groups: Vec<SeedGroup>,
    pub group_members: Vec<SeedGroupMember>,
    pub sessions: Vec<SeedSession>,
    pub goals: Vec<SeedGoal>,
    pub health_entries: Vec<SeedHealthEntry>,
}

#[derive(Deserialize)]
pub struct SeedUser {
    pub id: String,
    pub name: String,
    pub telegram_id: Option<String>,
    pub signal_id: Option<String>,
    pub timezone: String,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct SeedExercise {
    pub id: String,
    pub name: String,
    pub aliases: Option<String>,
    pub muscle_group_id: i32,
    pub purpose: String,
    pub measurement_type_id: i32,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct SeedGroup {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct SeedGroupMember {
    pub user_id: String,
    pub group_id: String,
    pub level: String,
}

#[derive(Deserialize)]
pub struct SeedSession {
    pub id: String,
    pub user_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub notes: Option<String>,
    pub logs: Vec<SeedLog>,
    pub conversation: Vec<SeedConversation>,
}

#[derive(Deserialize)]
pub struct SeedLog {
    pub id: String,
    pub exercise_id: String,
    pub sets: Option<i32>,
    pub reps: Option<i32>,
    pub weight_kg: Option<f64>,
    pub duration_secs: Option<i32>,
    pub distance_m: Option<f64>,
    pub difficulty: String,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
pub struct SeedConversation {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize)]
pub struct SeedGoal {
    pub id: String,
    pub user_id: String,
    pub exercise_id: String,
    pub target_value: f64,
    pub start_date: String,
    pub end_date: Option<String>,
    pub achieved: bool,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
pub struct SeedHealthEntry {
    pub id: String,
    pub user_id: String,
    pub entry_type: String,
    pub body_part: Option<String>,
    pub severity: String,
    pub description: String,
    pub started_at: String,
    pub resolved_at: Option<String>,
    pub notes: Option<String>,
}

const MEASUREMENT_TYPE_NAMES: &[&str] = &["", "weight_reps", "time_based", "distance_based", "level_based", "score_based"];

pub fn load_seed_data() -> SeedData {
    let json = include_str!("../fixtures/seed_data.json");
    serde_json::from_str(json).expect("Failed to parse seed_data.json")
}

pub fn seed_database(db: &Database, data: &SeedData) -> anyhow::Result<()> {
    // 1. Users
    for u in &data.users {
        let now = &u.created_at;
        let user = User {
            id: u.id.clone(),
            name: u.name.clone(),
            telegram_id: u.telegram_id.clone(),
            signal_id: u.signal_id.clone(),
            timezone: u.timezone.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        db.insert_user(&user)?;
    }

    // 2. Groups
    for g in &data.groups {
        let group = Group {
            id: g.id.clone(),
            name: g.name.clone(),
            description: g.description.clone(),
            created_at: g.created_at.clone(),
        };
        db.insert_group(&group)?;
    }

    // 3. Group members
    for gm in &data.group_members {
        db.add_member(&gm.user_id, &gm.group_id, AccessLevel::from_str_loose(&gm.level))?;
    }

    // 4. Exercises (insert from fixture, using INSERT OR IGNORE since seed may overlap)
    for ex in &data.exercises {
        let mt_name = MEASUREMENT_TYPE_NAMES.get(ex.measurement_type_id as usize).copied().unwrap_or("weight_reps");
        let exercise = Exercise {
            id: ex.id.clone(),
            name: ex.name.clone(),
            aliases: ex.aliases.clone(),
            muscle_group_id: ex.muscle_group_id,
            purpose: ex.purpose.clone(),
            measurement_type: MeasurementType::from_str_loose(mt_name),
            description: ex.description.clone(),
            created_at: ex.created_at.clone(),
        };
        // Use INSERT OR IGNORE since seed_exercises may have already inserted some
        db.conn().execute(
            "INSERT OR IGNORE INTO exercises (id, name, aliases, muscle_group_id, purpose, measurement_type_id, description, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![exercise.id, exercise.name, exercise.aliases, exercise.muscle_group_id, exercise.purpose, ex.measurement_type_id, exercise.description, exercise.created_at],
        )?;
    }

    // 5. Sessions + logs + conversation
    for sess in &data.sessions {
        let session = Session {
            id: sess.id.clone(),
            user_id: sess.user_id.clone(),
            started_at: sess.started_at.clone(),
            ended_at: sess.ended_at.clone(),
            notes: sess.notes.clone(),
        };
        db.conn().execute(
            "INSERT INTO sessions (id, user_id, started_at, ended_at, notes) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![session.id, session.user_id, session.started_at, session.ended_at, session.notes],
        )?;

        for log in &sess.logs {
            let exercise_log = ExerciseLog {
                id: log.id.clone(),
                user_id: sess.user_id.clone(),
                exercise_id: log.exercise_id.clone(),
                session_id: Some(sess.id.clone()),
                logged_at: sess.started_at.clone(),
                sets: log.sets,
                reps: log.reps,
                weight_kg: log.weight_kg,
                duration_secs: log.duration_secs,
                distance_m: log.distance_m,
                level: None,
                difficulty: Difficulty::from_str_loose(&log.difficulty),
                notes: log.notes.clone(),
            };
            db.insert_log(&exercise_log)?;
        }

        for (i, msg) in sess.conversation.iter().enumerate() {
            let conv = ConversationMessage {
                id: format!("{}-msg-{i}", sess.id),
                user_id: sess.user_id.clone(),
                platform: "telegram".into(),
                role: ConversationRole::from_str_loose(&msg.role),
                content: msg.content.clone(),
                timestamp: sess.started_at.clone(),
            };
            db.insert_message(&conv)?;
        }
    }

    // 6. Goals
    for g in &data.goals {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let goal = ExerciseGoal {
            id: g.id.clone(),
            user_id: g.user_id.clone(),
            exercise_id: g.exercise_id.clone(),
            target_value: g.target_value,
            start_date: g.start_date.clone(),
            end_date: g.end_date.clone(),
            achieved: g.achieved,
            notes: g.notes.clone(),
            created_at: now.clone(),
            updated_at: now,
        };
        db.insert_goal(&goal)?;
    }

    // 7. Health entries
    for h in &data.health_entries {
        let entry = HealthEntry {
            id: h.id.clone(),
            user_id: h.user_id.clone(),
            entry_type: HealthEntryType::from_str_loose(&h.entry_type),
            body_part: h.body_part.clone(),
            severity: h.severity.clone(),
            description: h.description.clone(),
            started_at: h.started_at.clone(),
            resolved_at: h.resolved_at.clone(),
            notes: h.notes.clone(),
            updated_at: h.started_at.clone(),
        };
        db.insert_health_entry(&entry)?;
    }

    Ok(())
}

pub fn seeded_db() -> (Database, SeedData) {
    let db = Database::open_in_memory().unwrap();
    let data = load_seed_data();
    seed_database(&db, &data).unwrap();
    (db, data)
}
