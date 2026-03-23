use std::sync::Arc;

use anyhow::Context as _;
use chrono::Utc;
use corre_core::app::{LlmProvider, LlmMessage, LlmRequest, LlmRole};
use tokio::sync::Mutex;

use crate::config::GymConfig;
use crate::db::{
    ConversationRole, Database, Difficulty, FullExercise, User,
    new_conversation_message, new_exercise_goal, new_exercise_log, new_health_entry, new_user,
};
use crate::telegram::Message as TgMessage;

use super::actions::AssistantAction;
use super::matching::find_exercise;
use super::parser::parse_assistant_response;
use super::prompts::{PromptContext, build_system_prompt};

pub struct AssistantHandler {
    db: Arc<Mutex<Database>>,
    llm: Box<dyn LlmProvider>,
    config: GymConfig,
    exercises: Vec<FullExercise>,
}

impl AssistantHandler {
    pub async fn new(db: Arc<Mutex<Database>>, llm: Box<dyn LlmProvider>, config: GymConfig) -> anyhow::Result<Self> {
        let exercises = db.lock().await.list_full_exercises()?;
        Ok(Self { db, llm, config, exercises })
    }

    /// Process an incoming text message and return a reply string.
    pub async fn handle_text_message(&self, message: &TgMessage, text: &str) -> anyhow::Result<String> {
        // 1. Identify user (auto-register if new)
        let (user, is_new) = self.ensure_user(message).await?;
        if is_new {
            return Ok(self.welcome_message(&user));
        }

        // 2. Check for slash commands
        if let Some(reply) = self.handle_command(text, &user).await? {
            return Ok(reply);
        }

        // 3. Auto-close stale sessions
        self.close_stale_session(&user).await?;

        // 4. Truncate message to max_message_length
        let text = if text.len() > self.config.max_message_length {
            &text[..self.config.max_message_length]
        } else {
            text
        };

        // 5. Build system prompt with current context
        let system_prompt = self.build_context(&user).await?;

        // 6. Load conversation history
        let history = {
            let db = self.db.lock().await;
            db.get_recent_messages_for_platform(&user.id, "telegram", self.config.conversation_history_limit)?
        };

        // 7. Call LLM
        let llm_response = self.call_llm(&system_prompt, &history, text).await?;

        // 8. Parse response
        let parsed = parse_assistant_response(&llm_response);

        // 9. Execute actions, track failures
        let mut failures: Vec<String> = Vec::new();
        for action in &parsed.actions {
            if let Err(e) = self.execute_action(action, &user).await {
                tracing::warn!("Action execution failed: {e:#}");
                failures.push(format!("{e:#}"));
            }
        }

        // 10. Build final reply
        let reply = if failures.is_empty() {
            parsed.message.clone()
        } else {
            format!("{}\n\n(Note: some actions failed: {})", parsed.message, failures.join("; "))
        };

        // 11. Store conversation turn
        self.store_conversation(&user.id, text, &parsed.message).await?;

        // 12. Prune old messages
        self.db
            .lock()
            .await
            .prune_old_messages(&user.id, self.config.conversation_history_limit * 2)?;

        Ok(reply)
    }

    async fn ensure_user(&self, message: &TgMessage) -> anyhow::Result<(User, bool)> {
        let from = message.from.as_ref().ok_or_else(|| anyhow::anyhow!("message has no sender"))?;
        let telegram_id = from.id.to_string();

        let db = self.db.lock().await;
        if let Some(user) = db.get_user_by_telegram_id(&telegram_id)? {
            return Ok((user, false));
        }

        let name = match &from.last_name {
            Some(last) => format!("{} {last}", from.first_name),
            None => from.first_name.clone(),
        };
        let user = new_user(&name, Some(&telegram_id), &self.config.default_timezone);
        db.insert_user(&user)?;
        tracing::info!("Registered new user: {} (telegram_id: {telegram_id})", user.name);
        Ok((user, true))
    }

