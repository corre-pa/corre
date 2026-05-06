use std::sync::Arc;

use anyhow::Context as _;
use chrono::Utc;
use corre_core::app::{LlmMessage, LlmProvider, LlmRequest, LlmRole};
use tokio::sync::Mutex;

use crate::config::GymConfig;
use crate::db::{
    ConversationRole, Database, Difficulty, ExerciseEntry, ExerciseSet, ExerciseTypeWithAncestry, MeasurementType, User,
    new_conversation_message, new_exercise_entry_at, new_exercise_goal, new_exercise_set, new_health_entry, new_user,
};
use crate::telegram::Message as TgMessage;

use super::actions::AssistantAction;
use super::matching::find_exercise_type;
use super::parser::parse_assistant_response;
use super::prompts::{ActivePlanView, EntryView, PlanExerciseView, PromptContext, build_system_prompt};

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
        let mut suffixes: Vec<String> = Vec::new();
        for action in &parsed.actions {
            match self.execute_action(action, user).await {
                Ok(Some(suffix)) => suffixes.push(suffix),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("Action execution failed: {e:#}");
                    failures.push(format!("{e:#}"));
                }
            }
        }

        let mut reply = parsed.message.clone();
        for s in &suffixes {
            reply.push_str("\n\n");
            reply.push_str(s);
        }
        if !failures.is_empty() {
            reply.push_str(&format!("\n\n(Note: some actions failed: {})", failures.join("; ")));
        }

        if is_refusal {
            self.store_excluded_conversation_on_platform(user.id, platform, text, &llm_response).await?;
        } else {
            self.store_conversation_on_platform(user.id, platform, text, &llm_response).await?;
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

                let mut completed: Vec<(String, Vec<ExerciseSet>)> = Vec::new();
                let mut open: Vec<(String, Vec<ExerciseSet>)> = Vec::new();
                for entry in &entries {
                    let sets = db.list_sets_for_entry(entry.id)?;
                    let name = sets
                        .first()
                        .and_then(|s| self.catalogue.iter().find(|e| e.exercise_type.id == s.exercise_type_id))
                        .map(|e| e.exercise_type.name.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    if entry.end_timestamp.is_some() {
                        completed.push((name, sets));
                    } else {
                        open.push((name, sets));
                    }
                }

                if completed.is_empty() && open.is_empty() {
                    result.push_str("No exercises logged yet\n");
                }

                if !completed.is_empty() {
                    result.push_str("<b>Completed:</b>\n");
                    for (name, sets) in &completed {
                        result.push_str(&format!(
                            "- <b>{}</b> ({} {}) — {}\n",
                            escape_html(name),
                            sets.len(),
                            if sets.len() == 1 { "set" } else { "sets" },
                            escape_html(&sets.iter().map(format_set_short).collect::<Vec<_>>().join(", ")),
                        ));
                    }
                }

                if open.len() > 1 {
                    result.push_str("<b>Superset (in progress):</b>\n");
                    for (i, (name, sets)) in open.iter().enumerate() {
                        result.push_str(&format!(
                            "  {}. <b>{}</b> ({} {}) — {}\n",
                            i + 1,
                            escape_html(name),
                            sets.len(),
                            if sets.len() == 1 { "set" } else { "sets" },
                            escape_html(&sets.iter().map(format_set_short).collect::<Vec<_>>().join(", ")),
                        ));
                    }
                } else if let Some((name, sets)) = open.first() {
                    result.push_str("<b>Current exercise:</b>\n");
                    result.push_str(&format!(
                        "- <b>{}</b> ({} {}) — {}\n",
                        escape_html(name),
                        sets.len(),
                        if sets.len() == 1 { "set" } else { "sets" },
                        escape_html(&sets.iter().map(format_set_short).collect::<Vec<_>>().join(", ")),
                    ));
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
        let mut session_sets: Vec<(ExerciseSet, String)> = Vec::new();
        let mut session_entries: Vec<EntryView> = Vec::new();
        let mut closed_exercise_ids_in_session: Vec<i64> = Vec::new();

        if let Some(session) = &active_session {
            let entries = db.list_entries_for_session(session.id)?;
            for entry in &entries {
                let sets = db.list_sets_for_entry(entry.id)?;
                let exercise_type_id = sets.first().map(|s| s.exercise_type_id);
                let exercise_name = exercise_type_id
                    .and_then(|id| self.catalogue.iter().find(|e| e.exercise_type.id == id).map(|e| e.exercise_type.name.clone()))
                    .unwrap_or_else(|| "unknown".to_string());
                let summary_parts: Vec<String> = sets.iter().map(format_set_short).collect();
                session_entries.push(EntryView {
                    id: entry.id,
                    exercise_name: exercise_name.clone(),
                    set_count: sets.len(),
                    sets_summary: summary_parts.join(", "),
                    is_open: entry.end_timestamp.is_none(),
                });
                if entry.end_timestamp.is_some() {
                    if let Some(id) = exercise_type_id {
                        closed_exercise_ids_in_session.push(id);
                    }
                }
                for set in sets {
                    session_sets.push((set, exercise_name.clone()));
                }
            }
        }

        // Active plan, recovered from sentinel-prefixed session.notes
        let active_plan = match active_session.as_ref().and_then(|s| s.notes.as_deref()) {
            Some(notes) => {
                let (plan_name, _rest) = parse_plan_from_notes(Some(notes));
                match plan_name {
                    Some(name) => self.build_active_plan(&db, user.id, &name, &closed_exercise_ids_in_session)?,
                    None => None,
                }
            }
            None => None,
        };

        // Leaked open entries: open EEs not belonging to the active session, OR open
        // entries in the active session (so the LLM can decide to close/delete before
        // a new session is requested).
        let leaked_open_entries = build_leaked_view(&db, &self.catalogue, user.id, active_session.as_ref().map(|s| s.id))?;

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
            session_entries,
            leaked_open_entries,
            active_plan,
            health_entries,
            recent_summaries,
            recent_sets,
            exercise_types: self.catalogue.clone(),
            active_goals,
            schedules,
        };

        Ok(build_system_prompt(&ctx))
    }

    fn build_active_plan(
        &self,
        db: &Database,
        user_id: i64,
        plan_name: &str,
        completed_exercise_ids: &[i64],
    ) -> anyhow::Result<Option<ActivePlanView>> {
        let schedules = db.list_schedules(user_id)?;
        let Some(schedule) = schedules.into_iter().find(|s| s.name.eq_ignore_ascii_case(plan_name)) else {
            return Ok(None);
        };
        let mut planned = db.list_schedule_exercises(schedule.id)?;
        planned.sort_by_key(|p| p.order_idx);
        let next = planned.iter().find(|p| !completed_exercise_ids.contains(&p.exercise_type_id)).map(|p| {
            let exercise_name = self
                .catalogue
                .iter()
                .find(|e| e.exercise_type.id == p.exercise_type_id)
                .map(|e| e.exercise_type.name.clone())
                .unwrap_or_else(|| "unknown".to_string());
            PlanExerciseView { exercise_name, target_sets: p.target_sets, target_reps: p.target_reps, target_weight_kg: p.target_weight_kg }
        });
        Ok(Some(ActivePlanView { name: schedule.name, completed_exercise_ids: completed_exercise_ids.to_vec(), next }))
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

        let request = LlmRequest { messages, temperature: Some(0.1), max_completion_tokens: Some(1024), json_mode: true };

        let response = self.llm.complete(request).await.context("LLM completion failed")?;
        Ok(response.content)
    }

    /// Returns an optional suffix appended to the assistant's reply (set-count
    /// checkpoint, premature-close pushback, leaked-entry warning).
    async fn execute_action(&self, action: &AssistantAction, user: &User) -> anyhow::Result<Option<String>> {
        tracing::debug!(action = ?action, user_id = user.id, "Executing action");
        match action {
            AssistantAction::LogExercise { exercise, reps, weight_kg, perceived_difficulty, comment, .. } => {
                let et = find_exercise_type(&self.catalogue, exercise).ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let session = self.ensure_session(user).await?;
                let entry_id = self.ensure_entry_for_exercise(user.id, session.id, et.exercise_type.id).await?;
                let weight = weight_kg.unwrap_or(0.0);
                let pd = perceived_difficulty.unwrap_or(Difficulty::Medium);
                {
                    let db = self.db.lock().await;
                    let existing = db.list_sets_for_entry(entry_id)?.len() as i32;
                    let mut s = new_exercise_set(entry_id, et.exercise_type.id, MeasurementType::WeightReps, weight);
                    s.count = *reps;
                    s.order_idx = existing;
                    s.perceived_difficulty = pd;
                    s.comment = comment.clone();
                    db.insert_set(&s)?;
                }
                Ok(self.set_count_checkpoint_suffix(entry_id, &et.exercise_type.name).await?)
            }
            AssistantAction::LogExerciseTimed { exercise, duration_secs, perceived_difficulty, comment } => {
                let et = find_exercise_type(&self.catalogue, exercise).ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let session = self.ensure_session(user).await?;
                let entry_id = self.ensure_entry_for_exercise(user.id, session.id, et.exercise_type.id).await?;
                let mut s = new_exercise_set(entry_id, et.exercise_type.id, MeasurementType::TimeBased, *duration_secs as f64);
                s.perceived_difficulty = perceived_difficulty.unwrap_or(Difficulty::Medium);
                s.comment = comment.clone();
                self.db.lock().await.insert_set(&s)?;
                Ok(self.set_count_checkpoint_suffix(entry_id, &et.exercise_type.name).await?)
            }
            AssistantAction::LogExerciseDistance { exercise, distance_m, duration_secs, perceived_difficulty, comment } => {
                let et = find_exercise_type(&self.catalogue, exercise).ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let session = self.ensure_session(user).await?;
                let entry_id = self.ensure_entry_for_exercise(user.id, session.id, et.exercise_type.id).await?;
                let value = distance_m.unwrap_or_else(|| duration_secs.unwrap_or(0) as f64);
                let mt = if distance_m.is_some() { MeasurementType::DistanceBased } else { MeasurementType::TimeBased };
                let mut s = new_exercise_set(entry_id, et.exercise_type.id, mt, value);
                s.perceived_difficulty = perceived_difficulty.unwrap_or(Difficulty::Medium);
                s.comment = comment.clone();
                self.db.lock().await.insert_set(&s)?;
                Ok(self.set_count_checkpoint_suffix(entry_id, &et.exercise_type.name).await?)
            }
            AssistantAction::StartSession { notes, plan } => {
                let db = self.db.lock().await;
                if let Some(active) = db.get_active_session(user.id)? {
                    let open = db.list_open_entries_for_session(active.id)?;
                    if !open.is_empty() {
                        let names = self.entry_exercise_names(&db, &open)?;
                        let suffix = format!(
                            "You still have {n} open exercise {entries} in your active session ({list}). \
                             Want me to close them or delete them before starting a new session?",
                            n = open.len(),
                            entries = if open.len() == 1 { "entry" } else { "entries" },
                            list = names.join(", "),
                        );
                        return Ok(Some(suffix));
                    }
                    tracing::debug!("Session already active, skipping start");
                    return Ok(None);
                }
                // No active session — clean up any leaked open entries from previously
                // ended sessions before starting fresh.
                drop(db);
                self.silent_close_leaked_entries(user.id).await?;
                let db = self.db.lock().await;
                let combined_notes = combine_plan_with_notes(plan.as_deref(), notes.as_deref());
                let session = db.start_session(user.id, combined_notes.as_deref())?;
                tracing::debug!(id = session.id, plan = ?plan, "Started session");
                Ok(None)
            }
            AssistantAction::EndSession => {
                let db = self.db.lock().await;
                if let Some(session) = db.get_active_session(user.id)? {
                    tracing::debug!(id = session.id, "Ending session");
                    db.end_session(session.id)?;
                } else {
                    tracing::debug!("No active session to end");
                }
                Ok(None)
            }
            AssistantAction::CloseExerciseEntry { exercise, entry_id } => {
                self.close_exercise_entry_action(user, exercise.as_deref(), *entry_id, false).await
            }
            AssistantAction::ConfirmCloseExerciseEntry { exercise, entry_id } => {
                self.close_exercise_entry_action(user, exercise.as_deref(), *entry_id, true).await
            }
            AssistantAction::DeleteExerciseEntry { entry_id } => {
                let db = self.db.lock().await;
                let entry = db.get_entry(*entry_id)?.ok_or_else(|| anyhow::anyhow!("entry {entry_id} not found"))?;
                anyhow::ensure!(entry.user_id == user.id, "entry {entry_id} does not belong to user");
                db.delete_entry(*entry_id)?;
                Ok(None)
            }
            AssistantAction::CloseAllOpenEntries => {
                let db = self.db.lock().await;
                if let Some(session) = db.get_active_session(user.id)? {
                    let n = db.close_open_entries_for_session(session.id, None)?;
                    tracing::debug!(session_id = session.id, closed = n, "Closed all open entries");
                }
                Ok(None)
            }
            AssistantAction::LogHealth { entry_type, body_part, severity, description } => {
                let mut entry = new_health_entry(user.id, *entry_type, description);
                entry.body_part = body_part.clone();
                if let Some(sev) = severity {
                    entry.severity = sev.clone();
                }
                tracing::debug!(entry_type = ?entry_type, body_part = ?body_part, severity = ?severity, "Inserting health entry");
                self.db.lock().await.insert_health_entry(&entry)?;
                Ok(None)
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
                Ok(None)
            }
            AssistantAction::SetGoal { exercise, target_value, end_date } => {
                let et = find_exercise_type(&self.catalogue, exercise).ok_or_else(|| anyhow::anyhow!("Unknown exercise: {exercise}"))?;
                let mut goal = new_exercise_goal(user.id, et.exercise_type.id, *target_value);
                goal.end_date = end_date.clone();
                tracing::debug!(exercise = %et.exercise_type.name, target = %target_value, end_date = ?end_date, "Inserting goal");
                self.db.lock().await.insert_goal(&goal)?;
                Ok(None)
            }
            AssistantAction::Unknown => {
                tracing::debug!("Ignoring unknown action type from LLM");
                Ok(None)
            }
        }
    }

    async fn ensure_session(&self, user: &User) -> anyhow::Result<crate::db::Session> {
        if let Some(session) = self.db.lock().await.get_active_session(user.id)? {
            return Ok(session);
        }
        // Auto-start path: no active session exists, so any open exercise_entries
        // for this user are leaks from a previously-ended session. Close them
        // silently before starting fresh.
        self.silent_close_leaked_entries(user.id).await?;
        self.db.lock().await.start_session(user.id, None)
    }

    /// Open exercise_entry whose sets match `exercise_type_id`, or insert a new
    /// entry. The new entry's `start_timestamp` is computed once and reused as the
    /// caller's reference, so the first set inserted into it can be given the same
    /// `logged_at` value (matching the brief's "same start timestamp as the first
    /// set" requirement).
    async fn ensure_entry_for_exercise(&self, user_id: i64, session_id: i64, exercise_type_id: i64) -> anyhow::Result<i64> {
        let db = self.db.lock().await;
        if let Some(open) = db.find_open_entry_for_exercise(user_id, session_id, exercise_type_id)? {
            return Ok(open.id);
        }
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let entry = new_exercise_entry_at(user_id, Some(session_id), None, &now);
        db.insert_entry(&entry)
    }

    /// Resolve an entry to close (explicit id > exercise-name match > most recent
    /// open in the active session). When `confirm` is false and the resolved entry
    /// has fewer than 3 sets, returns the pushback suffix and leaves the entry
    /// open.
    async fn close_exercise_entry_action(
        &self,
        user: &User,
        exercise: Option<&str>,
        entry_id: Option<i64>,
        confirm: bool,
    ) -> anyhow::Result<Option<String>> {
        let db = self.db.lock().await;
        let active = db.get_active_session(user.id)?;
        let resolved = if let Some(id) = entry_id {
            let entry = db.get_entry(id)?.ok_or_else(|| anyhow::anyhow!("entry {id} not found"))?;
            anyhow::ensure!(entry.user_id == user.id, "entry {id} does not belong to user");
            anyhow::ensure!(entry.end_timestamp.is_none(), "entry {id} is already closed");
            entry
        } else {
            let session = active.as_ref().ok_or_else(|| anyhow::anyhow!("no active session"))?;
            let open = db.list_open_entries_for_session(session.id)?;
            let entry = if let Some(name) = exercise {
                let et = find_exercise_type(&self.catalogue, name).ok_or_else(|| anyhow::anyhow!("Unknown exercise: {name}"))?;
                open.into_iter().find(|e| {
                    db.list_sets_for_entry(e.id).map(|sets| sets.iter().any(|s| s.exercise_type_id == et.exercise_type.id)).unwrap_or(false)
                })
            } else {
                open.into_iter().last()
            };
            entry.ok_or_else(|| anyhow::anyhow!("no matching open exercise_entry to close"))?
        };

        let count = db.count_sets_for_entry(resolved.id)?;
        if !confirm && count < 3 {
            let name = entry_exercise_name(&db, &self.catalogue, resolved.id)?;
            let suffix = format!(
                "You've only done {count} {sets} of {name}. You should really push for one more! Should we keep going?",
                sets = if count == 1 { "set" } else { "sets" },
            );
            return Ok(Some(suffix));
        }
        db.end_entry(resolved.id)?;
        Ok(None)
    }

    /// Set-count checkpoint: every time the user logs a set in an open entry that
    /// already has ≥3 sets total, ask whether they want to keep going or move on.
    async fn set_count_checkpoint_suffix(&self, entry_id: i64, exercise_name: &str) -> anyhow::Result<Option<String>> {
        let count = self.db.lock().await.count_sets_for_entry(entry_id)?;
        if count >= 3 {
            Ok(Some(format!("You've logged {count} sets of {exercise_name}. Want another set, or move to the next exercise?")))
        } else {
            Ok(None)
        }
    }

    /// Close any open exercise_entries that belong to already-ended sessions for
    /// this user. Best-effort: uses the parent session's `ended_at` when present,
    /// otherwise `datetime('now')`.
    async fn silent_close_leaked_entries(&self, user_id: i64) -> anyhow::Result<()> {
        let db = self.db.lock().await;
        let leaks = db.list_open_entries_for_user(user_id)?;
        for entry in leaks {
            let Some(session_id) = entry.session_id else {
                db.end_entry(entry.id)?;
                continue;
            };
            let session = db.get_session(session_id)?;
            match session.and_then(|s| s.ended_at) {
                Some(ended_at) => {
                    db.conn().execute(
                        "UPDATE exercise_entry SET end_timestamp = ?1 WHERE id = ?2 AND end_timestamp IS NULL",
                        rusqlite::params![ended_at, entry.id],
                    )?;
                }
                None => {
                    // session is still active — not a leak; leave it alone.
                }
            }
        }
        Ok(())
    }

    fn entry_exercise_names(&self, db: &Database, entries: &[ExerciseEntry]) -> anyhow::Result<Vec<String>> {
        entries.iter().map(|e| entry_exercise_name(db, &self.catalogue, e.id)).collect()
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

/// Resolve the exercise an entry "belongs to" via its first set. Returns
/// `"unknown"` if the entry has no sets yet (which only happens transiently
/// before the first insert).
pub(crate) fn entry_exercise_name(db: &Database, catalogue: &[ExerciseTypeWithAncestry], entry_id: i64) -> anyhow::Result<String> {
    let sets = db.list_sets_for_entry(entry_id)?;
    let Some(first) = sets.first() else {
        return Ok("unknown".to_string());
    };
    let name = catalogue
        .iter()
        .find(|e| e.exercise_type.id == first.exercise_type_id)
        .map(|e| e.exercise_type.name.clone())
        .unwrap_or_else(|| "unknown".to_string());
    Ok(name)
}

/// Encode an optional plan name into the session's `notes` field using the
/// `plan:<name>` sentinel prefix so the active plan can be recovered later
/// without a schema change.
pub(crate) fn combine_plan_with_notes(plan: Option<&str>, notes: Option<&str>) -> Option<String> {
    match (plan, notes) {
        (Some(p), Some(n)) => Some(format!("plan:{p}\n{n}")),
        (Some(p), None) => Some(format!("plan:{p}")),
        (None, Some(n)) => Some(n.to_string()),
        (None, None) => None,
    }
}

/// Inverse of `combine_plan_with_notes`. Returns `(plan_name, remaining_notes)`.
pub(crate) fn parse_plan_from_notes(notes: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(text) = notes else {
        return (None, None);
    };
    if let Some(rest) = text.strip_prefix("plan:") {
        match rest.split_once('\n') {
            Some((plan, body)) => (Some(plan.trim().to_string()), Some(body.to_string())),
            None => (Some(rest.trim().to_string()), None),
        }
    } else {
        (None, Some(text.to_string()))
    }
}

/// Compact rendering of a single set used inside an entry summary, e.g. "8×80kg",
/// "30s", "5000m".
pub(crate) fn format_set_short(set: &ExerciseSet) -> String {
    match set.measurement_type {
        MeasurementType::WeightReps => match set.count {
            Some(c) => format!("{c}×{:.1}kg", set.value),
            None => format!("{:.1}kg", set.value),
        },
        MeasurementType::TimeBased => format!("{:.0}s", set.value),
        MeasurementType::DistanceBased => format!("{:.0}m", set.value),
        MeasurementType::LevelBased => format!("L{:.0}", set.value),
        MeasurementType::ScoreBased => format!("{:.1}pt", set.value),
    }
}

/// Build EntryView rows for any open exercise_entries the user has, so the prompt
/// (and LLM-driven cleanup logic) can see them. When `active_session_id` is given,
/// only entries inside that session are reported (the caller's contract: leaks are
/// what blocks a *new* session, not what's normal in the current one).
fn build_leaked_view(
    db: &Database,
    catalogue: &[ExerciseTypeWithAncestry],
    user_id: i64,
    active_session_id: Option<i64>,
) -> anyhow::Result<Vec<EntryView>> {
    let all_open = db.list_open_entries_for_user(user_id)?;
    let filtered: Vec<_> = all_open
        .into_iter()
        .filter(|e| match active_session_id {
            Some(sid) => e.session_id == Some(sid),
            None => true,
        })
        .collect();
    let mut views = Vec::with_capacity(filtered.len());
    for entry in filtered {
        let sets = db.list_sets_for_entry(entry.id)?;
        let exercise_name = sets
            .first()
            .and_then(|s| catalogue.iter().find(|e| e.exercise_type.id == s.exercise_type_id))
            .map(|e| e.exercise_type.name.clone())
            .unwrap_or_else(|| "unknown".to_string());
        views.push(EntryView {
            id: entry.id,
            exercise_name,
            set_count: sets.len(),
            sets_summary: sets.iter().map(format_set_short).collect::<Vec<_>>().join(", "),
            is_open: true,
        });
    }
    Ok(views)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use corre_core::app::{LlmRequest, LlmResponse};

    struct MockLlm {
        response: std::sync::Mutex<String>,
        recorded: std::sync::Mutex<Vec<LlmRequest>>,
    }

    impl MockLlm {
        fn new(response: &str) -> Self {
            Self { response: std::sync::Mutex::new(response.to_string()), recorded: std::sync::Mutex::new(Vec::new()) }
        }

        fn set_response(&self, response: &str) {
            *self.response.lock().unwrap() = response.to_string();
        }

        fn recorded_requests(&self) -> Vec<LlmRequest> {
            self.recorded.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockLlm {
        async fn complete(&self, request: LlmRequest) -> anyhow::Result<LlmResponse> {
            self.recorded.lock().unwrap().push(request);
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
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0, "perceived_difficulty": "medium"},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0, "perceived_difficulty": "medium"},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0, "perceived_difficulty": "medium"}
        ]}"#;
        let (handler, _) = setup_handler(response).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let reply = handler.handle_text_message(&msg, "3 sets bench 80kg 8 reps").await.unwrap();
        assert!(reply.text.starts_with("Logged your bench press!"));
        assert!(reply.text.contains("You've logged 3 sets of Bench Press. Want another set, or move to the next exercise?"));

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let session = db.get_active_session(user.id).unwrap().unwrap();
        let entries = db.list_entries_for_session(session.id).unwrap();
        assert_eq!(entries.len(), 1);
        let sets = db.list_sets_for_entry(entries[0].id).unwrap();
        assert_eq!(sets.len(), 3);
        assert!(sets.iter().all(|s| s.count == Some(8) && (s.value - 80.0).abs() < 1e-6));
        assert!(entries[0].end_timestamp.is_none(), "entry should still be open");
    }

    #[tokio::test]
    async fn session_auto_start() {
        let response = r#"{"message": "Logged!", "actions": [{"type": "log_exercise", "exercise": "Bench Press", "sets": 1, "reps": 1}]}"#;
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
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0}
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

    #[tokio::test]
    async fn assistant_history_preserves_json_envelope() {
        // The LLM is contracted to emit `{"message": "...", "actions": [...]}`. If
        // we strip the envelope before persisting the assistant turn, the next
        // call shows the model plain prose in history and it abandons the JSON
        // contract. Pin the round-trip: turn-2's request must contain the prior
        // assistant turn as a parseable AssistantResponse with non-empty actions.
        let canned = r#"{"message":"Logged.","actions":[{"type":"log_exercise","exercise":"Bench Press","sets":1,"reps":8,"weight_kg":60.0,"perceived_difficulty":"easy"}]}"#;
        let (handler, llm) = setup_handler(canned).await;
        let msg = make_message(12345, "hello");

        // First call registers the user and returns the welcome reply without
        // hitting the LLM; subsequent calls are real LLM round-trips.
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "bench 60 8 reps easy").await.unwrap();
        let _ = handler.handle_text_message(&msg, "another set, 6 reps at 70 kg, hard").await.unwrap();

        let recorded = llm.recorded_requests();
        assert_eq!(recorded.len(), 2, "expected two LlmRequests after registration + two user turns");

        let second = &recorded[1];
        let assistant_turn = second
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, LlmRole::Assistant))
            .expect("turn-2 history must include the prior assistant turn");

        let parsed: crate::assistant::actions::AssistantResponse =
            serde_json::from_str(&assistant_turn.content).expect("assistant turn in history must round-trip as the JSON envelope");
        assert!(
            !parsed.actions.is_empty(),
            "expected the prior assistant turn to retain its actions array, got: {}",
            assistant_turn.content
        );
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

    // ─── New behaviour tests for the set-centric workflow ──────────────────

    #[tokio::test]
    async fn supersets_keep_separate_entries() {
        let response_a = r#"{"message": "Logged bench.", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "sets": 1, "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, llm) = setup_handler(response_a).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "bench 80kg 8 reps").await.unwrap();

        // Without closing, log a different exercise.
        llm.set_response(
            r#"{"message": "Logged pull-ups.", "actions": [
                {"type": "log_exercise", "exercise": "Pull-Up", "sets": 1, "reps": 10}
            ]}"#,
        );
        let _ = handler.handle_text_message(&msg, "now pull-ups, 10 reps").await.unwrap();

        // Then back to bench — should reuse the existing open Bench Press entry.
        llm.set_response(
            r#"{"message": "Logged another bench set.", "actions": [
                {"type": "log_exercise", "exercise": "Bench Press", "sets": 1, "reps": 8, "weight_kg": 80.0}
            ]}"#,
        );
        let _ = handler.handle_text_message(&msg, "another bench set").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let session = db.get_active_session(user.id).unwrap().unwrap();
        let entries = db.list_entries_for_session(session.id).unwrap();
        assert_eq!(entries.len(), 2, "two distinct entries for two exercises (superset)");
        let mut counts: Vec<usize> = entries.iter().map(|e| db.list_sets_for_entry(e.id).unwrap().len()).collect();
        counts.sort();
        assert_eq!(counts, vec![1, 2], "Pull-Up=1, Bench Press=2");
        for e in &entries {
            assert!(e.end_timestamp.is_none(), "both entries remain open");
        }
    }

    #[tokio::test]
    async fn checkpoint_suffix_appears_at_3_sets_and_repeats_at_4() {
        let log_three = r#"{"message": "Done!", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, llm) = setup_handler(log_three).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();

        let reply = handler.handle_text_message(&msg, "3 sets bench 80kg 8").await.unwrap();
        assert!(reply.text.contains("You've logged 3 sets of Bench Press. Want another set, or move to the next exercise?"));

        // 4th set — checkpoint should fire again with n=4.
        llm.set_response(
            r#"{"message": "Logged.", "actions": [
                {"type": "log_exercise", "exercise": "Bench Press", "sets": 1, "reps": 8, "weight_kg": 80.0}
            ]}"#,
        );
        let reply = handler.handle_text_message(&msg, "one more").await.unwrap();
        assert!(reply.text.contains("You've logged 4 sets of Bench Press"));
    }

    #[tokio::test]
    async fn premature_close_pushback_at_2_sets() {
        let response_log = r#"{"message": "Logged.", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, llm) = setup_handler(response_log).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "2 sets bench").await.unwrap();

        llm.set_response(
            r#"{"message": "Closing bench.", "actions": [
                {"type": "close_exercise_entry", "exercise": "Bench Press"}
            ]}"#,
        );
        let reply = handler.handle_text_message(&msg, "close bench").await.unwrap();
        assert!(reply.text.contains("You've only done 2 sets of Bench Press. You should really push for one more! Should we keep going?"));

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let session = db.get_active_session(user.id).unwrap().unwrap();
        let entries = db.list_entries_for_session(session.id).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].end_timestamp.is_none(), "entry must stay open after pushback");
    }

    #[tokio::test]
    async fn confirm_close_after_pushback_actually_closes() {
        let response_log = r#"{"message": "Logged.", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, llm) = setup_handler(response_log).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "2 sets bench").await.unwrap();

        llm.set_response(
            r#"{"message": "Closing for real.", "actions": [
                {"type": "confirm_close_exercise_entry", "exercise": "Bench Press"}
            ]}"#,
        );
        let _ = handler.handle_text_message(&msg, "yes really close it").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let session = db.get_active_session(user.id).unwrap().unwrap();
        let entries = db.list_entries_for_session(session.id).unwrap();
        assert!(entries[0].end_timestamp.is_some(), "confirm_close_exercise_entry bypasses pushback");
    }

    #[tokio::test]
    async fn close_exercise_entry_with_three_sets_succeeds() {
        let response_log = r#"{"message": "Logged.", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, llm) = setup_handler(response_log).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "3 sets bench").await.unwrap();

        llm.set_response(
            r#"{"message": "Closing bench.", "actions": [
                {"type": "close_exercise_entry", "exercise": "Bench Press"}
            ]}"#,
        );
        let _ = handler.handle_text_message(&msg, "move on").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let session = db.get_active_session(user.id).unwrap().unwrap();
        let entries = db.list_entries_for_session(session.id).unwrap();
        assert!(entries[0].end_timestamp.is_some(), "≥3-set close should succeed without pushback");
    }

    #[tokio::test]
    async fn end_session_closes_all_open_entries() {
        let log = r#"{"message": "Logged.", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "sets": 1, "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, llm) = setup_handler(log).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "bench 80kg 8 reps").await.unwrap();

        llm.set_response(r#"{"message": "Ending.", "actions": [{"type": "end_session"}]}"#);
        let _ = handler.handle_text_message(&msg, "end session").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        // No active session anymore.
        assert!(db.get_active_session(user.id).unwrap().is_none());
        // No open entries either.
        let leftover = db.list_open_entries_for_user(user.id).unwrap();
        assert!(leftover.is_empty(), "end_session must cascade-close every open entry");
    }

    #[tokio::test]
    async fn start_session_with_open_entries_in_active_session_blocks() {
        // Step 1: log a set so an open entry exists in the active session.
        let log = r#"{"message": "Logged.", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "sets": 1, "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, llm) = setup_handler(log).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "bench").await.unwrap();

        // Step 2: try to start a new session — should be blocked.
        llm.set_response(r#"{"message": "Starting.", "actions": [{"type": "start_session"}]}"#);
        let reply = handler.handle_text_message(&msg, "start a new session").await.unwrap();
        assert!(reply.text.contains("open exercise"));
        assert!(reply.text.contains("close them or delete them"));

        // The original session is still the active one — no new session was created.
        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let session_count: i64 =
            db.conn().query_row("SELECT COUNT(*) FROM sessions WHERE user_id = ?1", rusqlite::params![user.id], |r| r.get(0)).unwrap();
        assert_eq!(session_count, 1);
    }

    #[tokio::test]
    async fn close_all_open_entries_clears_block_for_new_session() {
        let log = r#"{"message": "Logged.", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "sets": 1, "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, llm) = setup_handler(log).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "bench").await.unwrap();

        llm.set_response(r#"{"message": "Closing.", "actions": [{"type": "close_all_open_entries"}]}"#);
        let _ = handler.handle_text_message(&msg, "close them").await.unwrap();

        let db = handler.db.lock().await;
        let user = db.get_user_by_telegram_id("12345").unwrap().unwrap();
        let leftover = db.list_open_entries_for_user(user.id).unwrap();
        assert!(leftover.is_empty());
    }

    #[tokio::test]
    async fn cmd_status_renders_superset_label_when_two_open() {
        let log_a = r#"{"message": "ok", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "sets": 1, "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, llm) = setup_handler(log_a).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "bench").await.unwrap();
        llm.set_response(
            r#"{"message": "ok", "actions": [
                {"type": "log_exercise", "exercise": "Pull-Up", "sets": 1, "reps": 10}
            ]}"#,
        );
        let _ = handler.handle_text_message(&msg, "pull-ups").await.unwrap();

        let reply = handler.handle_text_message(&msg, "/status").await.unwrap();
        assert!(reply.text.contains("Superset (in progress)"));
        assert!(reply.text.contains("Bench Press"));
        assert!(reply.text.contains("Pull-Up"));
    }

    #[tokio::test]
    async fn cmd_status_renders_completed_section() {
        let log = r#"{"message": "ok", "actions": [
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0},
            {"type": "log_exercise", "exercise": "Bench Press", "reps": 8, "weight_kg": 80.0}
        ]}"#;
        let (handler, llm) = setup_handler(log).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let _ = handler.handle_text_message(&msg, "3 sets bench").await.unwrap();

        llm.set_response(r#"{"message": "ok", "actions": [{"type": "close_exercise_entry", "exercise": "Bench Press"}]}"#);
        let _ = handler.handle_text_message(&msg, "done").await.unwrap();

        let reply = handler.handle_text_message(&msg, "/status").await.unwrap();
        assert!(reply.text.contains("Completed:"));
        assert!(reply.text.contains("Bench Press"));
    }

    #[tokio::test]
    async fn start_session_with_plan_stores_sentinel_in_notes() {
        // Seed a schedule named "Push Day" for the user.
        let (handler, llm) = setup_handler(r#"{"message": "hi", "actions": []}"#).await;
        let msg = make_message(12345, "hello");
        let _ = handler.handle_text_message(&msg, "hello").await.unwrap();
        let user_id = {
            let db = handler.db.lock().await;
            let u = db.get_user_by_telegram_id("12345").unwrap().unwrap();
            let bp = db.get_exercise_type_by_name("Bench Press").unwrap().unwrap();
            let pull = db.get_exercise_type_by_name("Pull-Up").unwrap().unwrap();
            let sched_id = db
                .insert_schedule(&crate::db::Schedule {
                    id: 0,
                    user_id: u.id,
                    name: "Push Day".to_string(),
                    cron_expr: "0 0 6 * * 1".to_string(),
                    reminder_type: crate::db::ReminderType::Text,
                    reminder_notice_mins: 30,
                    enabled: true,
                    created_at: String::new(),
                    updated_at: String::new(),
                })
                .unwrap();
            db.add_schedule_exercise(&crate::db::ScheduleExercise {
                schedule_id: sched_id,
                exercise_type_id: bp.id,
                order_idx: 0,
                target_sets: Some(3),
                target_reps: Some(8),
                target_weight_kg: Some(80.0),
            })
            .unwrap();
            db.add_schedule_exercise(&crate::db::ScheduleExercise {
                schedule_id: sched_id,
                exercise_type_id: pull.id,
                order_idx: 1,
                target_sets: Some(3),
                target_reps: Some(10),
                target_weight_kg: None,
            })
            .unwrap();
            u.id
        };

        llm.set_response(r#"{"message": "Starting.", "actions": [{"type": "start_session", "plan": "Push Day"}]}"#);
        let _ = handler.handle_text_message(&msg, "start push day").await.unwrap();

        let db = handler.db.lock().await;
        let session = db.get_active_session(user_id).unwrap().unwrap();
        let notes = session.notes.unwrap();
        assert!(notes.starts_with("plan:Push Day"));
    }
}
