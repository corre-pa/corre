use std::sync::Arc;

use anyhow::Context as _;
use chrono::Utc;
use corre_core::app::{LlmMessage, LlmProvider, LlmRequest, LlmRole};
use tokio::sync::Mutex;

use crate::config::GymConfig;
use crate::db::{
    ConversationRole, Database, Difficulty, ExerciseSet, ExerciseTypeWithAncestry, MeasurementType, User, new_conversation_message,
    new_exercise_entry, new_exercise_goal, new_exercise_set, new_health_entry, new_user,
};
use crate::telegram::Message as TgMessage;

use super::actions::AssistantAction;
use super::matching::find_exercise_type;
use super::parser::parse_assistant_response;
use super::prompts::{PromptContext, build_system_prompt};

pub struct Reply {
    pub text: String,
    pub parse_mode: Option<&'static str>,
}

impl Reply {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), parse_mode: None }
    }

    pub fn as_html(&mut self) {
        self.parse_mode = Some("HTML");
    }
}

pub struct AssistantHandler {
    db: Arc<Mutex<Database>>,
    llm: Box<dyn LlmProvider>,
    config: GymConfig,
    catalogue: Vec<ExerciseTypeWithAncestry>,
}

impl AssistantHandler {
    pub async fn new(db: Arc<Mutex<Database>>, llm: Box<dyn LlmProvider>, config: GymConfig) -> anyhow::Result<Self> {
        let catalogue = db.lock().await.list_exercise_types_with_ancestry()?;
        Ok(Self { db, llm, config, catalogue })
    }

    /// Process an incoming Telegram text message and return a reply.
    pub async fn handle_text_message(&self, message: &TgMessage, text: &str) -> anyhow::Result<Reply> {
        let (user, is_new) = self.ensure_user(message).await?;
        if is_new {
            return Ok(Reply::new(self.welcome_message(&user)));
        }
        self.handle_message_for_user(&user, text, "telegram").await
    }