    fn welcome_message(&self, user: &User) -> String {
        format!(
            "Welcome, {}! I'm your personal gym trainer assistant.\n\n\
             Here's what I can do:\n\
             - Track your exercises (just tell me what you did)\n\
             - Manage workout sessions\n\
             - Track injuries and health issues\n\
             - Set and monitor exercise goals\n\
             - Show your workout history\n\n\
             Try telling me something like:\n\
             \"I just did 3 sets of bench press at 80kg, 8 reps\"\n\n\
             Type /help for a list of commands.",
            user.name
        )
    }

    async fn handle_command(&self, text: &str, user: &User) -> anyhow::Result<Option<String>> {
        let cmd = text.split_whitespace().next().unwrap_or("").to_lowercase();
        match cmd.as_str() {
            "/start" => Ok(Some(self.cmd_start(user))),
            "/help" => Ok(Some(Self::cmd_help())),
            "/status" => Ok(Some(self.cmd_status(user).await?)),
            "/history" => Ok(Some(self.cmd_history(user).await?)),
            "/exercises" => Ok(Some(self.cmd_exercises())),
            _ => Ok(None),
        }
    }

    fn cmd_start(&self, user: &User) -> String {
        format!(
            "You're already registered, {}! Here's what I can help with:\n\
             - Tell me about your exercises and I'll log them\n\
             - /status -- see your current session\n\
             - /history -- recent workout summaries\n\
             - /exercises -- available exercises\n\
             - /help -- all commands",
            user.name
        )
    }

    fn cmd_help() -> String {
        "Available commands:\n\
         /start -- Introduction and registration\n\
         /status -- Current session and today's stats\n\
         /history -- Last 5 workout summaries\n\
         /exercises -- List available exercises by muscle group\n\
         /help -- This message\n\n\
         You can also just chat naturally:\n\
         - \"3 sets of bench press, 80kg, 8 reps\"\n\
         - \"I ran 5km in 25 minutes\"\n\
         - \"My shoulder is sore\"\n\
         - \"End my session\"\n\
         - \"What did I do today?\""
            .to_string()
    }

    async fn cmd_status(&self, user: &User) -> anyhow::Result<String> {
        let db = self.db.lock().await;
        let mut parts = vec![format!("Status for {}", user.name)];

        match db.get_active_session(&user.id)? {
            Some(session) => {
                let logs = db.get_logs_for_session(&session.id)?;
                parts.push(format!("Active session (started {})", session.started_at));
                if logs.is_empty() {
                    parts.push("  No exercises logged yet".to_string());
                } else {
                    for log in &logs {
                        let name = self
                            .exercises
                            .iter()
                            .find(|e| e.exercise.id == log.exercise_id)
                            .map(|e| e.exercise.name.as_str())
                            .unwrap_or("unknown");
                        parts.push(format!("  - {name}: {}", format_log_compact(log)));
                    }
                }
            }
            None => parts.push("No active session".to_string()),
        }

        let health = db.list_active_health_entries(&user.id)?;
        if !health.is_empty() {
            parts.push(String::new());
            parts.push("Active health issues:".to_string());
            for entry in &health {
                let body = entry.body_part.as_deref().unwrap_or("general");
                parts.push(format!("  - {} ({body}): {}", entry.entry_type, entry.description));
            }
        }

        Ok(parts.join("\n"))
    }

    async fn cmd_history(&self, user: &User) -> anyhow::Result<String> {
        let db = self.db.lock().await;
        let summaries = db.list_session_summaries(&user.id, None, None)?;

        if summaries.is_empty() {
            return Ok("No workout history yet. Start by telling me about an exercise!".to_string());
        }

        let mut parts = vec!["Recent workouts:".to_string()];
        for summary in summaries.iter().take(5) {
            let duration = summary
                .duration_mins
                .map(|d| format!(" ({d} min)"))
                .unwrap_or_default();
            let status = if summary.session.ended_at.is_some() { "done" } else { "active" };
            parts.push(format!(
                "- {} [{status}]: {} exercises{duration}",
                summary.session.started_at, summary.exercise_count,
            ));
        }

        Ok(parts.join("\n"))
    }

