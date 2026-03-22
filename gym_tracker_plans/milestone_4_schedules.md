# Milestone 4: Schedules and Reminders

## Goal

Users define workout schedules via conversational chat. The bot sends proactive reminders
before scheduled workouts and escalates if ignored. Supports programme creation based on
user goals and history.

## Prerequisites

- Milestone 1 complete (Telegram text chat with LLM assistant)
- Milestone 0 complete (schedules + schedule_exercises tables exist)

## File structure

```
crates/corre-gym/src/
    scheduler/
      mod.rs              Re-exports
      reminders.rs        Reminder job management and escalation logic
      programme.rs        LLM-powered programme suggestion based on goals
    assistant/
      actions.rs          Add: CreateSchedule, ModifySchedule, ListSchedule actions
      prompts.rs          Add schedule context to system prompt
```

## Internal scheduler

Use `tokio-cron-scheduler` (already in workspace deps) inside the daemon. This runs
independently of Corre's host scheduler since the gym tracker is a standalone binary.

### Startup

On daemon startup:
1. Load all enabled schedules from the database
2. For each schedule, calculate the next fire time in the user's timezone
3. Register a tokio-cron job that fires the reminder

### Dynamic schedule management

When a user creates/modifies/deletes a schedule via chat:
1. Execute the DB mutation
2. Remove the old cron job (if modifying/deleting)
3. Add the new cron job (if creating/modifying)

The scheduler needs a `SchedulerHandle` that can add/remove jobs at runtime:

```rust
pub struct ReminderScheduler {
    scheduler: JobScheduler,
    job_ids: HashMap<String, Uuid>,  // schedule_id -> job_id
    db: Arc<Mutex<Database>>,
    telegram: Arc<TelegramClient>,
    llm: Arc<LlmProvider>,
}

impl ReminderScheduler {
    pub async fn new(db, telegram, llm) -> Result<Self>;

    /// Load all enabled schedules and register jobs.
    pub async fn load_all(&mut self) -> Result<()>;

    /// Add a reminder job for a specific schedule.
    pub async fn add_schedule(&mut self, schedule_id: &str) -> Result<()>;

    /// Remove a reminder job.
    pub async fn remove_schedule(&mut self, schedule_id: &str) -> Result<()>;

    /// Reload a schedule (remove + add). Call after schedule is modified.
    pub async fn reload_schedule(&mut self, schedule_id: &str) -> Result<()>;

    /// Start the scheduler.
    pub async fn start(&self) -> Result<()>;
}
```

## Reminder flow

### Cron expression handling

User-friendly schedule inputs need to be converted to 6-field cron expressions:

| User says | Cron expression | Notes |
|-----------|----------------|-------|
| "Monday Wednesday Friday at 7am" | `0 0 7 * * 1,3,5` | sec min hour dom month dow |
| "Every day at 6:30am" | `0 30 6 * * *` | |
| "Twice a week, Tuesday and Thursday at 5pm" | `0 0 17 * * 2,4` | |
| "Every other day at 8am" | Not directly expressible | Use "Mon Wed Fri" or "Tue Thu Sat" approximation |

The LLM can help map natural language to cron, but we should validate the output.

```rust
/// Validate a 6-field cron expression.
pub fn validate_cron(expr: &str) -> Result<()>;

/// Convert a natural language schedule description to a cron expression.
/// Returns the cron expression and a human-readable description.
pub fn parse_schedule_description(desc: &str) -> Result<(String, String)>;
```

### Timezone handling

Schedules are stored with the user's timezone. The cron job fires in UTC, so we need to
convert:

```rust
// When registering a cron job
let user_tz: chrono_tz::Tz = user.timezone.parse()?;
// tokio-cron-scheduler supports timezone-aware scheduling
```

Note: may need `chrono-tz` crate for timezone conversion. Add to workspace deps.

### Reminder message generation

When a reminder fires:

```rust
async fn send_reminder(schedule: &Schedule, user: &User, stage: ReminderStage) {
    // 1. Get today's planned exercises
    let exercises = db.list_schedule_exercises(&schedule.id)?;
    let health_issues = db.list_active_health_entries(&user.id)?;

    // 2. Generate personalised reminder via LLM
    let prompt = format!(
        "Generate a {} workout reminder for {}.
         Today's plan: {}
         Active health issues: {}
         Tone: {}",
        stage.tone_description(),
        user.name,
        format_exercises(&exercises),
        format_health(&health_issues),
        stage.tone(),
    );

    let response = llm.complete(LlmRequest::simple(REMINDER_SYSTEM_PROMPT, &prompt)).await?;

    // 3. Send via Telegram
    telegram.send_message(user.telegram_id, &response.content, Some("Markdown")).await?;

    // 4. Schedule escalation (if not final stage)
    if let Some(next_stage) = stage.next() {
        schedule_escalation(user, schedule, next_stage, stage.escalation_delay()).await;
    }
}
```

### Escalation stages

```rust
pub enum ReminderStage {
    Initial,        // reminder_notice_mins before scheduled time
    FirstNudge,     // +30 min after initial
    FinalCheck,     // +60 min after initial
}

impl ReminderStage {
    pub fn tone(&self) -> &str {
        match self {
            Self::Initial => "friendly and motivating",
            Self::FirstNudge => "slightly more insistent, playfully persistent",
            Self::FinalCheck => "understanding but checking in, offer to skip",
        }
    }

    pub fn escalation_delay(&self) -> Duration {
        match self {
            Self::Initial => Duration::from_secs(30 * 60),     // 30 min to next
            Self::FirstNudge => Duration::from_secs(30 * 60),  // 30 min to final
            Self::FinalCheck => unreachable!(),                 // no next stage
        }
    }

    pub fn next(&self) -> Option<Self> {
        match self {
            Self::Initial => Some(Self::FirstNudge),
            Self::FirstNudge => Some(Self::FinalCheck),
            Self::FinalCheck => None,
        }
    }
}
```