    pub async fn handle_message_for_user(&self, user: &User, text: &str, platform: &str) -> anyhow::Result<Reply> {
        if let Some(reply) = self.handle_command(text, user, platform).await? {
            return Ok(reply);
        }

        self.close_stale_session(user).await?;

        let text = if text.len() > self.config.max_message_length { &text[..self.config.max_message_length] } else { text };

        let system_prompt = self.build_context(user).await?;

        let history = {
            let db = self.db.lock().await;
            db.get_recent_messages_for_platform(user.id, platform, self.config.conversation_history_limit)?
        };

        let llm_response = match self.call_llm(&system_prompt, &history, text).await {
            Ok(response) => response,
            Err(e) => {
                let err_msg = format!("{e:#}");
                tracing::error!("LLM call failed: {err_msg}");
                let error_reply = if err_msg.contains("401") || err_msg.contains("Unauthorized") || err_msg.contains("Authentication") {
                    "I could not access the AI engine. You'll need to check that I'm properly configured with a valid API key."
                } else {
                    "I had trouble processing that -- could you try again?"
                };
                self.store_excluded_conversation_on_platform(user.id, platform, text, error_reply).await?;
                return Ok(Reply::new(error_reply));
            }
        };

        let parsed = parse_assistant_response(&llm_response);

        let is_refusal = is_refusal_response(&parsed.message);
        if is_refusal {
            tracing::info!("LLM response detected as refusal, excluding from context");
        }

        let mut failures: Vec<String> = Vec::new();
        for action in &parsed.actions {
            if let Err(e) = self.execute_action(action, user).await {
                tracing::warn!("Action execution failed: {e:#}");
                failures.push(format!("{e:#}"));
            }
        }

        let reply = if failures.is_empty() {
            parsed.message.clone()
        } else {
            format!("{}\n\n(Note: some actions failed: {})", parsed.message, failures.join("; "))
        };

        if is_refusal {
            self.store_excluded_conversation_on_platform(user.id, platform, text, &parsed.message).await?;
        } else {
            self.store_conversation_on_platform(user.id, platform, text, &parsed.message).await?;
        }

        self.db.lock().await.prune_old_messages(user.id, self.config.conversation_history_limit * 2)?;

        Ok(Reply::new(reply))
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
        let draft = new_user(&name, Some(&telegram_id), &self.config.default_timezone);
        let user_id = db.insert_user(&draft)?;
        let user = db.get_user(user_id)?.context("user disappeared after insert")?;
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

    async fn handle_command(&self, text: &str, user: &User, platform: &str) -> anyhow::Result<Option<Reply>> {
        let cmd = text.split_whitespace().next().unwrap_or("").to_lowercase();
        match cmd.as_str() {
            "/start" => Ok(Some(Reply::new(self.cmd_start(user)))),
            "/help" => Ok(Some(Reply::new(Self::cmd_help()))),
            "/status" => Ok(Some(self.cmd_status(user).await?)),
            "/history" => Ok(Some(Reply::new(self.cmd_history(user).await?))),
            "/exercises" => Ok(Some(self.cmd_exercises())),
            "/clear" => Ok(Some(Reply::new(self.cmd_clear(user, platform).await?))),
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
             - /clear -- clear conversation context\n\
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
         /clear -- Clear conversation context (fresh start)\n\
         /help -- This message\n\n\
         You can also just chat naturally:\n\
         - \"3 sets of bench press, 80kg, 8 reps\"\n\
         - \"I ran 5km in 25 minutes\"\n\
         - \"My shoulder is sore\"\n\
         - \"End my session\"\n\
         - \"What did I do today?\""
            .to_string()
    }

    async fn cmd_status(&self, user: &User) -> anyhow::Result<Reply> {
        let db = self.db.lock().await;
        let mut result = format!("<b>Status for {}</b>\n", escape_html(&user.name));

        match db.get_active_session(user.id)? {
            Some(session) => {
                let entries = db.list_entries_for_session(session.id)?;
                result.push_str(&format!("\n<b>Active session</b> (started {})\n", escape_html(&session.started_at)));
                if entries.is_empty() {
                    result.push_str("No exercises logged yet\n");
                } else {
                    for entry in &entries {
                        let sets = db.list_sets_for_entry(entry.id)?;
                        for set in &sets {
                            let name = self
                                .catalogue
                                .iter()
                                .find(|e| e.exercise_type.id == set.exercise_type_id)
                                .map(|e| e.exercise_type.name.as_str())
                                .unwrap_or("unknown");
                            result.push_str(&format!("- <b>{}</b>: {}\n", escape_html(name), escape_html(&format_set_compact(set))));
                        }
                    }
                }
            }
            None => result.push_str("No active session\n"),
        }

        let health = db.list_active_health_entries(user.id)?;
        if !health.is_empty() {
            result.push_str("\n<b>Active health issues</b>\n");
            for entry in &health {
                let body = entry.body_part.as_deref().unwrap_or("general");
                result
                    .push_str(&format!("- {} ({}): {}\n", entry.entry_type.as_str(), escape_html(body), escape_html(&entry.description),));
            }
        }

        let mut reply = Reply::new(result);
        reply.as_html();
        Ok(reply)
    }

    async fn cmd_history(&self, user: &User) -> anyhow::Result<String> {
        let db = self.db.lock().await;
        let summaries = db.list_session_summaries(user.id, None, None)?;

        if summaries.is_empty() {
            return Ok("No workout history yet. Start by telling me about an exercise!".to_string());
        }

        let mut parts = vec!["Recent workouts:".to_string()];
        for summary in summaries.iter().take(5) {
            let duration = summary.duration_mins.map(|d| format!(" ({d} min)")).unwrap_or_default();
            let status = if summary.session.ended_at.is_some() { "done" } else { "active" };
            parts.push(format!("- {} [{status}]: {} entries{duration}", summary.session.started_at, summary.exercise_count));
        }

        Ok(parts.join("\n"))
    }

    fn cmd_exercises(&self) -> Reply {
        use super::prompts::capitalize;

        let mut result = String::new();

        let mut groups: Vec<(&str, Vec<(&str, &str, &str)>)> = Vec::new();
        for et in &self.catalogue {
            if !matches!(et.exercise_type.level, crate::db::ExerciseLevel::Exercise | crate::db::ExerciseLevel::Variation) {
                continue;
            }
            let group = et.muscle_group.as_deref().unwrap_or("Other");
            let aliases = et.exercise_type.aliases.as_deref().unwrap_or("");
            let mt = et.exercise_type.measurement_type.map(|m| m.as_str()).unwrap_or("weight_reps");
            match groups.last_mut() {
                Some((g, rows)) if *g == group => rows.push((&et.exercise_type.name, aliases, mt)),
                _ => groups.push((group, vec![(&et.exercise_type.name, aliases, mt)])),
            }
        }

        for (group, rows) in &groups {
            let name_w = rows.iter().map(|(n, _, _)| n.len()).max().unwrap_or(4).max(4);
            let alias_w = rows.iter().map(|(_, a, _)| a.len()).max().unwrap_or(7).max(7);

            result.push_str(&format!("\n<b>{}</b>\n<pre>", escape_html(&capitalize(group))));
            result.push_str(&format!("{:<name_w$} | {:<alias_w$} | Type\n", "Name", "Aliases"));
            result.push_str(&format!("{:-<name_w$}-+-{:-<alias_w$}-+------\n", "", ""));
            for (name, aliases, mt) in rows {
                result.push_str(&format!("{:<name_w$} | {:<alias_w$} | {mt}\n", escape_html(name), escape_html(aliases),));
            }
            result.push_str("</pre>");
        }

        let mut reply = Reply::new(result);
        reply.as_html();
        reply
    }

    async fn close_stale_session(&self, user: &User) -> anyhow::Result<()> {
        let db = self.db.lock().await;
        let Some(session) = db.get_active_session(user.id)? else {
            return Ok(());
        };

        let entries = db.list_entries_for_session(session.id)?;
        let last_activity = entries.last().map(|e| e.start_timestamp.clone()).unwrap_or_else(|| session.started_at.clone());

        let threshold_hours = self.config.session_timeout_hours as i64;
        if let Ok(last) = chrono::NaiveDateTime::parse_from_str(&last_activity, "%Y-%m-%d %H:%M:%S") {
            let elapsed = Utc::now().naive_utc() - last;
            if elapsed.num_hours() >= threshold_hours {
                tracing::info!("Auto-closing stale session {} (last activity: {last_activity})", session.id);
                db.end_session(session.id)?;
            }
        }

        Ok(())
    }

    async fn build_context(&self, user: &User) -> anyhow::Result<String> {
        let db = self.db.lock().await;
        let active_session = db.get_active_session(user.id)?;
        let session_sets = match &active_session {
            Some(session) => {
                let entries = db.list_entries_for_session(session.id)?;
                let mut all_sets: Vec<(ExerciseSet, String)> = Vec::new();
                for entry in &entries {
                    let sets = db.list_sets_for_entry(entry.id)?;
                    for set in sets {
                        let name = self
                            .catalogue
                            .iter()
                            .find(|e| e.exercise_type.id == set.exercise_type_id)
                            .map(|e| e.exercise_type.name.clone())
                            .unwrap_or_else(|| "unknown".to_string());
                        all_sets.push((set, name));
                    }
                }
                all_sets
            }
            None => vec![],
        };
        let health_entries = db.list_active_health_entries(user.id)?;
        let recent_summaries = db.list_session_summaries(user.id, None, None)?;
        let recent_summaries: Vec<_> = recent_summaries.into_iter().take(5).collect();
        let recent_sets = db.list_recent_sets(user.id, 7)?;
        let session_set_ids: std::collections::HashSet<i64> = session_sets.iter().map(|(s, _)| s.id).collect();
        let recent_sets: Vec<_> = recent_sets.into_iter().filter(|s| !session_set_ids.contains(&s.id)).take(10).collect();
        let active_goals = db.goal_progress_report(user.id, None, None)?;
        let schedules = db.list_schedules(user.id)?;

        let ctx = PromptContext {
            user_name: user.name.clone(),
            timezone: user.timezone.clone(),
            current_time: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            active_session,
            session_sets,
            health_entries,
            recent_summaries,
            recent_sets,
            exercise_types: self.catalogue.clone(),
            active_goals,
            schedules,
        };

        Ok(build_system_prompt(&ctx))
    }

    async fn call_llm(&self, system_prompt: &str, history: &[crate::db::ConversationMessage], user_text: &str) -> anyhow::Result<String> {
        let mut messages = vec![LlmMessage { role: LlmRole::System, content: system_prompt.to_string() }];

        for msg in history {
            let role = match msg.role {
                ConversationRole::User => LlmRole::User,
                ConversationRole::Assistant => LlmRole::Assistant,
                ConversationRole::System => LlmRole::System,
            };
            messages.push(LlmMessage { role, content: msg.content.clone() });
        }

        messages.push(LlmMessage { role: LlmRole::User, content: user_text.to_string() });

        let request = LlmRequest { messages, temperature: Some(0.3), max_completion_tokens: Some(1024), json_mode: false };

        let response = self.llm.complete(request).await.context("LLM completion failed")?;
        Ok(response.content)
    }

    async fn execute_action(&self, action: &AssistantAction, user: &User) -> anyhow::Result<()> {
        tracing::debug!(action = ?action, user_id = user.id, "Executing action");
        match action {
            AssistantAction::LogExercise { exercise, sets, reps, weight_kg, perceived_difficulty, comment } => {
                let et = find_exercise_type(&self.catalogue, exercise)
                    .ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let session = self.ensure_session(user).await?;
                let entry_id = self.ensure_entry(user.id, session.id).await?;
                let n_sets = sets.unwrap_or(1).max(1);
                let weight = weight_kg.unwrap_or(0.0);
                let pd = perceived_difficulty.unwrap_or(Difficulty::Medium);
                let db = self.db.lock().await;
                for i in 0..n_sets {
                    let mut s = new_exercise_set(entry_id, et.exercise_type.id, MeasurementType::WeightReps, weight);
                    s.count = *reps;
                    s.order_idx = i;
                    s.perceived_difficulty = pd;
                    s.comment = comment.clone();
                    db.insert_set(&s)?;
                }
            }
            AssistantAction::LogExerciseTimed { exercise, duration_secs, perceived_difficulty, comment } => {
                let et = find_exercise_type(&self.catalogue, exercise)
                    .ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let session = self.ensure_session(user).await?;
                let entry_id = self.ensure_entry(user.id, session.id).await?;
                let mut s = new_exercise_set(entry_id, et.exercise_type.id, MeasurementType::TimeBased, *duration_secs as f64);
                s.perceived_difficulty = perceived_difficulty.unwrap_or(Difficulty::Medium);
                s.comment = comment.clone();
                self.db.lock().await.insert_set(&s)?;
            }
            AssistantAction::LogExerciseDistance { exercise, distance_m, duration_secs, perceived_difficulty, comment } => {
                let et = find_exercise_type(&self.catalogue, exercise)
                    .ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let session = self.ensure_session(user).await?;
                let entry_id = self.ensure_entry(user.id, session.id).await?;
                let value = distance_m.unwrap_or_else(|| duration_secs.unwrap_or(0) as f64);
                let mt = if distance_m.is_some() { MeasurementType::DistanceBased } else { MeasurementType::TimeBased };
                let mut s = new_exercise_set(entry_id, et.exercise_type.id, mt, value);
                s.perceived_difficulty = perceived_difficulty.unwrap_or(Difficulty::Medium);
                s.comment = comment.clone();
                self.db.lock().await.insert_set(&s)?;
            }
            AssistantAction::StartSession { notes } => {
                let db = self.db.lock().await;
                if db.get_active_session(user.id)?.is_none() {
                    let session = db.start_session(user.id, notes.as_deref())?;
                    tracing::debug!(id = session.id, notes = ?notes, "Started session");
                } else {
                    tracing::debug!("Session already active, skipping start");
                }
            }
            AssistantAction::EndSession => {
                let db = self.db.lock().await;
                if let Some(session) = db.get_active_session(user.id)? {
                    tracing::debug!(id = session.id, "Ending session");
                    db.end_session(session.id)?;
                } else {
                    tracing::debug!("No active session to end");
                }
            }
            AssistantAction::LogHealth { entry_type, body_part, severity, description } => {
                let mut entry = new_health_entry(user.id, *entry_type, description);
                entry.body_part = body_part.clone();
                if let Some(sev) = severity {
                    entry.severity = sev.clone();
                }
                tracing::debug!(entry_type = ?entry_type, body_part = ?body_part, severity = ?severity, "Inserting health entry");
                self.db.lock().await.insert_health_entry(&entry)?;
            }
            AssistantAction::ResolveHealth { description } => {
                let db = self.db.lock().await;
                let entries = db.list_active_health_entries(user.id)?;
                if let Some(entry) = entries.iter().find(|e| e.description.to_lowercase().contains(&description.to_lowercase())) {
                    tracing::debug!(id = entry.id, description = %entry.description, "Resolving health entry");
                    db.resolve_health_entry(entry.id)?;
                } else {
                    tracing::debug!(search = %description, "No matching health entry found to resolve");
                }
            }
            AssistantAction::SetGoal { exercise, target_value, end_date } => {
                let et = find_exercise_type(&self.catalogue, exercise)
                    .ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let mut goal = new_exercise_goal(user.id, et.exercise_type.id, *target_value);
                goal.end_date = end_date.clone();
                tracing::debug!(exercise = %et.exercise_type.name, target = %target_value, end_date = ?end_date, "Inserting goal");
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
        if let Some(session) = db.get_active_session(user.id)? {
            return Ok(session);
        }
        db.start_session(user.id, None)
    }

    /// Reuse an open exercise_entry within the last 30 minutes, or create a new one.
    async fn ensure_entry(&self, user_id: i64, session_id: i64) -> anyhow::Result<i64> {
        let db = self.db.lock().await;
        if let Some(open) = db.latest_open_entry(user_id, 30)?
            && open.session_id == Some(session_id)
        {
            return Ok(open.id);
        }
        let entry = new_exercise_entry(user_id, Some(session_id), None);
        db.insert_entry(&entry)
    }

    async fn cmd_clear(&self, user: &User, platform: &str) -> anyhow::Result<String> {
        let db = self.db.lock().await;
        let excluded = db.exclude_all_messages_for_platform(user.id, platform)?;
        tracing::info!(user_id = user.id, %platform, excluded, "Cleared conversation context");
        Ok("Conversation context cleared. I'll start fresh from here.".to_string())
    }

    async fn store_conversation_on_platform(
        &self,
        user_id: i64,
        platform: &str,
        user_text: &str,
        assistant_text: &str,
    ) -> anyhow::Result<()> {
        let db = self.db.lock().await;
        db.insert_message(&new_conversation_message(user_id, platform, ConversationRole::User, user_text))?;
        db.insert_message(&new_conversation_message(user_id, platform, ConversationRole::Assistant, assistant_text))?;
        Ok(())
    }

    async fn store_excluded_conversation_on_platform(
        &self,
        user_id: i64,
        platform: &str,
        user_text: &str,
        assistant_text: &str,
    ) -> anyhow::Result<()> {
        let db = self.db.lock().await;
        let mut user_msg = new_conversation_message(user_id, platform, ConversationRole::User, user_text);
        user_msg.exclude_from_context = true;
        db.insert_message(&user_msg)?;
        let mut assistant_msg = new_conversation_message(user_id, platform, ConversationRole::Assistant, assistant_text);
        assistant_msg.exclude_from_context = true;
        db.insert_message(&assistant_msg)?;
        Ok(())
    }
}

/// Detect LLM refusal responses that indicate the message was off-topic or blocked.
fn is_refusal_response(text: &str) -> bool {
    let lower = text.to_lowercase();
    const REFUSAL_PATTERNS: &[&str] = &[
        "i cannot provide",
        "i can't provide",
        "i cannot help with",
        "i can't help with",
        "i'm not able to",
        "i am not able to",
        "outside my scope",
        "beyond my capabilities",
        "i don't have the ability",
        "not something i can help",
        "i'm unable to",
        "i am unable to",
        "i cannot assist with",
        "i can't assist with",
    ];
    REFUSAL_PATTERNS.iter().any(|p| lower.contains(p))
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn format_set_compact(set: &ExerciseSet) -> String {
    let mut parts = Vec::new();
    match set.measurement_type {
        MeasurementType::WeightReps => {
            if let Some(c) = set.count {
                parts.push(format!("{c} reps"));
            }
            parts.push(format!("{:.1}kg", set.value));
        }
        MeasurementType::TimeBased => parts.push(format!("{:.0}s", set.value)),
        MeasurementType::DistanceBased => parts.push(format!("{:.0}m", set.value)),
        MeasurementType::LevelBased => parts.push(format!("level {:.0}", set.value)),
        MeasurementType::ScoreBased => parts.push(format!("score {:.1}", set.value)),
    }
    if parts.is_empty() { "no details".to_string() } else { parts.join(", ") }
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
            Self { response: std::sync::Mutex::new(response.to_string()) }
        }

        fn set_response(&self, response: &str) {
            *self.response.lock().unwrap() = response.to_string();
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockLlm {
        async fn complete(&self, _request: LlmRequest) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse { content: self.response.lock().unwrap().clone() })
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
            chat: crate::telegram::Chat { id: user_id, chat_type: "private".to_string() },
            date: 0,
            text: Some(text.to_string()),
            voice: None,
            audio: None,
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
            voice: None,
        }
    }

    async fn setup_handler(response: &str) -> (AssistantHandler, Arc<MockLlm>) {
        let db = Database::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db));
        let llm = Arc::new(MockLlm::new(response));
        let handler = AssistantHandler::new(db, Box::new(MockLlmWrapper(llm.clone())), test_config()).await.unwrap();
        (handler, llm)
    }

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
        assert!(reply.text.contains("Welcome"));
    }