    fn cmd_exercises(&self) -> String {
        use super::prompts::format_exercise_list;
        let mut result = "Available exercises:\n".to_string();
        result.push_str(&format_exercise_list(&self.exercises));
        result
    }

    async fn close_stale_session(&self, user: &User) -> anyhow::Result<()> {
        let db = self.db.lock().await;
        let Some(session) = db.get_active_session(&user.id)? else {
            return Ok(());
        };

        // Find the last activity time: most recent log or session start
        let logs = db.get_logs_for_session(&session.id)?;
        let last_activity = logs
            .last()
            .map(|l| l.logged_at.as_str())
            .unwrap_or(&session.started_at);

        let threshold_hours = self.config.session_timeout_hours as i64;
        if let Ok(last) = chrono::NaiveDateTime::parse_from_str(last_activity, "%Y-%m-%d %H:%M:%S") {
            let elapsed = Utc::now().naive_utc() - last;
            if elapsed.num_hours() >= threshold_hours {
                tracing::info!("Auto-closing stale session {} (last activity: {last_activity})", session.id);
                db.end_session(&session.id)?;
            }
        }

        Ok(())
    }

    async fn build_context(&self, user: &User) -> anyhow::Result<String> {
        let db = self.db.lock().await;
        let active_session = db.get_active_session(&user.id)?;
        let session_logs = match &active_session {
            Some(session) => {
                let logs = db.get_logs_for_session(&session.id)?;
                logs.into_iter()
                    .map(|log| {
                        let name = self
                            .exercises
                            .iter()
                            .find(|e| e.exercise.id == log.exercise_id)
                            .map(|e| e.exercise.name.clone())
                            .unwrap_or_else(|| "unknown".to_string());
                        (log, name)
                    })
                    .collect()
            }
            None => vec![],
        };
        let health_entries = db.list_active_health_entries(&user.id)?;
        let recent_summaries = db.list_session_summaries(&user.id, None, None)?;
        let recent_summaries: Vec<_> = recent_summaries.into_iter().take(5).collect();
        let recent_logs = db.get_recent_logs(&user.id, 7)?;
        let recent_logs: Vec<_> = recent_logs.into_iter().take(10).collect();
        let active_goals = db.goal_progress_report(&user.id, None, None)?;
        let schedules = db.list_schedules(&user.id)?;

        let ctx = PromptContext {
            user_name: user.name.clone(),
            timezone: user.timezone.clone(),
            current_time: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            active_session,
            session_logs,
            health_entries,
            recent_summaries,
            recent_logs,
            exercises: self.exercises.clone(),
            active_goals,
            schedules,
        };

        Ok(build_system_prompt(&ctx))
    }

    async fn call_llm(
        &self,
        system_prompt: &str,
        history: &[crate::db::ConversationMessage],
        user_text: &str,
    ) -> anyhow::Result<String> {
        let mut messages = vec![LlmMessage {
            role: LlmRole::System,
            content: system_prompt.to_string(),
        }];

        // Add conversation history as user/assistant pairs
        for msg in history {
            let role = match msg.role {
                ConversationRole::User => LlmRole::User,
                ConversationRole::Assistant => LlmRole::Assistant,
                ConversationRole::System => LlmRole::System,
            };
            messages.push(LlmMessage {
                role,
                content: msg.content.clone(),
            });
        }

        // Add current user message
        messages.push(LlmMessage {
            role: LlmRole::User,
            content: user_text.to_string(),
        });

        let request = LlmRequest {
            messages,
            temperature: Some(0.3),
            max_completion_tokens: Some(1024),
            json_mode: false,
        };

        let response = self.llm.complete(request).await.context("LLM completion failed")?;
        Ok(response.content)
    }