### Escalation cancellation

The escalation chain is cancelled when:
- User starts a session (detected from incoming Telegram message + action)
- User explicitly says "skip" or "rest day" (detected by LLM action)
- User responds to the reminder with a message (any response = acknowledgement)

Implementation: use `tokio::sync::watch` or a shared `AtomicBool` per user to signal
cancellation from the message handler to pending escalation tasks.

## Schedule creation via chat

### Conversational flow

```
User: "I want to work out three times a week"
Bot: "Sure! What days work best for you, and what time?"
User: "Monday Wednesday Friday at 7am"
Bot: "Got it! What would you like to focus on each day?
     Common splits:
     - Push/Pull/Legs
     - Upper/Lower/Full Body
     - Chest+Tri / Back+Bi / Legs+Shoulders"
User: "Push pull legs"
Bot: "Here's your programme:

     Monday 7:00 AM - Push Day
     • Barbell Bench Press: 4x8
     • Overhead Press: 3x10
     • Incline Dumbbell Press: 3x10
     • Lateral Raise: 3x12
     • Tricep Pushdown: 3x12

     Wednesday 7:00 AM - Pull Day
     • Conventional Deadlift: 4x5
     • Barbell Row: 4x8
     • Pull-up: 3xAMRAP
     • Face Pull: 3x15
     • Barbell Curl: 3x10

     Friday 7:00 AM - Legs Day
     • Barbell Back Squat: 4x8
     • Romanian Deadlift: 3x10
     • Leg Press: 3x12
     • Leg Curl: 3x12
     • Calf Raise: 4x15

     I'll remind you 30 minutes before each session. Sound good?"
User: "Perfect"
Bot: [creates 3 schedules with exercises]
```

### LLM-powered programme suggestion (scheduler/programme.rs)

The LLM gets context about:
- User's exercise history (what they've been doing, at what weights)
- User's stated goals (from targets table)
- Active injuries/health issues (avoid affected muscle groups)
- Available exercise catalogue
- Standard training programme templates

System prompt for programme generation:
```
You are designing a workout programme. Consider:
- User's experience level (inferred from exercise history)
- Current strength levels (from recent logs)
- Active injuries: {health_entries}
- User's goals: {targets}
- Recovery time between muscle groups (48-72 hours minimum)

Output a JSON schedule with exercises, sets, reps, and suggested weights
based on their recent performance. Suggest a slight progression (2.5-5%)
from their last recorded weight for each exercise.
```

### New assistant actions

```rust
// Add to AssistantAction enum
CreateSchedule {
    name: String,
    days: Vec<String>,         // "monday", "wednesday", etc.
    time: String,              // "07:00"
    exercises: Vec<ScheduledExercise>,
},
ModifySchedule {
    schedule_id: String,
    // Fields to modify...
},
DeleteSchedule {
    schedule_id: String,
},
SkipWorkout,
```

### System prompt additions

Add to the LLM system prompt:
```
Current schedules:
{formatted_schedules}

The user can ask to:
- Create a new workout schedule ("I want to train 3x/week")
- Modify an existing schedule ("change Monday to 8am", "swap squats for leg press")
- Delete a schedule ("cancel my Wednesday workout")
- Skip today's workout ("skip", "rest day")
- See their schedule ("what's my programme?")

When creating a schedule, use the CreateSchedule action with specific exercises,
sets, reps, and suggested weights based on their history.
```

## Dashboard integration

Add a "Schedule" section to the dashboard (M3):
- View all schedules with next fire time
- Enable/disable schedules with a toggle
- View upcoming week as a calendar strip

Add API endpoints:
- `PUT /api/schedule/{id}` -- update schedule (toggle enable, change time)
- `DELETE /api/schedule/{id}` -- delete schedule

## Tests

### Scheduler tests
- `cron_expression_validation` -- valid and invalid expressions
- `schedule_fires_at_correct_time` -- mock timer
- `escalation_stages` -- verify progression Initial -> FirstNudge -> FinalCheck
- `escalation_cancelled_on_session_start` -- no further reminders after session starts
- `escalation_cancelled_on_skip` -- no further reminders after skip

### Programme tests
- `push_pull_legs_split` -- verify correct muscle group assignment
- `injury_aware_programme` -- exercises avoid injured body parts
- `weight_progression` -- suggested weights are ~2.5-5% above recent max

### Integration tests
- Create schedule via chat, verify DB records created
- Modify schedule via chat, verify cron job updated
- Receive reminder, start session, verify escalation cancelled

## Verification

```sh
# Tests
cargo test -p corre-gym -- scheduler

# Manual
# 1. Text bot: "I want to work out Monday Wednesday Friday at 7am"
# 2. Follow conversation to create programme
# 3. Verify /status shows schedules
# 4. Set a schedule for 2 minutes from now to test reminders
# 5. Verify reminder arrives on Telegram
# 6. Ignore reminder, verify escalation arrives 30 min later
# 7. Reply "skip", verify no more reminders
```