    #[tokio::test]
    async fn existing_user_gets_llm_response() {
        let (handler, _) = setup_handler(r#"{"message": "Got it!", "actions": []}"#).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "how are you").await.unwrap();
        assert_eq!(reply.text, "Got it!");
    }

    #[tokio::test]
    async fn exercise_logging_creates_records() {
        let response = r#"{"message": "Logged your bench press!", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "sets": 3, "reps": 8, "weight_kg": 80.0, "perceived_difficulty": "medium"}
        ]}"#;
        let (handler, _) = setup_handler(response).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "3 sets bench 80kg 8 reps").await.unwrap();
        assert_eq!(reply.text, "Logged your bench press!");

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let session = db.get_active_session(user.id).unwrap().unwrap();
        let entries = db.list_entries_for_session(session.id).unwrap();
        assert_eq!(entries.len(), 1);
        let sets = db.list_sets_for_entry(entries[0].id).unwrap();
        assert_eq!(sets.len(), 3);
        assert!(sets.iter().all(|s| s.count == Some(8) && (s.value - 80.0).abs() < 1e-6));
    }

    #[tokio::test]
    async fn session_auto_start() {
        let response =
            r#"{"message": "Logged!", "actions": [{"type": "log_exercise", "exercise": "Bench Press", "sets": 1, "reps": 1}]}"#;
        let (handler, _) = setup_handler(response).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        assert!(db.get_active_session(user.id).unwrap().is_none());
        drop(db);

        let _ = handler.handle_text_message(&msg, "bench press").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        assert!(db.get_active_session(user.id).unwrap().is_some());
    }

    #[tokio::test]
    async fn slash_help_command() {
        let (handler, _) = setup_handler(r#"{"message": "x", "actions": []}"#).await;
        let msg = make_message(12345, "/help");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "/help").await.unwrap();
        assert!(reply.text.contains("Available commands"));
    }

    #[tokio::test]
    async fn slash_start_existing_user() {
        let (handler, _) = setup_handler(r#"{"message": "x", "actions": []}"#).await;
        let msg = make_message(12345, "/start");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "/start").await.unwrap();
        assert!(reply.text.contains("already registered"));
    }

    #[tokio::test]
    async fn multiple_actions_execute() {
        let response = r#"{"message": "Started session and logged exercise!", "actions": [
            {"type": "start_session", "notes": "Chest day"},
            {"type": "log_exercise", "exercise": "Bench Press", "sets": 3, "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, _) = setup_handler(response).await;
        let msg = make_message(12345, "hello");

        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "start chest day, 3x8 bench 80kg").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let session = db.get_active_session(user.id).unwrap().unwrap();
        assert_eq!(session.notes.as_deref(), Some("Chest day"));
        let entries = db.list_entries_for_session(session.id).unwrap();
        assert_eq!(entries.len(), 1);
        let sets = db.list_sets_for_entry(entries[0].id).unwrap();
        assert_eq!(sets.len(), 3);
    }

    #[tokio::test]
    async fn partial_action_failure_appends_note() {
        let response = r#"{"message": "Tried to log both!", "actions": [
            {"type": "start_session"},
            {"type": "log_exercise", "exercise": "Nonexistent Exercise 99", "sets": 3, "reps": 8}
        ]}"#;
        let (handler, _) = setup_handler(response).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "do stuff").await.unwrap();
        assert!(reply.text.contains("some actions failed"));
    }

    #[tokio::test]
    async fn message_truncation() {
        let (handler, llm) = setup_handler(r#"{"message": "ok", "actions": []}"#).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();

        let long_text = "a".repeat(3000);
        llm.set_response(r#"{"message": "received", "actions": []}"#);
        let reply = handler.handle_text_message(&msg, &long_text).await.unwrap();
        assert_eq!(reply.text, "received");

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let msgs = db.get_recent_messages_for_platform(user.id, "telegram", 10).unwrap();
        let user_msgs: Vec<_> = msgs.iter().filter(|m| m.role == ConversationRole::User).collect();
        let last_user_msg = user_msgs.last().unwrap();
        assert_eq!(last_user_msg.content.len(), 2000);
    }

    #[tokio::test]
    async fn slash_clear_excludes_prior_messages() {
        let (handler, _) = setup_handler(r#"{"message": "Got it!", "actions": []}"#).await;
        let msg = make_message(12345, "hello");

        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "how are you").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let msgs = db.get_recent_messages_for_platform(user.id, "telegram", 100).unwrap();
        assert_eq!(msgs.len(), 2);
        drop(db);

        let reply = handler.handle_text_message(&msg, "/clear").await.unwrap();
        assert!(reply.text.contains("cleared"));

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let msgs = db.get_recent_messages_for_platform(user.id, "telegram", 100).unwrap();
        assert_eq!(msgs.len(), 0);
    }

    #[tokio::test]
    async fn slash_clear_in_help_text() {
        let (handler, _) = setup_handler(r#"{"message": "x", "actions": []}"#).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "/help").await.unwrap();
        assert!(reply.text.contains("/clear"));
    }

    struct FailingMockLlm;

    #[async_trait::async_trait]
    impl LlmProvider for FailingMockLlm {
        async fn complete(&self, _request: LlmRequest) -> anyhow::Result<LlmResponse> {
            anyhow::bail!("Service temporarily unavailable")
        }
    }

    async fn setup_failing_handler() -> AssistantHandler {
        let db = Database::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db));
        let llm: Box<dyn LlmProvider> = Box::new(FailingMockLlm);
        AssistantHandler::new(db, llm, test_config()).await.unwrap()
    }

    #[tokio::test]
    async fn llm_error_stores_excluded_conversation() {
        let handler = setup_failing_handler().await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();

        let reply = handler.handle_text_message(&msg, "some bad request").await.unwrap();
        assert!(reply.text.contains("trouble processing"));

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let context_msgs = db.get_recent_messages_for_platform(user.id, "telegram", 100).unwrap();
        assert_eq!(context_msgs.len(), 0);

        let all_count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM conversation_history WHERE user_id = ?1", rusqlite::params![user.id], |row| row.get(0))
            .unwrap();
        assert_eq!(all_count, 2);
    }

    #[tokio::test]
    async fn refusal_response_excluded_from_context() {
        let (handler, _) = setup_handler(r#"{"message": "I cannot provide advice on that topic.", "actions": []}"#).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "off topic stuff").await.unwrap();
        assert!(reply.text.contains("I cannot provide"));

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let context_msgs = db.get_recent_messages_for_platform(user.id, "telegram", 100).unwrap();
        assert_eq!(context_msgs.len(), 0);
    }

    #[test]
    fn refusal_detection() {
        assert!(is_refusal_response("I cannot provide advice on that topic."));
        assert!(is_refusal_response("I can't help with that request."));
        assert!(is_refusal_response("That's outside my scope as a gym assistant."));
        assert!(is_refusal_response("I'm unable to assist with cooking recipes."));
        assert!(!is_refusal_response("Great job! I logged your bench press."));
        assert!(!is_refusal_response("Your session has been started."));
        assert!(!is_refusal_response("Here's your workout history."));
    }
}