    async fn execute_action(&self, action: &AssistantAction, user: &User) -> anyhow::Result<()> {
        match action {
            AssistantAction::LogExercise { exercise, sets, reps, weight_kg, difficulty } => {
                let ex =
                    find_exercise(&self.exercises, exercise).ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let session = self.ensure_session(user).await?;
                let mut log = new_exercise_log(&user.id, &ex.exercise.id, Some(&session.id));
                log.sets = *sets;
                log.reps = *reps;
                log.weight_kg = *weight_kg;
                log.difficulty = difficulty.unwrap_or(Difficulty::Medium);
                self.db.lock().await.insert_log(&log)?;
            }
            AssistantAction::LogExerciseTimed { exercise, duration_secs, difficulty } => {
                let ex =
                    find_exercise(&self.exercises, exercise).ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let session = self.ensure_session(user).await?;
                let mut log = new_exercise_log(&user.id, &ex.exercise.id, Some(&session.id));
                log.duration_secs = Some(*duration_secs);
                log.difficulty = difficulty.unwrap_or(Difficulty::Medium);
                self.db.lock().await.insert_log(&log)?;
            }
            AssistantAction::LogExerciseDistance { exercise, distance_m, duration_secs, difficulty } => {
                let ex =
                    find_exercise(&self.exercises, exercise).ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let session = self.ensure_session(user).await?;
                let mut log = new_exercise_log(&user.id, &ex.exercise.id, Some(&session.id));
                log.distance_m = *distance_m;
                log.duration_secs = *duration_secs;
                log.difficulty = difficulty.unwrap_or(Difficulty::Medium);
                self.db.lock().await.insert_log(&log)?;
            }
            AssistantAction::StartSession { notes } => {
                let db = self.db.lock().await;
                if db.get_active_session(&user.id)?.is_none() {
                    db.start_session(&user.id, notes.as_deref())?;
                }
            }
            AssistantAction::EndSession => {
                let db = self.db.lock().await;
                if let Some(session) = db.get_active_session(&user.id)? {
                    db.end_session(&session.id)?;
                }
            }
            AssistantAction::LogHealth { entry_type, body_part, severity, description } => {
                let mut entry = new_health_entry(&user.id, *entry_type, description);
                entry.body_part = body_part.clone();
                if let Some(sev) = severity {
                    entry.severity = sev.clone();
                }
                self.db.lock().await.insert_health_entry(&entry)?;
            }
            AssistantAction::ResolveHealth { description } => {
                let db = self.db.lock().await;
                let entries = db.list_active_health_entries(&user.id)?;
                if let Some(entry) = entries
                    .iter()
                    .find(|e| e.description.to_lowercase().contains(&description.to_lowercase()))
                {
                    db.resolve_health_entry(&entry.id)?;
                }
            }
            AssistantAction::SetGoal { exercise, target_value, end_date } => {
                let ex =
                    find_exercise(&self.exercises, exercise).ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let mut goal = new_exercise_goal(&user.id, &ex.exercise.id, *target_value);
                goal.end_date = end_date.clone();
                self.db.lock().await.insert_goal(&goal)?;
            }
            AssistantAction::Unknown => {
                tracing::debug!("Ignoring unknown action type from LLM");
            }
        }
        Ok(())
    }

    async fn ensure_session(&self, user: &User) -> anyhow::Result<crate::db::Session> {
        let db = self.db.lock().await;
        if let Some(session) = db.get_active_session(&user.id)? {
            return Ok(session);
        }
        db.start_session(&user.id, None)
    }

    async fn store_conversation(&self, user_id: &str, user_text: &str, assistant_text: &str) -> anyhow::Result<()> {
        let db = self.db.lock().await;
        let user_msg = new_conversation_message(user_id, "telegram", ConversationRole::User, user_text);
        db.insert_message(&user_msg)?;
        let assistant_msg = new_conversation_message(user_id, "telegram", ConversationRole::Assistant, assistant_text);
        db.insert_message(&assistant_msg)?;
        Ok(())
    }
}

