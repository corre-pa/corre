use serde::Deserialize;

use crate::db::{Difficulty, HealthEntryType};

#[derive(Debug, Deserialize)]
pub struct AssistantResponse {
    pub message: String,
    #[serde(default)]
    pub actions: Vec<AssistantAction>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantAction {
    LogExercise {
        exercise: String,
        sets: Option<i32>,
        reps: Option<i32>,
        weight_kg: Option<f64>,
        #[serde(default)]
        difficulty: Option<Difficulty>,
    },
    LogExerciseTimed {
        exercise: String,
        duration_secs: i32,
        #[serde(default)]
        difficulty: Option<Difficulty>,
    },
    LogExerciseDistance {
        exercise: String,
        distance_m: Option<f64>,
        duration_secs: Option<i32>,
        #[serde(default)]
        difficulty: Option<Difficulty>,
    },
    StartSession {
        notes: Option<String>,
    },
    EndSession,
    LogHealth {
        entry_type: HealthEntryType,
        body_part: Option<String>,
        severity: Option<String>,
        description: String,
    },
    ResolveHealth {
        description: String,
    },
    SetGoal {
        exercise: String,
        target_value: f64,
        end_date: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_log_exercise_action() {
        let json =
            r#"{"type": "log_exercise", "exercise": "Barbell Bench Press", "sets": 3, "reps": 8, "weight_kg": 80.0, "difficulty": "hard"}"#;
        let action: AssistantAction = serde_json::from_str(json).unwrap();
        match action {
            AssistantAction::LogExercise { exercise, sets, reps, weight_kg, difficulty } => {
                assert_eq!(exercise, "Barbell Bench Press");
                assert_eq!(sets, Some(3));
                assert_eq!(reps, Some(8));
                assert_eq!(weight_kg, Some(80.0));
                assert_eq!(difficulty, Some(Difficulty::Hard));
            }
            _ => panic!("expected LogExercise"),
        }
    }

    #[test]
    fn parse_log_exercise_timed() {
        let json = r#"{"type": "log_exercise_timed", "exercise": "Plank", "duration_secs": 60}"#;
        let action: AssistantAction = serde_json::from_str(json).unwrap();
        assert!(matches!(action, AssistantAction::LogExerciseTimed { duration_secs: 60, .. }));
    }

    #[test]
    fn parse_log_exercise_distance() {
        let json = r#"{"type": "log_exercise_distance", "exercise": "Running", "distance_m": 5000.0, "duration_secs": 1800}"#;
        let action: AssistantAction = serde_json::from_str(json).unwrap();
        assert!(matches!(action, AssistantAction::LogExerciseDistance { .. }));
    }

    #[test]
    fn parse_start_session() {
        let json = r#"{"type": "start_session", "notes": "Leg day"}"#;
        let action: AssistantAction = serde_json::from_str(json).unwrap();
        match action {
            AssistantAction::StartSession { notes } => assert_eq!(notes.as_deref(), Some("Leg day")),
            _ => panic!("expected StartSession"),
        }
    }

    #[test]
    fn parse_end_session() {
        let json = r#"{"type": "end_session"}"#;
        let action: AssistantAction = serde_json::from_str(json).unwrap();
        assert!(matches!(action, AssistantAction::EndSession));
    }

    #[test]
    fn parse_log_health() {
        let json = r#"{"type": "log_health", "entry_type": "injury", "body_part": "shoulder", "severity": "moderate", "description": "Shoulder pain"}"#;
        let action: AssistantAction = serde_json::from_str(json).unwrap();
        assert!(matches!(action, AssistantAction::LogHealth { .. }));
    }

    #[test]
    fn parse_resolve_health() {
        let json = r#"{"type": "resolve_health", "description": "shoulder"}"#;
        let action: AssistantAction = serde_json::from_str(json).unwrap();
        assert!(matches!(action, AssistantAction::ResolveHealth { .. }));
    }

    #[test]
    fn parse_set_goal() {
        let json = r#"{"type": "set_goal", "exercise": "Barbell Bench Press", "target_value": 100.0, "end_date": "2026-06-01"}"#;
        let action: AssistantAction = serde_json::from_str(json).unwrap();
        assert!(matches!(action, AssistantAction::SetGoal { target_value, .. } if target_value == 100.0));
    }

    #[test]
    fn unknown_action_type() {
        let json = r#"{"type": "dance_party"}"#;
        let action: AssistantAction = serde_json::from_str(json).unwrap();
        assert!(matches!(action, AssistantAction::Unknown));
    }

    #[test]
    fn missing_optional_fields() {
        let json = r#"{"type": "log_exercise", "exercise": "Barbell Bench Press"}"#;
        let action: AssistantAction = serde_json::from_str(json).unwrap();
        match action {
            AssistantAction::LogExercise { sets, reps, weight_kg, difficulty, .. } => {
                assert_eq!(sets, None);
                assert_eq!(reps, None);
                assert_eq!(weight_kg, None);
                assert_eq!(difficulty, None);
            }
            _ => panic!("expected LogExercise"),
        }
    }

    #[test]
    fn parse_response_with_actions() {
        let json = r#"{
            "message": "Logged it!",
            "actions": [
                {"type": "start_session"},
                {"type": "log_exercise", "exercise": "Bench", "sets": 3, "reps": 8, "weight_kg": 80.0}
            ]
        }"#;
        let resp: AssistantResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.message, "Logged it!");
        assert_eq!(resp.actions.len(), 2);
    }

    #[test]
    fn parse_response_absent_actions() {
        let json = r#"{"message": "Hello!"}"#;
        let resp: AssistantResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.message, "Hello!");
        assert!(resp.actions.is_empty());
    }

    #[test]
    fn parse_response_null_actions() {
        let json = r#"{"message": "Hello!", "actions": null}"#;
        // serde(default) on Vec means missing field -> empty vec, but null needs special handling
        // With #[serde(default)] alone, null will error. Let's test the parser handles it via fallback.
        let result = serde_json::from_str::<AssistantResponse>(json);
        // null for a Vec<T> with #[serde(default)] will fail serde; our parser.rs handles this gracefully
        assert!(result.is_err());
    }
}
