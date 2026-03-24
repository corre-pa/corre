use crate::db::{ExerciseLog, FullExercise, GoalProgress, HealthEntry, MeasurementType, Schedule, Session, SessionSummary};

pub struct PromptContext {
    pub user_name: String,
    pub timezone: String,
    pub current_time: String,
    pub active_session: Option<Session>,
    pub session_logs: Vec<(ExerciseLog, String)>, // (log, exercise_name)
    pub health_entries: Vec<HealthEntry>,
    pub recent_summaries: Vec<SessionSummary>,
    pub recent_logs: Vec<ExerciseLog>,
    pub exercises: Vec<FullExercise>,
    pub active_goals: Vec<GoalProgress>,
    pub schedules: Vec<Schedule>,
}

pub fn build_system_prompt(ctx: &PromptContext) -> String {
    let session_status = match &ctx.active_session {
        Some(s) => {
            let log_summary = if ctx.session_logs.is_empty() {
                "no exercises logged yet".to_string()
            } else {
                let entries: Vec<String> = ctx.session_logs.iter().map(|(log, name)| format_log_entry(log, name)).collect();
                entries.join("; ")
            };
            format!("Active (started {}). Logged: {log_summary}", s.started_at)
        }
        None => "No active session".to_string(),
    };

    let health_section = format_health_entries(&ctx.health_entries);
    let history_section = format_recent_history(&ctx.recent_summaries, &ctx.recent_logs, &ctx.exercises);
    let goals_section = format_active_goals(&ctx.active_goals);
    let exercise_list = format_exercise_list(&ctx.exercises);

    format!(
        "You are a personal gym trainer assistant. You help users track workouts, log exercises, \
manage health issues, and provide coaching.\n\
\n\
SCOPE: You ONLY discuss topics related to exercise, workouts, gym training, fitness goals, \
nutrition as it relates to training, and health issues that affect exercise. If the user \
asks about anything unrelated, politely decline and remind them that you are a gym trainer \
assistant. Do not answer general knowledge questions, write code, tell stories, or engage \
with off-topic requests, even if the user insists.\n\
\n\
RESPONSE FORMAT: You MUST respond with ONLY a JSON object. No text before or after.\n\
{{\n\
  \"message\": \"Your conversational response to the user\",\n\
  \"actions\": []\n\
}}\n\
\n\
ACTION TYPES:\n\
- {{\"type\": \"log_exercise\", \"exercise\": \"<EXACT NAME>\", \"sets\": N, \"reps\": N, \
\"weight_kg\": N.N, \"difficulty\": \"easy|medium|hard|failure\"}}\n\
- {{\"type\": \"log_exercise_timed\", \"exercise\": \"<EXACT NAME>\", \"duration_secs\": N, \
\"difficulty\": \"easy|medium|hard|failure\"}}\n\
- {{\"type\": \"log_exercise_distance\", \"exercise\": \"<EXACT NAME>\", \"distance_m\": N.N, \
\"duration_secs\": N, \"difficulty\": \"easy|medium|hard|failure\"}}\n\
- {{\"type\": \"start_session\", \"notes\": \"<optional>\"}}\n\
- {{\"type\": \"end_session\"}}\n\
- {{\"type\": \"log_health\", \"entry_type\": \"injury|illness|wellbeing\", \
\"body_part\": \"<optional>\", \"severity\": \"mild|moderate|severe\", \"description\": \"...\"}}\n\
- {{\"type\": \"resolve_health\", \"description\": \"match by description substring\"}}\n\
- {{\"type\": \"set_goal\", \"exercise\": \"<EXACT NAME>\", \"target_value\": N.N, \
\"end_date\": \"<optional YYYY-MM-DD>\"}}\n\
\n\
EXERCISE NAME RULE: You MUST use exercise names EXACTLY as they appear in double quotes \
in the Available Exercises list below. Do not abbreviate, paraphrase, or invent names. \
If the user mentions an exercise not in the list, use the closest match and note the \
substitution in your message.\n\
\n\
GUIDELINES:\n\
- When the user reports an exercise, include a log_exercise action\n\
- Auto-start a session (start_session action) before logging if no session is active\n\
- If the user mentions pain, injury, or illness, log it with log_health\n\
- Keep responses concise -- this is a chat interface\n\
- Be encouraging but not patronizing\n\
- All action fields use metric units (weight_kg, distance_m). If the user specifies \
imperial, convert to metric in the action and mention the conversion in your message\n\
\n\
COLLECTING DATA BEFORE LOGGING:\n\
Do NOT emit any log_exercise action until you have ALL required data. Respond with \
\"actions\": [] while gathering info. Collect data across multiple messages using \
conversation history to build up the complete picture.\n\
\n\
For weight_reps exercises, you need: exercise name, total sets, reps, weight, and difficulty.\n\
\n\
1. SETS: Users often report one set at a time (e.g. \"bench 80kg 8 reps\", then later \
\"another set, 6 reps\"). When the user reports a set without specifying a total number \
of sets, ask if they are done with the exercise or have more sets to go. Keep a running \
count from conversation history. Only when they confirm they are finished (or report a \
specific total like \"3 sets\"), move on to ask about difficulty.\n\
2. DIFFICULTY: Once all sets are accounted for, ask how it felt: easy, medium, hard, or \
failure. Do not guess.\n\
3. FINAL LOG: Only when you have the exercise name, total sets, reps, weight, AND \
difficulty, emit the log_exercise action with the complete data.\n\
\n\
If the user reports everything in one message (e.g. \"3 sets bench 80kg 8 reps, felt hard\"), \
emit the action immediately -- no need to ask follow-ups.\n\
\n\
When the user answers a follow-up (e.g. \"done\", \"easy\", \"one more\"), use conversation \
history to reconstruct the context. Do not ask for information already provided.\n\
\n\
You may log partial data only when the user explicitly says to skip a field.\n\
\n\
GOALS: The same collect-before-emitting rule applies to set_goal. You need: exercise name, \
target value (e.g. target weight), and optionally an end date. If the user says \"I want to \
hit 100kg on bench\", ask by when they want to achieve it before emitting the action. If \
they say they don't have a deadline, emit with no end_date. Do not guess dates.\n\
\n\
CURRENT STATE:\n\
User: {user_name}\n\
Time: {current_time} ({timezone})\n\
Active session: {session_status}\n\
\n\
{health_section}\n\
{history_section}\n\
{goals_section}\n\
AVAILABLE EXERCISES:\n\
{exercise_list}",
        user_name = ctx.user_name,
        current_time = ctx.current_time,
        timezone = ctx.timezone,
    )
}