fn format_log_compact(log: &crate::db::ExerciseLog) -> String {
    let mut parts = Vec::new();
    if let Some(s) = log.sets {
        parts.push(format!("{s} sets"));
    }
    if let Some(r) = log.reps {
        parts.push(format!("{r} reps"));
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
    if parts.is_empty() {
        "no details".to_string()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use corre_core::app::{LlmRequest, LlmResponse};

    struct MockLlm {
        response: std::sync::Mutex<String>,
    }

    impl MockLlm {
        fn new(response: &str) -> Self {
            Self {
                response: std::sync::Mutex::new(response.to_string()),
            }
        }

        fn set_response(&self, response: &str) {
            *self.response.lock().unwrap() = response.to_string();
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockLlm {
        async fn complete(&self, _request: LlmRequest) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse {
                content: self.response.lock().unwrap().clone(),
            })
        }
    }

    fn make_message(user_id: i64, text: &str) -> TgMessage {
        TgMessage {
            message_id: 1,
            from: Some(crate::telegram::TelegramUser {
                id: user_id,
                first_name: "Test".to_string(),
                last_name: Some("User".to_string()),
                username: Some("testuser".to_string()),
            }),
            chat: crate::telegram::Chat {
                id: user_id,
                chat_type: "private".to_string(),
            },
            date: 0,
            text: Some(text.to_string()),
            voice: None,
        }
    }

    fn test_config() -> GymConfig {
        GymConfig {
            bind: "127.0.0.1:5520".to_string(),
            telegram_bot_token: "123456:ABC".to_string(),
            telegram_allowed_ids: vec![],
            default_timezone: "UTC".to_string(),
            conversation_history_limit: 20,
            db_path: "test.db".to_string(),
            max_message_length: 2000,
            session_timeout_hours: 4,
            llm: None,
        }
    }

    async fn setup_handler(response: &str) -> (AssistantHandler, Arc<MockLlm>) {
        let db = Database::open_in_memory().unwrap();
        db.seed_exercises().unwrap();
        let db = Arc::new(Mutex::new(db));
        let llm = Arc::new(MockLlm::new(response));
        let handler = AssistantHandler::new(db, Box::new(MockLlmWrapper(llm.clone())), test_config()).await.unwrap();
        (handler, llm)
    }

    // Wrapper to let Arc<MockLlm> implement LlmProvider
    struct MockLlmWrapper(Arc<MockLlm>);

    #[async_trait::async_trait]
    impl LlmProvider for MockLlmWrapper {
        async fn complete(&self, request: LlmRequest) -> anyhow::Result<LlmResponse> {
            self.0.complete(request).await
        }
    }

    #[tokio::test]
    async fn user_auto_registration() {
        let (handler, _) = setup_handler(r#"{"message": "Hello!", "actions": []}"#).await;
        let msg = make_message(12345, "hello");
        let reply = handler.handle_text_message(&msg, "hello").await.unwrap();
        assert!(reply.contains("Welcome"));
    }

    #[tokio::test]
    async fn existing_user_gets_llm_response() {
        let (handler, _) = setup_handler(r#"{"message": "Got it!", "actions": []}"#).await;
        let msg = make_message(12345, "hello");

        // First message registers
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        // Second message goes through LLM
        let reply = handler.handle_text_message(&msg, "how are you").await.unwrap();
        assert_eq!(reply, "Got it!");
    }

    #[tokio::test]
    async fn exercise_logging_creates_records() {
        let response = r#"{"message": "Logged your bench press!", "actions": [
            {"type": "log_exercise", "exercise": "Barbell Bench Press", "sets": 3, "reps": 8, "weight_kg": 80.0, "difficulty": "medium"}
        ]}"#;
        let (handler, _) = setup_handler(response).await;
        let msg = make_message(12345, "hello");

        // Register first
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        // Log exercise
        let reply = handler.handle_text_message(&msg, "3 sets bench 80kg 8 reps").await.unwrap();
        assert_eq!(reply, "Logged your bench press!");

        // Verify DB records
        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let session = db.get_active_session(&user.id).unwrap();
        assert!(session.is_some());
        let logs = db.get_logs_for_session(&session.unwrap().id).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].sets, Some(3));
        assert_eq!(logs[0].reps, Some(8));
        assert_eq!(logs[0].weight_kg, Some(80.0));
    }

    #[tokio::test]
    async fn session_auto_start() {
        let response =
            r#"{"message": "Logged!", "actions": [{"type": "log_exercise", "exercise": "Barbell Bench Press", "sets": 1, "reps": 1}]}"#;
        let (handler, _) = setup_handler(response).await;
        let msg = make_message(12345, "hello");

        // Register
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();

        // Verify no session yet
        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        assert!(db.get_active_session(&user.id).unwrap().is_none());
        drop(db);

        // Log exercise -- should auto-start session
        let _ = handler.handle_text_message(&msg, "bench press").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        assert!(db.get_active_session(&user.id).unwrap().is_some());
    }

    #[tokio::test]
    async fn slash_help_command() {
        let (handler, _) = setup_handler(r#"{"message": "x", "actions": []}"#).await;
        let msg = make_message(12345, "/help");

        // Register first
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "/help").await.unwrap();
        assert!(reply.contains("Available commands"));
    }

    #[tokio::test]
    async fn slash_start_existing_user() {
        let (handler, _) = setup_handler(r#"{"message": "x", "actions": []}"#).await;
        let msg = make_message(12345, "/start");

        // Register first
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "/start").await.unwrap();
        assert!(reply.contains("already registered"));
    }

    #[tokio::test]
    async fn multiple_actions_execute() {
        let response = r#"{"message": "Started session and logged exercise!", "actions": [
            {"type": "start_session", "notes": "Chest day"},
            {"type": "log_exercise", "exercise": "Barbell Bench Press", "sets": 3, "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, _) = setup_handler(response).await;
        let msg = make_message(12345, "hello");

        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "start chest day, 3x8 bench 80kg").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let session = db.get_active_session(&user.id).unwrap().unwrap();
        assert_eq!(session.notes.as_deref(), Some("Chest day"));
        let logs = db.get_logs_for_session(&session.id).unwrap();
        assert_eq!(logs.len(), 1);
    }

    #[tokio::test]
    async fn partial_action_failure_appends_note() {
        // Second action has an invalid exercise name
        let response = r#"{"message": "Tried to log both!", "actions": [
            {"type": "start_session"},
            {"type": "log_exercise", "exercise": "Nonexistent Exercise 99", "sets": 3, "reps": 8}
        ]}"#;
        let (handler, _) = setup_handler(response).await;
        let msg = make_message(12345, "hello");

        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "do stuff").await.unwrap();
        assert!(reply.contains("some actions failed"));
    }

    #[tokio::test]
    async fn message_truncation() {
        let (handler, llm) = setup_handler(r#"{"message": "ok", "actions": []}"#).await;
        let msg = make_message(12345, "hello");

        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();

        // Create a message longer than max_message_length (2000)
        let long_text = "a".repeat(3000);
        llm.set_response(r#"{"message": "received", "actions": []}"#);
        let reply = handler.handle_text_message(&msg, &long_text).await.unwrap();
        assert_eq!(reply, "received");

        // Verify stored message was truncated
        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let msgs = db.get_recent_messages_for_platform(&user.id, "telegram", 10).unwrap();
        let user_msgs: Vec<_> = msgs.iter().filter(|m| m.role == ConversationRole::User).collect();
        let last_user_msg = user_msgs.last().unwrap();
        assert_eq!(last_user_msg.content.len(), 2000);
    }
}
