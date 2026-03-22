use std::fmt;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Enums ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementType {
    WeightReps,
    TimeBased,
    DistanceBased,
    LevelBased,
    ScoreBased,
}

impl MeasurementType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WeightReps => "weight_reps",
            Self::TimeBased => "time_based",
            Self::DistanceBased => "distance_based",
            Self::LevelBased => "level_based",
            Self::ScoreBased => "score_based",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().replace('-', "_").as_str() {
            "weight_reps" | "weightreps" => Self::WeightReps,
            "time_based" | "timebased" => Self::TimeBased,
            "distance_based" | "distancebased" => Self::DistanceBased,
            "level_based" | "levelbased" => Self::LevelBased,
            "score_based" | "scorebased" => Self::ScoreBased,
            _ => Self::WeightReps,
        }
    }
}

impl fmt::Display for MeasurementType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
    Failure,
}

impl Difficulty {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Easy => "easy",
            Self::Medium => "medium",
            Self::Hard => "hard",
            Self::Failure => "failure",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "easy" => Self::Easy,
            "hard" => Self::Hard,
            "failure" => Self::Failure,
            _ => Self::Medium,
        }
    }
}

impl fmt::Display for Difficulty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthEntryType {
    Injury,
    Illness,
    Wellbeing,
}

impl HealthEntryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Injury => "injury",
            Self::Illness => "illness",
            Self::Wellbeing => "wellbeing",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "injury" => Self::Injury,
            "illness" => Self::Illness,
            _ => Self::Wellbeing,
        }
    }
}

impl fmt::Display for HealthEntryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    Read,
    Write,
    Admin,
}

impl AccessLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "write" => Self::Write,
            "admin" => Self::Admin,
            _ => Self::Read,
        }
    }
}

impl fmt::Display for AccessLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderType {
    Text,
    Voice,
}

impl ReminderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "voice" => Self::Voice,
            _ => Self::Text,
        }
    }
}

impl fmt::Display for ReminderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationRole {
    User,
    Assistant,
    System,
}

impl ConversationRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "assistant" => Self::Assistant,
            "system" => Self::System,
            _ => Self::User,
        }
    }
}

impl fmt::Display for ConversationRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Achieved,
    Failed,
}

// ── Structs ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exercise {
    pub id: String,
    pub name: String,
    pub aliases: Option<String>,
    pub muscle_group_id: i32,
    pub purpose: String,
    pub measurement_type: MeasurementType,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullExercise {
    pub exercise: Exercise,
    pub muscle_group: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub telegram_id: Option<String>,
    pub signal_id: Option<String>,
    pub timezone: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    pub user_id: String,
    pub group_id: String,
    pub level: AccessLevel,
    pub granted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleExercise {
    pub schedule_id: String,
    pub exercise_id: String,
    pub order_idx: i32,
    pub target_sets: Option<i32>,
    pub target_reps: Option<i32>,
    pub target_weight_kg: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthEntry {
    pub id: String,
    pub user_id: String,
    pub entry_type: HealthEntryType,
    pub body_part: Option<String>,
    pub severity: String,
    pub description: String,
    pub started_at: String,
    pub resolved_at: Option<String>,
    pub notes: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: String,
    pub user_id: String,
    pub platform: String,
    pub role: ConversationRole,
    pub content: String,
    pub timestamp: String,
}

// ── Time-series and goal progress types ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint {
    pub date: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeries {
    pub exercise_id: String,
    pub exercise_name: String,
    pub measurement_type: MeasurementType,
    pub points: Vec<TimeSeriesPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalProgress {
    pub goal: ExerciseGoal,
    pub exercise_name: String,
    pub status: GoalStatus,
    pub current_value: Option<f64>,
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session: Session,
    pub exercise_count: i32,
    pub duration_mins: Option<i32>,
}

// ── Constructors ───────────────────────────────────────────────────────────────

fn now_str() -> String {
    Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn new_user(name: &str, telegram_id: Option<&str>, timezone: &str) -> User {
    let now = now_str();
    User {
        id: Uuid::new_v4().to_string(),
        name: name.to_string(),
        telegram_id: telegram_id.map(String::from),
        signal_id: None,
        timezone: timezone.to_string(),
        created_at: now.clone(),
        updated_at: now,
    }
}

pub fn new_session(user_id: &str, notes: Option<&str>) -> Session {
    Session {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        started_at: now_str(),
        ended_at: None,
        notes: notes.map(String::from),
    }
}

pub fn new_exercise_log(user_id: &str, exercise_id: &str, session_id: Option<&str>) -> ExerciseLog {
    ExerciseLog {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        exercise_id: exercise_id.to_string(),
        session_id: session_id.map(String::from),
        logged_at: now_str(),
        sets: None,
        reps: None,
        weight_kg: None,
        duration_secs: None,
        distance_m: None,
        level: None,
        difficulty: Difficulty::Medium,
        notes: None,
    }
}

pub fn new_exercise_goal(user_id: &str, exercise_id: &str, target_value: f64) -> ExerciseGoal {
    let now = now_str();
    ExerciseGoal {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        exercise_id: exercise_id.to_string(),
        target_value,
        start_date: now.clone(),
        end_date: None,
        achieved: false,
        notes: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

pub fn new_health_entry(user_id: &str, entry_type: HealthEntryType, description: &str) -> HealthEntry {
    let now = now_str();
    HealthEntry {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        entry_type,
        body_part: None,
        severity: "mild".to_string(),
        description: description.to_string(),
        started_at: now.clone(),
        resolved_at: None,
        notes: None,
        updated_at: now,
    }
}

pub fn new_conversation_message(user_id: &str, platform: &str, role: ConversationRole, content: &str) -> ConversationMessage {
    ConversationMessage {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        platform: platform.to_string(),
        role,
        content: content.to_string(),
        timestamp: now_str(),
    }
}