fn format_log_entry(log: &ExerciseLog, exercise_name: &str) -> String {
    let mut parts = vec![exercise_name.to_string()];
    if let Some(s) = log.sets {
        parts.push(format!("{s}x"));
    }
    if let Some(r) = log.reps {
        parts.push(format!("{r}reps"));
    }
    if let Some(w) = log.weight_kg {
        parts.push(format!("{w}kg"));
    }
    if let Some(d) = log.duration_secs {
        parts.push(format!("{d}s"));
    }
    if let Some(d) = log.distance_m {
        parts.push(format!("{d}m"));
    }
    parts.join(" ")
}

pub fn format_exercise_list(exercises: &[FullExercise]) -> String {
    let mut result = String::new();
    let mut current_group = "";

    for ex in exercises {
        if ex.muscle_group != current_group {
            current_group = &ex.muscle_group;
            result.push_str(&format!("\n## {}\n", capitalize(current_group)));
        }

        let aliases = ex.exercise.aliases.as_deref().map(|a| format!(" (aliases: {a})")).unwrap_or_default();

        let mt = match ex.exercise.measurement_type {
            MeasurementType::WeightReps => "weight_reps",
            MeasurementType::TimeBased => "time_based",
            MeasurementType::DistanceBased => "distance_based",
            MeasurementType::LevelBased => "level_based",
            MeasurementType::ScoreBased => "score_based",
        };

        result.push_str(&format!("- \"{}\"{aliases} [{mt}]\n", ex.exercise.name));
    }

    result
}

pub fn format_health_entries(entries: &[HealthEntry]) -> String {
    if entries.is_empty() {
        return "ACTIVE HEALTH ISSUES: None\n".to_string();
    }

    let mut result = "ACTIVE HEALTH ISSUES:\n".to_string();
    for entry in entries {
        let body = entry.body_part.as_deref().unwrap_or("general");
        result.push_str(&format!(
            "- {} ({}, {body}): {} (since {})\n",
            entry.entry_type, entry.severity, entry.description, entry.started_at,
        ));
    }
    result
}

pub fn format_recent_history(summaries: &[SessionSummary], logs: &[ExerciseLog], exercises: &[FullExercise]) -> String {
    if summaries.is_empty() && logs.is_empty() {
        return "RECENT HISTORY: No recent workouts\n".to_string();
    }

    let mut result = "RECENT HISTORY:\n".to_string();

    for summary in summaries {
        let duration = summary.duration_mins.map(|d| format!(" ({d} min)")).unwrap_or_default();
        let status = if summary.session.ended_at.is_some() { "completed" } else { "active" };
        result.push_str(&format!("- {} [{}]: {} exercises{duration}\n", summary.session.started_at, status, summary.exercise_count,));
    }

    if !logs.is_empty() {
        result.push_str("\nRecent exercises:\n");
        for log in logs.iter().take(10) {
            let name = exercises.iter().find(|e| e.exercise.id == log.exercise_id).map(|e| e.exercise.name.as_str()).unwrap_or("unknown");
            result.push_str(&format!("- {}: {}\n", log.logged_at, format_log_entry(log, name)));
        }
    }

    result
}

pub fn format_active_goals(goals: &[GoalProgress]) -> String {
    if goals.is_empty() {
        return "ACTIVE GOALS: None\n".to_string();
    }

    let mut result = "ACTIVE GOALS:\n".to_string();
    for gp in goals {
        let current = gp.current_value.map(|v| format!("{v:.1}")).unwrap_or_else(|| "N/A".to_string());
        let end = gp.goal.end_date.as_deref().map(|d| format!(" by {d}")).unwrap_or_default();
        result.push_str(&format!("- {}: {current}/{:.1}{end} ({:.0}%)\n", gp.exercise_name, gp.goal.target_value, gp.percentage,));
    }
    result
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().to_string() + &chars.as_str().replace('_', " "),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Difficulty, Exercise, GoalProgress, HealthEntry, HealthEntryType, MeasurementType, Session};
    use crate::db::{ExerciseGoal, GoalStatus};

    fn make_exercise(name: &str, aliases: &str, muscle_group: &str, mt: MeasurementType) -> FullExercise {
        FullExercise {
            exercise: Exercise {
                id: name.to_lowercase().replace(' ', "-"),
                name: name.to_string(),
                aliases: if aliases.is_empty() { None } else { Some(aliases.to_string()) },
                muscle_group_id: 1,
                purpose: "strength".to_string(),
                measurement_type: mt,
                description: None,
                created_at: String::new(),
            },
            muscle_group: muscle_group.to_string(),
        }
    }

    fn base_context() -> PromptContext {
        PromptContext {
            user_name: "Test User".to_string(),
            timezone: "Europe/London".to_string(),
            current_time: "2026-03-23 10:30:00".to_string(),
            active_session: None,
            session_logs: vec![],
            health_entries: vec![],
            recent_summaries: vec![],
            recent_logs: vec![],
            exercises: vec![
                make_exercise("Barbell Bench Press", "bench,bench press", "chest", MeasurementType::WeightReps),
                make_exercise("Running", "run,jogging", "cardio", MeasurementType::DistanceBased),
            ],
            active_goals: vec![],
            schedules: vec![],
        }
    }

    #[test]
    fn prompt_includes_no_active_session() {
        let ctx = base_context();
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("No active session"));
    }

    #[test]
    fn prompt_includes_active_session_with_logs() {
        let mut ctx = base_context();
        ctx.active_session = Some(Session {
            id: "s1".to_string(),
            user_id: "u1".to_string(),
            started_at: "2026-03-23 09:00:00".to_string(),
            ended_at: None,
            notes: None,
        });
        ctx.session_logs = vec![(
            ExerciseLog {
                id: "l1".to_string(),
                user_id: "u1".to_string(),
                exercise_id: "bench".to_string(),
                session_id: Some("s1".to_string()),
                logged_at: "2026-03-23 09:10:00".to_string(),
                sets: Some(3),
                reps: Some(8),
                weight_kg: Some(80.0),
                duration_secs: None,
                distance_m: None,
                level: None,
                difficulty: Difficulty::Medium,
                notes: None,
            },
            "Barbell Bench Press".to_string(),
        )];

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Active (started 2026-03-23 09:00:00)"));
        assert!(prompt.contains("Barbell Bench Press"));
        assert!(prompt.contains("80kg"));
    }

    #[test]
    fn prompt_includes_health_entries() {
        let mut ctx = base_context();
        ctx.health_entries = vec![HealthEntry {
            id: "h1".to_string(),
            user_id: "u1".to_string(),
            entry_type: HealthEntryType::Injury,
            body_part: Some("shoulder".to_string()),
            severity: "moderate".to_string(),
            description: "Rotator cuff pain".to_string(),
            started_at: "2026-03-20".to_string(),
            resolved_at: None,
            notes: None,
            updated_at: "2026-03-20".to_string(),
        }];

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Rotator cuff pain"));
        assert!(prompt.contains("shoulder"));
        assert!(prompt.contains("injury"));
    }

    #[test]
    fn prompt_no_health_entries() {
        let ctx = base_context();
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("ACTIVE HEALTH ISSUES: None"));
    }

    #[test]
    fn exercise_list_grouped_by_muscle_group() {
        let ctx = base_context();
        let list = format_exercise_list(&ctx.exercises);
        assert!(list.contains("## Cardio"));
        assert!(list.contains("## Chest"));
        assert!(list.contains("\"Barbell Bench Press\" (aliases: bench,bench press) [weight_reps]"));
        assert!(list.contains("\"Running\" (aliases: run,jogging) [distance_based]"));
    }

    #[test]
    fn format_goals() {
        let goals = vec![GoalProgress {
            goal: ExerciseGoal {
                id: "g1".to_string(),
                user_id: "u1".to_string(),
                exercise_id: "bench".to_string(),
                target_value: 100.0,
                start_date: "2026-01-01".to_string(),
                end_date: Some("2026-06-01".to_string()),
                achieved: false,
                notes: None,
                created_at: "2026-01-01".to_string(),
                updated_at: "2026-01-01".to_string(),
            },
            exercise_name: "Barbell Bench Press".to_string(),
            status: GoalStatus::Active,
            current_value: Some(80.0),
            percentage: 80.0,
        }];
        let text = format_active_goals(&goals);
        assert!(text.contains("Barbell Bench Press: 80.0/100.0 by 2026-06-01 (80%)"));
    }

    #[test]
    fn format_no_goals() {
        let text = format_active_goals(&[]);
        assert!(text.contains("ACTIVE GOALS: None"));
    }
}
